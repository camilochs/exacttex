//! The verification record: what the world answered, dated.
//!
//! External verification never happens here — the core owns no network and
//! never will. What lives here is the RECORD a verifier leaves behind: for
//! each claim the document makes about the world (a bibliography entry, a
//! url, a doi, a repository), what was asked, what source answered, when,
//! and with which verdict. The record is a project sidecar; the check reads
//! it offline, so compilation stays deterministic whatever the network did.
//!
//! # The rule that keeps this honest
//!
//! A record that does not parse is *no record* — the same all-or-nothing
//! stance the bibliography reader takes. A partial record would let a stale
//! or mangled verdict pass as a fresh one, which is worse than none: it
//! reports success while measuring something else.
//!
//! Every verdict is a dated statement, never a timeless fact: `fetched_at`
//! is part of the claim's meaning, and an `Unverified` verdict must carry
//! the note saying why — an unreachable source is never "probably fine".

use crate::json::{self, Value};

/// What kind of thing the document asserted exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClaimKind {
    /// An entry in a declared `.bib`, keyed by its citation key.
    BibEntry,
    /// A `\url{…}` or `\href{…}` target.
    Url,
    /// A DOI, wherever it was written.
    Doi,
    /// A source repository address.
    Repository,
}

impl ClaimKind {
    fn name(self) -> &'static str {
        match self {
            Self::BibEntry => "bib-entry",
            Self::Url => "url",
            Self::Doi => "doi",
            Self::Repository => "repository",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "bib-entry" => Some(Self::BibEntry),
            "url" => Some(Self::Url),
            "doi" => Some(Self::Doi),
            "repository" => Some(Self::Repository),
            _ => None,
        }
    }
}

/// The verdict on a bibliographic claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BibVerdict {
    /// Every compared field matches the source.
    Verified,
    /// Some fields match and some differ — the diffs say which.
    Partial,
    /// The source answers with a different work altogether.
    Mismatch,
    /// The source could not be consulted; the note says why.
    Unverified,
}

impl BibVerdict {
    fn name(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Partial => "partial",
            Self::Mismatch => "mismatch",
            Self::Unverified => "unverified",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "verified" => Some(Self::Verified),
            "partial" => Some(Self::Partial),
            "mismatch" => Some(Self::Mismatch),
            "unverified" => Some(Self::Unverified),
            _ => None,
        }
    }
}

/// The verdict on a reachability claim (urls, dois, repositories).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reachability {
    /// The address answered.
    Reachable,
    /// The address did not answer; the note says how it failed.
    Unreachable,
    /// The address answered from somewhere else — worth a look.
    Redirected,
}

impl Reachability {
    fn name(self) -> &'static str {
        match self {
            Self::Reachable => "reachable",
            Self::Unreachable => "unreachable",
            Self::Redirected => "redirected",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "reachable" => Some(Self::Reachable),
            "unreachable" => Some(Self::Unreachable),
            "redirected" => Some(Self::Redirected),
            _ => None,
        }
    }
}

/// A claim's verdict, in the vocabulary its kind speaks.
///
/// Bibliographic entries and reachable addresses are different questions
/// with different honest answers; one enum forced over both would deform
/// whichever it was not designed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// For [`ClaimKind::BibEntry`].
    Bibliographic(BibVerdict),
    /// For urls, dois and repositories.
    Reachability(Reachability),
}

/// How much a differing field matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSeverity {
    /// The author list, always: a fabricated author list behind a valid
    /// DOI is exactly the failure verification exists to catch.
    High,
    /// Everything else.
    Medium,
}

/// One field where the document and the source disagree, both values kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDiff {
    /// Which field (`authors`, `title`, `year`, …).
    pub field: String,
    /// What the document says.
    pub in_document: String,
    /// What the source says.
    pub in_source: String,
    /// How much the disagreement matters.
    pub severity: DiffSeverity,
}

/// One verified claim: what was asked, what answered, when, and the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClaim {
    /// What kind of claim this is.
    pub kind: ClaimKind,
    /// The claim's target: a citation key, or an address.
    pub target: String,
    /// The document's fields as they were compared, normalized.
    pub fields: Vec<(String, String)>,
    /// A hash of the source's raw response — the fingerprint later runs
    /// compare against. Empty only when nothing answered.
    pub response_hash: String,
    /// Which source answered (`crossref-doi`, `crossref-query`, `dblp`,
    /// `arxiv`, `http`).
    pub source: String,
    /// When, as RFC 3339 in UTC. Part of the verdict's meaning: the record
    /// never claims timeless existence.
    pub fetched_at: String,
    /// The dated verdict.
    pub verdict: Verdict,
    /// Where document and source disagree.
    pub diffs: Vec<FieldDiff>,
    /// Why verification failed, when it did. Mandatory for
    /// [`BibVerdict::Unverified`] and [`Reachability::Unreachable`].
    pub failure_note: Option<String>,
}

