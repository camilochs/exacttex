//! The surface carries claims and verdicts — called directly, natively:
//! the same functions the browser calls, minus the browser.

use xtex_core::verification::{
    BibVerdict, ClaimKind, DiffSeverity, FieldDiff, Verdict, VerificationRecord, VerifiedClaim,
    write_record,
};
use xtex_wasm::{xtex_check_with_record, xtex_claims};

fn push_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&u32::try_from(text.len()).expect("fits").to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn bundle(root: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    push_text(&mut out, root);
    out.extend_from_slice(&u32::try_from(files.len()).expect("fits").to_le_bytes());
    for (name, content) in files {
        push_text(&mut out, name);
        out.extend_from_slice(&u32::try_from(content.len()).expect("fits").to_le_bytes());
        out.extend_from_slice(content.as_bytes());
    }
    out
}

fn call(function: unsafe extern "C" fn(*const u8, usize) -> *mut u8, input: &[u8]) -> String {
    let pointer = unsafe { function(input.as_ptr(), input.len()) };
    assert!(!pointer.is_null());
    let len = u32::from_le_bytes(
        unsafe { std::slice::from_raw_parts(pointer, 4) }
            .try_into()
            .expect("length"),
    ) as usize;
    let bytes = unsafe { std::slice::from_raw_parts(pointer.add(4), len) }.to_vec();
    String::from_utf8_lossy(&bytes).into_owned()
}

const MAIN: &str = "\\documentclass{article}\n\\begin{document}\nSee @cite(knuth1984) and \\url{https://example.org/data}.\n\\bibliography{refs}\n\\end{document}\n";
const BIB: &str = "@book{knuth1984,\n  title = {The TeXbook},\n  author = {Donald E. Knuth}\n}\n";

#[test]
fn the_claims_export_inventories_the_bundle() {
    let input = bundle("main.tex", &[("main.tex", MAIN), ("refs.bib", BIB)]);
    let answer = call(xtex_claims, &input);
    assert!(answer.contains("\"kind\":\"bib-entry\""), "{answer}");
    assert!(answer.contains("\"target\":\"knuth1984\""), "{answer}");
    assert!(answer.contains("\"kind\":\"url\""), "{answer}");
    assert!(answer.contains("https://example.org/data"), "{answer}");
    assert!(
        answer.contains("\"title\":\"The TeXbook\""),
        "the fields travel: {answer}"
    );
}

#[test]
fn the_record_aware_check_appends_dated_findings() {
    let record = VerificationRecord {
        claims: vec![VerifiedClaim {
            kind: ClaimKind::BibEntry,
            target: "knuth1984".to_owned(),
            fields: vec![
                ("title".to_owned(), "The TeXbook".to_owned()),
                ("author".to_owned(), "Donald E. Knuth".to_owned()),
            ],
            response_hash: "cafe".to_owned(),
            source: "crossref-doi".to_owned(),
            fetched_at: "2026-08-30T00:00:00Z".to_owned(),
            verdict: Verdict::Bibliographic(BibVerdict::Partial),
            diffs: vec![FieldDiff {
                field: "authors".to_owned(),
                in_document: "Donald E. Knuth".to_owned(),
                in_source: "Donald E. Knuth and X. Fabricated".to_owned(),
                severity: DiffSeverity::High,
            }],
            failure_note: None,
        }],
    };
    let written = write_record(&record);
    let mut input = Vec::new();
    push_text(&mut input, "2026-09-01T00:00:00Z");
    input.extend_from_slice(&30u32.to_le_bytes());
    input.extend_from_slice(&u32::try_from(written.len()).expect("fits").to_le_bytes());
    input.extend_from_slice(written.as_bytes());
    input.extend_from_slice(&bundle(
        "main.tex",
        &[("main.tex", MAIN), ("refs.bib", BIB)],
    ));
    let answer = call(xtex_check_with_record, &input);
    assert!(answer.contains("XT1018"), "the partial replays: {answer}");
    assert!(
        answer.contains("X. Fabricated"),
        "both sides speak: {answer}"
    );
    assert!(answer.contains("2026-08-30"), "dated: {answer}");
    assert!(
        answer.contains("\"error\""),
        "authors on a demanded key is hard: {answer}"
    );
}

#[test]
fn an_empty_record_length_is_the_plain_check() {
    let mut input = Vec::new();
    push_text(&mut input, "2026-09-01T00:00:00Z");
    input.extend_from_slice(&30u32.to_le_bytes());
    input.extend_from_slice(&0u32.to_le_bytes());
    input.extend_from_slice(&bundle(
        "main.tex",
        &[("main.tex", MAIN), ("refs.bib", BIB)],
    ));
    let answer = call(xtex_check_with_record, &input);
    assert!(
        !answer.contains("XT1015"),
        "no record is not an unreadable record: {answer}"
    );
    assert!(!answer.contains("XT1018"), "{answer}");
    assert!(
        answer.contains("\"coverage\""),
        "the plain check shape: {answer}"
    );
}

#[test]
fn an_unreadable_record_is_an_advisory_not_a_silent_skip() {
    let mut input = Vec::new();
    push_text(&mut input, "2026-09-01T00:00:00Z");
    input.extend_from_slice(&30u32.to_le_bytes());
    let garbage = b"not a record";
    input.extend_from_slice(&u32::try_from(garbage.len()).expect("fits").to_le_bytes());
    input.extend_from_slice(garbage);
    input.extend_from_slice(&bundle(
        "main.tex",
        &[("main.tex", MAIN), ("refs.bib", BIB)],
    ));
    let answer = call(xtex_check_with_record, &input);
    assert!(answer.contains("XT1015"), "{answer}");
    assert!(answer.contains("unreadable"), "{answer}");
}
