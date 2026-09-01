//! The record-aware check: offline, dated, and graded by the standing
//! severity policy — hard only where an explicit construct demands it.

use xtex_core::check::Severity;
use xtex_core::claims::Claim;
use xtex_core::source::{Sources, Span};
use xtex_core::symbols::SymbolTable;
use xtex_core::verification::{
    BibVerdict, ClaimKind, DiffSeverity, FieldDiff, Reachability, RecordCheck, Verdict,
    VerificationRecord, VerifiedClaim, check_against_record,
};

const NOW: &str = "2026-09-01T12:00:00Z";
const FRESH: &str = "2026-08-30T12:00:00Z";
const OLD: &str = "2026-05-01T12:00:00Z";

/// A document whose `@cite` demands `knuth1984` and nothing else.
fn table_and_sources() -> (Sources, SymbolTable, xtex_core::source::SourceId) {
    let mut sources = Sources::new();
    let id = sources.add(
        "main.xtex",
        b"\\documentclass{article}\n\\begin{document}\nSee @cite(knuth1984).\n\\end{document}\n"
            .to_vec(),
    );
    let document = xtex_core::parse(&sources, id);
    let mut table = SymbolTable::new();
    table.merge(&sources, &document);
    (sources, table, id)
}

fn bib_claim(id: xtex_core::source::SourceId, key: &str, title: &str) -> Claim {
    Claim {
        kind: ClaimKind::BibEntry,
        target: key.to_owned(),
        source: id,
        span: Span::new(0, 4),
        fields: vec![("title".to_owned(), title.to_owned())],
    }
}

fn recorded(key: &str, title: &str, verdict: Verdict, fetched_at: &str) -> VerifiedClaim {
    VerifiedClaim {
        kind: ClaimKind::BibEntry,
        target: key.to_owned(),
        fields: vec![("title".to_owned(), title.to_owned())],
        response_hash: "cafe".to_owned(),
        source: "crossref-doi".to_owned(),
        fetched_at: fetched_at.to_owned(),
        verdict,
        diffs: Vec::new(),
        failure_note: None,
    }
}

fn run(
    claims: &[Claim],
    record: &VerificationRecord,
    table: &SymbolTable,
) -> Vec<xtex_core::check::Diagnostic> {
    check_against_record(&RecordCheck {
        record,
        claims,
        table,
        now: NOW,
        max_age_days: 30,
    })
}

#[test]
fn a_fresh_verified_claim_is_silent() {
    let (_, table, id) = table_and_sources();
    let claims = [bib_claim(id, "knuth1984", "The TeXbook")];
    let record = VerificationRecord {
        claims: vec![recorded(
            "knuth1984",
            "The TeXbook",
            Verdict::Bibliographic(BibVerdict::Verified),
            FRESH,
        )],
    };
    assert!(run(&claims, &record, &table).is_empty());
}

#[test]
fn an_edited_claim_retires_its_verdict_and_reports_the_drift() {
    let (_, table, id) = table_and_sources();
    let claims = [bib_claim(id, "knuth1984", "The TeXbook, 2nd impression")];
    let mut old = recorded(
        "knuth1984",
        "The TeXbook",
        Verdict::Bibliographic(BibVerdict::Mismatch),
        FRESH,
    );
    old.diffs.push(FieldDiff {
        field: "authors".to_owned(),
        in_document: "X".to_owned(),
        in_source: "Y".to_owned(),
        severity: DiffSeverity::High,
    });
    let record = VerificationRecord { claims: vec![old] };
    let findings = run(&claims, &record, &table);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].code, "XT1016");
    assert_eq!(findings[0].severity, Severity::Advisory);
    assert!(
        findings[0]
            .message
            .contains("edited after its verification")
    );
}

