//! Deterministic property tests assembled from LaTeX shapes that exercise the scanner.

use std::panic::{AssertUnwindSafe, catch_unwind};

use xtex_core::io::Memory;
use xtex_core::scanner::{EntryToken, Piece, scan};
use xtex_core::source::Span;
use xtex_core::{parse, transport};

const ITERATIONS: usize = 2_048;
const DEFAULT_SEED: u64 = 0x24_5eed_cafe_f00d;

// These are real scanner shapes rather than random bytes: every exclusion
// opener, entry-token family, representative hazard-fixture fragments, and
// invalid UTF-8. Some are deliberately incomplete so later fragments occur
// after a boundary the scanner cannot recover.
const FRAGMENTS: &[&[u8]] = &[
    b"ordinary prose @ and @{ tabular bytes\n",
    b"% comment with @ref(hidden)\n",
    b"$ @cite(math) $",
    b"$$ @id(display) $$",
    b"\\[ @ref(display) \\]",
    b"\\verb|@ref(verbatim)|",
    b"\\verb|unterminated @ref(hidden)",
    b"\\lstinline[language=C]|@id(listing)|",
    b"\\mintinline{python}|@cite(minted)|",
    b"\\begin{verbatim}\n@ref(hidden)\n\\end{verbatim}",
    b"\\begin{verbatim}\n@ref(unterminated)",
    b"\\begin{lstlisting}\n@id(hidden)\n\\end{lstlisting}",
    b"\\makeatletter \\@internal @ref(hidden) \\makeatother",
    b"\\makeatletter @ref(unterminated)",
    b"\\catcode`\\@=11\n@ref(hidden)",
    b"\\newcommand{\\mycmd}[2]{sees @ref(inside) and #1 and #2}",
    b"\\unknown{one}{two} @ref(after)",
    b"\\unknown{unterminated @ref(hidden)",
    b"\\csname fig@\\romannumeral 3 @ref(inside)\\endcsname",
    b"\\ifdefined\\pdfoutput @ref(taken)\\else @ref(other)\\fi",
    b"@add(words)",
    b"@del(words)",
    b"@sub(old)(new)",
    b"@note(note)",
    b"@id(entity)",
    b"@ref(entity)",
    b"@cite(key)",
    b"@import(chapter.tex)",
    b"latex { @ref(raw) }",
    b"\\figure(fig) { caption = {A @ref(child)} }",
    b"\\table(tab) { caption = {A table} columns = {l} rows = {{x}} }",
    b"@ref(unclosed",
    b"latex { unclosed @id(hidden)",
    b"\xff\xfe\x80 invalid UTF-8 \xc3(",
    b"{}[]()\\\\%%%\r\n\0",
];

/// `SplitMix64` has a published, fixed transition and needs no external crate.
/// It is adequate here because reproducibility, not cryptographic quality, is
/// the requirement. Each generated document records its initial state.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn below(&mut self, upper: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(upper).expect("fragment table fits in u64"))
            .expect("remainder is smaller than a usize")
    }
}

fn generate(seed: u64) -> Vec<&'static [u8]> {
    let mut random = SplitMix64(seed);
    let count = 8 + random.below(25);
    (0..count)
        .map(|_| FRAGMENTS[random.below(FRAGMENTS.len())])
        .collect()
}

fn join(fragments: &[&[u8]]) -> Vec<u8> {
    fragments.concat()
}

fn extent(piece: &Piece) -> Span {
    match piece {
        Piece::Text(span)
        | Piece::Excluded(span)
        | Piece::Arguments(span)
        | Piece::Quarantined(span)
        | Piece::Construct { span, .. }
        | Piece::Malformed { span, .. } => *span,
    }
}

