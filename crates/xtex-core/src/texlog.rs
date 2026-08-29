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
    /// A line that matched no known form.
    ///
    /// Kept rather than dropped: an engine's output is evidence, and silently
    /// discarding part of it makes the compiler look like it saw less than it
    /// did.
    Unrecognised(String),
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
