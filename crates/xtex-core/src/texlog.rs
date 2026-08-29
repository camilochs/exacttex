//! Reading what a TeX engine said.
//!
//! The engine reports against the emitted `.tex`, which the author has never
//! seen. This turns its output into records carrying a file and a line, so
//! [`crate::diagnostics`] can map them back to the source the author wrote.
//!
//! # What is parsed, and what is not
//!
//! Only forms observed from a live engine run and written down here. A line
//! that does not match one of them is not guessed at: it is returned as
//! [`Record::Unrecognised`], and the caller prints it unchanged.
//!
//! That is deliberate. A log parser that half-understands a message invents a
//! location for it, and a diagnostic pointing at the wrong line is worse than
//! one pointing nowhere.

/// Severity as the engine reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The engine could not continue, or continued under protest.
    Error,
    /// The engine finished but said something about the result.
    Warning,
}

/// A typesetting failure the compiler can restate.
///
/// Five shapes, every one transcribed from a live `tectonic` run rather than
/// recalled. The engine's own sentence is kept beside them; these only say
/// which kind it was and how much, so a diagnostic can name the author's
/// entity without discarding the evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visual {
    /// `Overfull \hbox (113.94pt too wide) …` — content wider than its line.
    OverfullHbox,
    /// `Overfull \vbox (38.48pt too high) …` — content taller than its box.
    OverfullVbox,
    /// `Missing character: There is no 日 ("65E5) in font ec-lmr10!`
    MissingGlyph,
    /// `LaTeX Warning: Float too large for page by 327.53pt on input line 9.`
    FloatTooLarge,
}

impl Visual {
    /// How the compiler names this failure to an author.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OverfullHbox => "overflows its line",
            Self::OverfullVbox => "overflows its box",
            Self::MissingGlyph => "uses a character the font does not have",
            Self::FloatTooLarge => "is too large for the page",
        }
    }
}

/// One line of engine output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A message the engine attached to a file and line.
    Located {
        /// How bad the engine said it was.
        severity: Severity,
        /// File the engine named, which is an emitted file.
        file: String,
        /// One-based line in that file.
        line: u32,
        /// The engine's own words, unchanged.
        message: String,
    },
    /// A typesetting failure, recognised and located.
    ///
    /// Separate from [`Record::Located`] because only these can be restated
    /// in terms of an entity, and only when a map segment and a declared
    /// entity both supply the evidence.
    Typeset {
        /// Which failure.
        visual: Visual,
        /// File the engine named, which is an emitted file.
        file: String,
        /// One-based line in that file.
        line: u32,
        /// The engine's own words, unchanged.
        message: String,
    },
    /// A line that matched no known form.
    ///
    /// Kept rather than dropped: an engine's output is evidence, and silently
    /// discarding part of it makes the compiler look like it saw less than it
    /// did.
    Unrecognised(String),
}

/// Recognises a typesetting failure in an engine's own sentence.
///
/// Returns `None` for anything not in the transcribed set, which is what keeps
/// an unfamiliar message from being restated as something it is not.
#[must_use]
pub fn classify(message: &str) -> Option<Visual> {
    if message.starts_with("Overfull \\hbox") {
        return Some(Visual::OverfullHbox);
    }
    if message.starts_with("Overfull \\vbox") {
        return Some(Visual::OverfullVbox);
    }
    if message.starts_with("Missing character:") {
        return Some(Visual::MissingGlyph);
    }
    if message.starts_with("LaTeX Warning: Float too large for page") {
        return Some(Visual::FloatTooLarge);
    }
    None
}