fn check_document(bytes: &[u8]) -> Result<(bool, bool, bool), String> {
    let pieces = scan(bytes);
    let mut next = 0;
    for piece in &pieces {
        let span = extent(piece);
        if span.start() != next || span.end() < span.start() {
            return Err(format!("piece {piece:?} breaks coverage after byte {next}"));
        }
        next = span.end();
    }
    if next != bytes.len() {
        return Err(format!("pieces cover {next} of {} bytes", bytes.len()));
    }

    let quarantine = pieces.iter().find_map(|piece| match piece {
        Piece::Quarantined(span) => Some(*span),
        _ => None,
    });
    if let Some(span) = quarantine {
        if span.end() != bytes.len() {
            return Err(format!(
                "quarantine at {} ends at {}",
                span.start(),
                span.end()
            ));
        }
        let mut recognised_after = false;
        for piece in &pieces {
            piece.walk(&mut |nested| {
                if matches!(nested, Piece::Construct { span: found, .. } if found.start() >= span.start())
                {
                    recognised_after = true;
                }
            });
        }
        if recognised_after {
            return Err(format!(
                "construct recognised after quarantine at {}",
                span.start()
            ));
        }
    }

    let mut contains_construct = false;
    let mut unterminated =
        quarantine.is_some_and(|span| !bytes[span.start()..].starts_with(b"\\catcode"));
    for piece in &pieces {
        piece.walk(&mut |piece| {
            contains_construct |= matches!(piece, Piece::Construct { .. });
            unterminated |= matches!(
                piece,
                Piece::Malformed {
                    kind: EntryToken::Raw,
                    ..
                }
            );
        });
    }

    let mut sources = xtex_core::source::Sources::new();
    let id = sources.add("generated.xtex", bytes);
    let document = parse(&sources, id);
    if document.quarantine_position() != quarantine.map(Span::start) {
        return Err("scanner and document disagree about quarantine".to_owned());
    }

    if !contains_construct {
        let mut memory = Memory::new().with_input("generated.xtex", bytes.to_vec());
        transport("generated.xtex", &memory.clone(), &mut memory)
            .map_err(|error| format!("transport failed: {error}"))?;
        if memory.output("generated.xtex") != Some(bytes) {
            return Err("construct-free transport changed bytes".to_owned());
        }
    }

    Ok((unterminated, quarantine.is_some(), contains_construct))
}

fn failure(bytes: &[u8]) -> Option<String> {
    match catch_unwind(AssertUnwindSafe(|| check_document(bytes))) {
        Ok(Ok(_)) => None,
        Ok(Err(message)) => Some(message),
        Err(_) => Some("scanner, parser, or transport panicked".to_owned()),
    }
}

fn shrink(mut fragments: Vec<&'static [u8]>) -> Vec<&'static [u8]> {
    let mut chunk = fragments.len() / 2;
    while chunk > 0 {
        let mut start = 0;
        let mut reduced = false;
        while start + chunk <= fragments.len() {
            let mut candidate = fragments.clone();
            candidate.drain(start..start + chunk);
            if failure(&join(&candidate)).is_some() {
                fragments = candidate;
                reduced = true;
            } else {
                start += 1;
            }
        }
        if !reduced {
            chunk /= 2;
        }
    }
    fragments
}

#[test]
fn generated_documents_preserve_scanner_invariants() {
    // 2,048 cases keep this suitable for `cargo test` while producing up to
    // 65,536 targeted fragment placements. Set XTEX_PROPERTY_SEED to replay
    // exactly one reported case.
    let replay = std::env::var("XTEX_PROPERTY_SEED")
        .ok()
        .map(|value| value.parse::<u64>().expect("XTEX_PROPERTY_SEED is decimal"));
    let iterations = if replay.is_some() { 1 } else { ITERATIONS };
    let mut seeds = SplitMix64(DEFAULT_SEED);
    let mut unterminated = 0;
    let mut quarantined = 0;
    let mut with_construct = 0;

    for _ in 0..iterations {
        let seed = replay.unwrap_or_else(|| seeds.next());
        let fragments = generate(seed);
        let bytes = join(&fragments);
        let checked = catch_unwind(AssertUnwindSafe(|| check_document(&bytes)));
        match checked {
            Ok(Ok((has_unterminated, did_quarantine, contains_construct))) => {
                unterminated += usize::from(has_unterminated);
                quarantined += usize::from(did_quarantine);
                with_construct += usize::from(contains_construct);
            }
            Ok(Err(message)) => {
                let shrunk = shrink(fragments);
                panic!(
                    "seed {seed}: {message}\nshrunk input ({} bytes): {:?}",
                    join(&shrunk).len(),
                    join(&shrunk)
                );
            }
            Err(_) => {
                let shrunk = shrink(fragments);
                panic!(
                    "seed {seed}: scanner, parser, or transport panicked\n\
                     shrunk input ({} bytes): {:?}",
                    join(&shrunk).len(),
                    join(&shrunk)
                );
            }
        }
    }

    println!(
        "generated {iterations} inputs: {unterminated} contained an unterminated region, \
         {quarantined} triggered quarantine, {with_construct} contained a recognised construct"
    );
    let activity_floor = iterations / 4;
    assert!(
        unterminated >= activity_floor,
        "only {unterminated} inputs contained an unterminated region"
    );
    assert!(
        quarantined >= activity_floor,
        "only {quarantined} inputs triggered quarantine"
    );
    assert!(
        with_construct >= activity_floor,
        "only {with_construct} inputs contained a recognised construct"
    );
}