/// A parsed verification record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationRecord {
    /// The record's claims, in file order.
    pub claims: Vec<VerifiedClaim>,
}

/// The format version this module reads and writes.
const VERSION: i64 = 1;

/// Why a record was refused. The message is for the diagnostic (`XT1015`);
/// nothing partial survives a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordError {
    /// What was wrong, in a sentence.
    pub message: String,
}

fn refuse<T>(message: impl Into<String>) -> Result<T, RecordError> {
    Err(RecordError {
        message: message.into(),
    })
}

/// Parses a `.xtexverified` record, whole or not at all.
///
/// # Errors
///
/// Any malformation — bad JSON, an unknown kind or verdict, a verdict in
/// the wrong vocabulary for its kind, a missing date, an `unverified` or
/// `unreachable` claim without its note — refuses the whole record.
pub fn parse_record(bytes: &[u8]) -> Result<VerificationRecord, RecordError> {
    let Some(value) = json::parse(bytes) else {
        return refuse("the record is not valid JSON");
    };
    let Some(version) = value.get("version").and_then(Value::integer) else {
        return refuse("the record carries no version");
    };
    if version != VERSION {
        return refuse(format!("record version {version} is not {VERSION}"));
    }
    let Some(Value::List(raw_claims)) = value.get("claims") else {
        return refuse("the record carries no claims list");
    };
    let mut claims = Vec::new();
    for (index, raw) in raw_claims.iter().enumerate() {
        claims.push(parse_claim(raw).map_err(|error| RecordError {
            message: format!("claim {index}: {}", error.message),
        })?);
    }
    Ok(VerificationRecord { claims })
}

fn field(raw: &Value, name: &str) -> Result<String, RecordError> {
    match raw.get(name).and_then(Value::text) {
        Some(text) => Ok(text.to_owned()),
        None => refuse(format!("`{name}` is missing")),
    }
}

fn parse_claim(raw: &Value) -> Result<VerifiedClaim, RecordError> {
    let kind_name = field(raw, "kind")?;
    let Some(kind) = ClaimKind::from_name(&kind_name) else {
        return refuse(format!("unknown kind `{kind_name}`"));
    };
    let target = field(raw, "target")?;
    let verdict_name = field(raw, "verdict")?;
    let verdict = match kind {
        ClaimKind::BibEntry => match BibVerdict::from_name(&verdict_name) {
            Some(v) => Verdict::Bibliographic(v),
            None => {
                return refuse(format!("`{verdict_name}` is not a bibliographic verdict"));
            }
        },
        _ => match Reachability::from_name(&verdict_name) {
            Some(v) => Verdict::Reachability(v),
            None => {
                return refuse(format!("`{verdict_name}` is not a reachability verdict"));
            }
        },
    };
    let fetched_at = field(raw, "fetched_at")?;
    if fetched_at.is_empty() {
        return refuse("`fetched_at` is empty — a verdict is a dated statement");
    }
    let source = field(raw, "source")?;
    let response_hash = raw
        .get("response_hash")
        .and_then(Value::text)
        .unwrap_or("")
        .to_owned();
    let failure_note = raw
        .get("failure_note")
        .and_then(Value::text)
        .map(str::to_owned);
    let failed = matches!(
        verdict,
        Verdict::Bibliographic(BibVerdict::Unverified)
            | Verdict::Reachability(Reachability::Unreachable)
    );
    if failed && failure_note.as_deref().unwrap_or("").is_empty() {
        return refuse(format!(
            "`{verdict_name}` without a failure note — an unanswered source is never \"probably fine\""
        ));
    }
    if !failed && response_hash.is_empty() {
        return refuse("`response_hash` is empty on a verdict that claims an answer");
    }
    let mut fields = Vec::new();
    if let Some(Value::Map(entries)) = raw.get("fields") {
        for (name, value) in entries {
            let Some(text) = value.text() else {
                return refuse(format!("field `{name}` is not text"));
            };
            fields.push((name.clone(), text.to_owned()));
        }
    }
    let mut diffs = Vec::new();
    if let Some(Value::List(raw_diffs)) = raw.get("diffs") {
        for raw_diff in raw_diffs {
            let severity_name = field(raw_diff, "severity")?;
            let severity = match severity_name.as_str() {
                "high" => DiffSeverity::High,
                "medium" => DiffSeverity::Medium,
                other => return refuse(format!("unknown severity `{other}`")),
            };
            diffs.push(FieldDiff {
                field: field(raw_diff, "field")?,
                in_document: field(raw_diff, "in_document")?,
                in_source: field(raw_diff, "in_source")?,
                severity,
            });
        }
    }
    Ok(VerifiedClaim {
        kind,
        target,
        fields,
        response_hash,
        source,
        fetched_at,
        verdict,
        diffs,
        failure_note,
    })
}