/// The line a `.log` record names, when it names one in its own text.
///
/// `LaTeX Warning: … on input line 9.` carries its line inside the sentence
/// rather than in a `file:line:` prefix, which is why the raw log needs its own
/// reader: that record never reaches stderr at all.
#[must_use]
pub fn line_in_message(message: &str) -> Option<u32> {
    let at = message.find("on input line ")? + "on input line ".len();
    let digits: String = message[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Reads a raw `.log`, which carries records stderr never shows.
///
/// Checked against a live run: `Float too large for page` appears only here.
/// A reader that took stderr alone would silently miss a whole class.
#[must_use]
pub fn parse_log(log: &str, file: &str) -> Vec<Record> {
    let mut records = Vec::new();
    for line in log.lines() {
        let line = line.trim_end();
        let Some(visual) = classify(line) else {
            continue;
        };
        let Some(number) = line_in_message(line).or_else(|| line_after(line)) else {
            continue;
        };
        let record = Record::Typeset {
            visual,
            file: file.to_owned(),
            line: number,
            message: line.to_owned(),
        };
        if !records.contains(&record) {
            records.push(record);
        }
    }
    records
}

/// The line an `Overfull` record names in its trailing `at line N` or
/// `at lines N--M`.
fn line_after(message: &str) -> Option<u32> {
    let at = message.rfind("at line")? + "at line".len();
    let rest = message[at..].trim_start_matches('s').trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Parses one line of engine output.
///
/// The forms are `tectonic`'s, transcribed from a run rather than recalled:
///
/// ```text
/// error: paper.tex:3: Undefined control sequence
/// warning: paper.tex:5: Overfull \hbox (266.11pt too wide) detected at line 5
/// ```
#[must_use]
pub fn parse_line(line: &str) -> Option<Record> {
    let line = line.trim_end();
    if line.is_empty() {
        return None;
    }

    let (severity, rest) = if let Some(rest) = line.strip_prefix("error: ") {
        (Severity::Error, rest)
    } else if let Some(rest) = line.strip_prefix("warning: ") {
        (Severity::Warning, rest)
    } else {
        return Some(Record::Unrecognised(line.to_owned()));
    };

    // `FILE:LINE: MESSAGE`, where FILE may itself contain a colon on a system
    // that allows one. The line number is the last colon-separated field that
    // parses as a number before the message, so the split is from the right of
    // the file rather than the left of the string.
    let Some((location, message)) = rest.split_once(": ") else {
        return Some(Record::Unrecognised(line.to_owned()));
    };
    let Some((file, number)) = location.rsplit_once(':') else {
        // A message with no location: "halted on potentially-recoverable
        // error". Real output, and it belongs to the run rather than to a line.
        return Some(Record::Unrecognised(line.to_owned()));
    };
    let Ok(number) = number.parse::<u32>() else {
        return Some(Record::Unrecognised(line.to_owned()));
    };

    Some(Record::Located {
        severity,
        file: file.to_owned(),
        line: number,
        message: message.to_owned(),
    })
}

/// Parses a whole stream of engine output.
///
/// Duplicates are removed. An engine that runs twice to settle references says
/// the same thing twice, and an author does not need to read it twice.
#[must_use]
pub fn parse(output: &str) -> Vec<Record> {
    let mut records = Vec::new();
    for line in output.lines() {
        let Some(record) = parse_line(line) else {
            continue;
        };
        if !records.contains(&record) {
            records.push(record);
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five shapes, transcribed from live tectonic runs on 2026-08-29.
    const OBSERVED: &[(&str, Option<Visual>)] = &[
        (
            "Overfull \\hbox (113.94pt too wide) in paragraph at lines 5--5",
            Some(Visual::OverfullHbox),
        ),
        (
            "Overfull \\hbox (215.64pt too wide) detected at line 14",
            Some(Visual::OverfullHbox),
        ),
        (
            "Overfull \\vbox (38.48pt too high) detected at line 16",
            Some(Visual::OverfullVbox),
        ),
        (
            "Missing character: There is no X (\"65E5) in font ec-lmr10!",
            Some(Visual::MissingGlyph),
        ),
        (
            "LaTeX Warning: Float too large for page by 327.5271pt on input line 9.",
            Some(Visual::FloatTooLarge),
        ),
        // Real lines from the same runs that must not be classified.
        (
            "Underfull \\hbox (badness 10000) in paragraph at lines 3--4",
            None,
        ),
        ("LaTeX Warning: There were undefined references.", None),
        (
            "Package hyperref Warning: Token not allowed in a PDF string.",
            None,
        ),
    ];

    #[test]
    fn only_the_transcribed_shapes_are_classified() {
        for (line, expected) in OBSERVED {
            assert_eq!(classify(line), *expected, "{line}");
        }
    }

    #[test]
    fn a_shape_that_was_never_transcribed_is_not_guessed_at() {
        // The failure this guards: restating an unfamiliar message as
        // something it is not. An underfull box is a real warning and a real
        // typesetting problem, and it is not in the supported set, so it stays
        // TeX's own sentence rather than becoming a claim about an entity.
        assert_eq!(
            classify("Underfull \\vbox (badness 10000) has occurred"),
            None
        );
        assert_eq!(classify("Overfull nothing at all"), None);
    }

    #[test]
    fn a_record_carrying_its_line_inside_the_sentence_is_read() {
        // `Float too large` never reaches stderr, and its line is in the prose
        // rather than in a `file:line:` prefix. A reader taking stderr alone
        // would miss the whole class.
        let log = "LaTeX Warning: Float too large for page by 327.5271pt on input line 9.\n";
        let records = parse_log(log, "paper.tex");
        assert_eq!(records.len(), 1);
        let Record::Typeset { visual, line, .. } = &records[0] else {
            panic!("expected a typeset record")
        };
        assert_eq!(*visual, Visual::FloatTooLarge);
        assert_eq!(*line, 9);
    }

    #[test]
    fn an_overfull_range_is_read_from_its_first_line() {
        let log = "Overfull \\hbox (113.94pt too wide) in paragraph at lines 5--5\n";
        let Record::Typeset { line, .. } = &parse_log(log, "p.tex")[0] else {
            panic!("expected a typeset record")
        };
        assert_eq!(*line, 5);
    }

    #[test]
    fn a_log_line_that_is_not_a_supported_failure_yields_no_record() {
        let log = "This is the LaTeX engine, version whatever\nUnderfull \\hbox somewhere\n";
        assert!(parse_log(log, "p.tex").is_empty());
    }

    #[test]
    fn the_engines_own_words_survive_classification() {
        let log = "Overfull \\hbox (215.64pt too wide) detected at line 14\n";
        let Record::Typeset { message, .. } = &parse_log(log, "p.tex")[0] else {
            panic!("expected a typeset record")
        };
        assert_eq!(
            message,
            "Overfull \\hbox (215.64pt too wide) detected at line 14"
        );
    }

    #[test]
    fn an_error_carries_its_file_and_line() {
        let Some(Record::Located {
            severity,
            file,
            line,
            message,
        }) = parse_line("error: paper.tex:3: Undefined control sequence")
        else {
            panic!("expected a located record")
        };
        assert_eq!(severity, Severity::Error);
        assert_eq!(file, "paper.tex");
        assert_eq!(line, 3);
        assert_eq!(message, "Undefined control sequence");
    }

    #[test]
    fn a_warning_keeps_the_engines_own_words() {
        // Including the engine's own "at line 5", which is about the emitted
        // file and must not be rewritten to the source line. Rewriting inside
        // the message would be editing evidence.
        let Some(Record::Located { message, .. }) = parse_line(
            "warning: paper.tex:5: Overfull \\hbox (266.10999pt too wide) detected at line 5",
        ) else {
            panic!("expected a located record")
        };
        assert!(message.ends_with("detected at line 5"), "{message}");
    }

    #[test]
    fn a_message_with_no_location_is_kept_rather_than_dropped() {
        let record = parse_line("error: halted on potentially-recoverable error as specified");
        assert!(
            matches!(record, Some(Record::Unrecognised(_))),
            "{record:?}"
        );
    }

    #[test]
    fn an_unknown_line_is_never_given_an_invented_location() {
        // The failure this guards: a parser that half-understands a message
        // and attaches a line number to it points the author at the wrong
        // place, which is worse than pointing nowhere.
        let record = parse_line("note: \"version 2\" Tectonic command-line interface activated");
        assert!(
            matches!(record, Some(Record::Unrecognised(_))),
            "{record:?}"
        );
    }

    #[test]
    fn a_repeated_message_is_reported_once() {
        // An engine runs twice to settle references and says everything twice.
        let output = "warning: p.tex:5: Overfull \\hbox\nwarning: p.tex:5: Overfull \\hbox\n";
        assert_eq!(parse(output).len(), 1);
    }

    #[test]
    fn a_path_containing_a_colon_still_splits_at_the_line_number() {
        let Some(Record::Located { file, line, .. }) =
            parse_line("error: odd:name.tex:12: Missing $ inserted")
        else {
            panic!("expected a located record")
        };
        assert_eq!(file, "odd:name.tex");
        assert_eq!(line, 12);
    }
}