#[test]
fn an_expired_verification_says_its_age_in_days() {
    let (_, table, id) = table_and_sources();
    let claims = [bib_claim(id, "knuth1984", "The TeXbook")];
    let record = VerificationRecord {
        claims: vec![recorded(
            "knuth1984",
            "The TeXbook",
            Verdict::Bibliographic(BibVerdict::Verified),
            OLD,
        )],
    };
    let findings = run(&claims, &record, &table);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].code, "XT1017");
    assert!(
        findings[0].message.contains("123 days ago"),
        "{}",
        findings[0].message
    );
}

#[test]
fn a_high_severity_diff_on_a_demanded_key_is_a_hard_error() {
    let (_, table, id) = table_and_sources();
    let claims = [bib_claim(id, "knuth1984", "The TeXbook")];
    let mut partial = recorded(
        "knuth1984",
        "The TeXbook",
        Verdict::Bibliographic(BibVerdict::Partial),
        FRESH,
    );
    partial.diffs.push(FieldDiff {
        field: "authors".to_owned(),
        in_document: "D. Knuth and A. Nobody".to_owned(),
        in_source: "Donald E. Knuth".to_owned(),
        severity: DiffSeverity::High,
    });
    let record = VerificationRecord {
        claims: vec![partial],
    };
    let findings = run(&claims, &record, &table);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "XT1018");
    assert_eq!(
        findings[0].severity,
        Severity::Error,
        "authors on a demanded key"
    );
    assert!(
        findings[0].message.contains("authors differs"),
        "{}",
        findings[0].message
    );
    assert!(
        findings[0].message.contains("A. Nobody"),
        "both sides speak"
    );
}

#[test]
fn the_same_finding_on_an_undemanded_key_is_advisory() {
    let (_, table, id) = table_and_sources();
    // `lamport1994` is in no @cite: the gradual policy keeps it advisory.
    let claims = [bib_claim(id, "lamport1994", "LaTeX")];
    let mut partial = recorded(
        "lamport1994",
        "LaTeX",
        Verdict::Bibliographic(BibVerdict::Partial),
        FRESH,
    );
    partial.diffs.push(FieldDiff {
        field: "authors".to_owned(),
        in_document: "L".to_owned(),
        in_source: "Leslie Lamport".to_owned(),
        severity: DiffSeverity::High,
    });
    let record = VerificationRecord {
        claims: vec![partial],
    };
    let findings = run(&claims, &record, &table);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, Severity::Advisory);
}

#[test]
fn an_unverified_claim_replays_its_note_dated() {
    let (_, table, id) = table_and_sources();
    let claims = [bib_claim(id, "knuth1984", "The TeXbook")];
    let mut unverified = recorded(
        "knuth1984",
        "The TeXbook",
        Verdict::Bibliographic(BibVerdict::Unverified),
        FRESH,
    );
    unverified.response_hash = String::new();
    unverified.failure_note = Some("crossref timed out".to_owned());
    let record = VerificationRecord {
        claims: vec![unverified],
    };
    let findings = run(&claims, &record, &table);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "XT1019");
    assert_eq!(findings[0].severity, Severity::Advisory);
    assert!(findings[0].message.contains("crossref timed out"));
    assert!(findings[0].message.contains("2026-08-30"), "dated");
}

#[test]
fn an_unreachable_url_is_never_a_hard_error() {
    let (_, table, id) = table_and_sources();
    let claims = [Claim {
        kind: ClaimKind::Url,
        target: "https://example.org/data".to_owned(),
        source: id,
        span: Span::new(0, 4),
        fields: Vec::new(),
    }];
    let record = VerificationRecord {
        claims: vec![VerifiedClaim {
            kind: ClaimKind::Url,
            target: "https://example.org/data".to_owned(),
            fields: Vec::new(),
            response_hash: String::new(),
            source: "http".to_owned(),
            fetched_at: FRESH.to_owned(),
            verdict: Verdict::Reachability(Reachability::Unreachable),
            diffs: Vec::new(),
            failure_note: Some("404".to_owned()),
        }],
    };
    let findings = run(&claims, &record, &table);
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].severity,
        Severity::Advisory,
        "a plain \\url is not a construct"
    );
    assert!(findings[0].message.contains("unreachable as of 2026-08-30"));
}
