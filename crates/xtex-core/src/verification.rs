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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