/// Renders a record as canonical JSON, the shape [`parse_record`] reads.
#[must_use]
pub fn write_record(record: &VerificationRecord) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = write!(out, "{{\"version\":{VERSION},\"claims\":[");
    for (index, claim) in record.claims.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"kind\":");
        json::write_text(claim.kind.name(), &mut out);
        out.push_str(",\"target\":");
        json::write_text(&claim.target, &mut out);
        out.push_str(",\"verdict\":");
        let verdict_name = match claim.verdict {
            Verdict::Bibliographic(v) => v.name(),
            Verdict::Reachability(v) => v.name(),
        };
        json::write_text(verdict_name, &mut out);
        out.push_str(",\"source\":");
        json::write_text(&claim.source, &mut out);
        out.push_str(",\"fetched_at\":");
        json::write_text(&claim.fetched_at, &mut out);
        out.push_str(",\"response_hash\":");
        json::write_text(&claim.response_hash, &mut out);
        out.push_str(",\"fields\":{");
        for (findex, (name, value)) in claim.fields.iter().enumerate() {
            if findex > 0 {
                out.push(',');
            }
            json::write_text(name, &mut out);
            out.push(':');
            json::write_text(value, &mut out);
        }
        out.push_str("},\"diffs\":[");
        for (dindex, diff) in claim.diffs.iter().enumerate() {
            if dindex > 0 {
                out.push(',');
            }
            out.push_str("{\"field\":");
            json::write_text(&diff.field, &mut out);
            out.push_str(",\"in_document\":");
            json::write_text(&diff.in_document, &mut out);
            out.push_str(",\"in_source\":");
            json::write_text(&diff.in_source, &mut out);
            out.push_str(",\"severity\":");
            json::write_text(
                match diff.severity {
                    DiffSeverity::High => "high",
                    DiffSeverity::Medium => "medium",
                },
                &mut out,
            );
            out.push('}');
        }
        out.push(']');
        if let Some(note) = &claim.failure_note {
            out.push_str(",\"failure_note\":");
            json::write_text(note, &mut out);
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

/// Days since the civil epoch for an RFC 3339 date's day part, or `None`
/// for a shape this reader does not recognise. Enough calendar for an age
/// in days; not a datetime library and not on its way to becoming one.
/// Public because the verifier asks the same question about freshness.
#[must_use]
pub fn civil_days(date: &str) -> Option<i64> {
    let bytes = date.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        std::str::from_utf8(&bytes[range]).ok()?.parse().ok()
    };
    let (y, m, d) = (number(0..4)?, number(5..7)?, number(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days-from-civil, the standard branchless form.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// What the record-aware check needs, all supplied by the caller — this
/// module owns no clock and no filesystem, deliberately.
pub struct RecordCheck<'a> {
    /// The parsed record.
    pub record: &'a VerificationRecord,
    /// The document's claims (see [`crate::claims`]).
    pub claims: &'a [crate::claims::Claim],
    /// The symbol table, for the one severity question: does an explicit
    /// `@cite` demand this key?
    pub table: &'a crate::symbols::SymbolTable,
    /// Today, RFC 3339 — the caller's clock, never ours.
    pub now: &'a str,
    /// The freshness window, in days.
    pub max_age_days: i64,
}

/// The deterministic half of verification: findings from crossing the
/// document's claims with the record — offline, reproducible, dated.
///
/// Three findings exist, each speaking its date plainly:
/// - the claim was **edited since its verification** (the document's fields
///   no longer match the record's), which retires the old verdict — the
///   drift is reported and the verdict is not replayed;
/// - the verification **expired** (older than the window);
/// - the recorded verdict itself: `partial`/`mismatch` replayed with their
///   diffs, `unverified`/`unreachable` with their notes.
///
/// Severity follows the standing policy: a bibliographic finding is a hard
/// error only when an explicit `@cite` demands the key AND the finding is a
/// mismatch or a high-severity diff (the author list). Everything else —
/// every reachability finding included, because a plain `\url` is not a
/// construct — is advisory.
#[must_use]
pub fn check_against_record(input: &RecordCheck<'_>) -> Vec<crate::check::Diagnostic> {
    use crate::check::{Blame, Diagnostic, Severity};
    use crate::symbols::EntityClass;
    let mut findings = Vec::new();
    let demanded: std::collections::BTreeSet<&str> =
        input.table.citations().map(|(key, _)| key).collect();
    let today = civil_days(input.now);

    for claim in input.claims {
        let Some(recorded) = input
            .record
            .claims
            .iter()
            .find(|r| r.kind == claim.kind && r.target == claim.target)
        else {
            continue; // never verified: the verifier's business, not a finding
        };
        let entity = match claim.kind {
            ClaimKind::BibEntry => EntityClass::Citation,
            _ => EntityClass::UnknownOpen,
        };
        let date = &recorded.fetched_at[..recorded.fetched_at.len().min(10)];
        let mut push = |code: &'static str, severity: Severity, message: String| {
            findings.push(Diagnostic {
                code,
                entity,
                name: Some(claim.target.clone()),
                source: claim.source,
                span: claim.span,
                message,
                related: Vec::new(),
                severity,
                blame: if severity == Severity::Error {
                    Blame::XtexConstruct
                } else {
                    Blame::Unresolved
                },
            });
        };

        // Drift retires the verdict: an edited claim's old diffs are noise.
        if claim.kind == ClaimKind::BibEntry && claim.fields != recorded.fields {
            push(
                "XT1016",
                Severity::Advisory,
                format!(
                    "`{}` was edited after its verification ({date}) — the recorded verdict no longer speaks for it; verify again",
                    claim.target
                ),
            );
            continue;
        }

        match (today, civil_days(&recorded.fetched_at)) {
            (Some(now), Some(then)) => {
                let age = now - then;
                if age > input.max_age_days {
                    push(
                        "XT1017",
                        Severity::Advisory,
                        format!(
                            "`{}` was last verified {age} days ago ({date}) — older than the {}-day window",
                            claim.target, input.max_age_days
                        ),
                    );
                }
            }
            _ => {
                push(
                    "XT1017",
                    Severity::Advisory,
                    format!("`{}` carries an unreadable verification date", claim.target),
                );
            }
        }

        replay_verdict(recorded, &demanded, claim, date, &mut push);
    }
    findings
}

/// Replays one recorded verdict as its finding, dated. Split from
/// [`check_against_record`] to keep each half readable on one screen.
fn replay_verdict(
    recorded: &VerifiedClaim,
    demanded: &std::collections::BTreeSet<&str>,
    claim: &crate::claims::Claim,
    date: &str,
    push: &mut impl FnMut(&'static str, crate::check::Severity, String),
) {
    use crate::check::Severity;
    use std::fmt::Write as _;
    match recorded.verdict {
        Verdict::Bibliographic(BibVerdict::Verified)
        | Verdict::Reachability(Reachability::Reachable) => {}
        Verdict::Bibliographic(BibVerdict::Partial) => {
            let matched: Vec<&str> = recorded
                .fields
                .iter()
                .map(|(name, _)| name.as_str())
                .filter(|name| !recorded.diffs.iter().any(|diff| diff.field == *name))
                .collect();
            let mut message = format!("partial against {} (as of {date})", recorded.source);
            if !matched.is_empty() {
                let _ = write!(message, " — {} match", matched.join(", "));
            }
            for diff in &recorded.diffs {
                let _ = write!(
                    message,
                    "; {} differs: this entry says \"{}\", the source says \"{}\"",
                    diff.field, diff.in_document, diff.in_source
                );
            }
            let hard = demanded.contains(claim.target.as_str())
                && recorded
                    .diffs
                    .iter()
                    .any(|diff| diff.severity == DiffSeverity::High);
            push(
                "XT1018",
                if hard {
                    Severity::Error
                } else {
                    Severity::Advisory
                },
                message,
            );
        }
        Verdict::Bibliographic(BibVerdict::Mismatch) => {
            let hard = demanded.contains(claim.target.as_str());
            push(
                "XT1018",
                if hard {
                    Severity::Error
                } else {
                    Severity::Advisory
                },
                format!(
                    "the source answers with a different work ({}, as of {date})",
                    recorded.source
                ),
            );
        }
        Verdict::Bibliographic(BibVerdict::Unverified) => {
            push(
                "XT1019",
                Severity::Advisory,
                format!(
                    "could not be verified ({date}): {}",
                    recorded.failure_note.as_deref().unwrap_or("no note")
                ),
            );
        }
        Verdict::Reachability(Reachability::Unreachable) => {
            push(
                "XT1019",
                Severity::Advisory,
                format!(
                    "unreachable as of {date}: {}",
                    recorded.failure_note.as_deref().unwrap_or("no note")
                ),
            );
        }
        Verdict::Reachability(Reachability::Redirected) => {
            push(
                "XT1019",
                Severity::Advisory,
                format!("answered from somewhere else as of {date} — worth a look"),
            );
        }
    }
}
