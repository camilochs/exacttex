//! A verification record is a dated statement, whole or not at all.

use xtex_core::verification::{
    BibVerdict, ClaimKind, DiffSeverity, FieldDiff, Reachability, Verdict, VerificationRecord,
    VerifiedClaim, parse_record, write_record,
};

fn sample() -> VerificationRecord {
    VerificationRecord {
        claims: vec![
            VerifiedClaim {
                kind: ClaimKind::BibEntry,
                target: "knuth1984".to_owned(),
                fields: vec![
                    ("title".to_owned(), "The TeXbook".to_owned()),
                    ("authors".to_owned(), "Donald E. Knuth".to_owned()),
                ],
                response_hash: "ab12cd34".to_owned(),
                source: "crossref-doi".to_owned(),
                fetched_at: "2026-09-01T10:00:00Z".to_owned(),
                verdict: Verdict::Bibliographic(BibVerdict::Partial),
                diffs: vec![FieldDiff {
                    field: "authors".to_owned(),
                    in_document: "D. Knuth".to_owned(),
                    in_source: "Donald E. Knuth".to_owned(),
                    severity: DiffSeverity::High,
                }],
                failure_note: None,
            },
            VerifiedClaim {
                kind: ClaimKind::Url,
                target: "https://example.org/dataset".to_owned(),
                fields: Vec::new(),
                response_hash: String::new(),
                source: "http".to_owned(),
                fetched_at: "2026-09-01T10:00:05Z".to_owned(),
                verdict: Verdict::Reachability(Reachability::Unreachable),
                diffs: Vec::new(),
                failure_note: Some("timed out after 10s".to_owned()),
            },
        ],
    }
}

#[test]
fn a_record_survives_the_round_trip_field_for_field() {
    let record = sample();
    let written = write_record(&record);
    let parsed = parse_record(written.as_bytes()).expect("the canonical shape parses");
    assert_eq!(parsed, record, "every field travels");
}

#[test]
fn the_written_shape_carries_the_verdict_the_date_and_both_diff_sides() {
    let written = write_record(&sample());
    for needle in [
        "\"verdict\":\"partial\"",
        "\"fetched_at\":\"2026-09-01T10:00:00Z\"",
        "\"in_document\":\"D. Knuth\"",
        "\"in_source\":\"Donald E. Knuth\"",
        "\"severity\":\"high\"",
        "\"failure_note\":\"timed out after 10s\"",
    ] {
        assert!(written.contains(needle), "missing {needle} in {written}");
    }
}

#[test]
fn a_record_that_is_not_json_is_no_record() {
    assert!(parse_record(b"not json at all").is_err());
}

#[test]
fn an_unknown_verdict_refuses_the_whole_record() {
    let written = write_record(&sample()).replace("\"partial\"", "\"probably\"");
    let error = parse_record(written.as_bytes()).expect_err("refused");
    assert!(error.message.contains("probably"), "{}", error.message);
}

#[test]
fn a_verdict_in_the_wrong_vocabulary_is_refused() {
    // a bib entry answered with a reachability verdict
    let written = write_record(&sample()).replace("\"partial\"", "\"reachable\"");
    assert!(parse_record(written.as_bytes()).is_err());
}

#[test]
fn an_unverified_claim_without_its_note_is_refused() {
    let mut record = sample();
    record.claims[1].failure_note = None;
    let written = write_record(&record);
    let error = parse_record(written.as_bytes()).expect_err("refused");
    assert!(error.message.contains("failure note"), "{}", error.message);
}

#[test]
fn an_answering_verdict_without_a_response_hash_is_refused() {
    let mut record = sample();
    record.claims[0].response_hash = String::new();
    let written = write_record(&record);
    assert!(parse_record(written.as_bytes()).is_err());
}

#[test]
fn a_missing_date_is_refused_because_a_verdict_is_a_dated_statement() {
    let written = write_record(&sample()).replace("2026-09-01T10:00:00Z", "");
    assert!(parse_record(written.as_bytes()).is_err());
}

#[test]
fn a_wrong_version_is_refused_whole() {
    let written = write_record(&sample()).replace("\"version\":1", "\"version\":2");
    assert!(parse_record(written.as_bytes()).is_err());
}
