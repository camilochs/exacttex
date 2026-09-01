//! The run: claims in, record out — incremental, budgeted, polite.

use std::collections::BTreeMap;
use std::time::Duration;

use xtex_core::claims::Claim;
use xtex_core::verification::{
    BibVerdict, ClaimKind, Reachability, Verdict, VerificationRecord, VerifiedClaim, parse_record,
    write_record,
};

use crate::bucket::Bucket;
use crate::sources::{Lookup, crossref_by_query, openalex_by_doi};
use crate::transport::{Transport, TransportError};

/// Everything one run needs, supplied by the caller.
pub struct Run<'a> {
    /// The seam the network passes through.
    pub transport: &'a dyn Transport,
    /// Sent to every source; joining the polite pools wants a mailto here.
    pub user_agent: String,
    /// Today, RFC 3339 UTC — stamped on every verdict.
    pub now: String,
    /// The freshness window in days: anything younger, unchanged and
    /// answered is skipped without touching the network.
    pub max_age_days: i64,
    /// Per-request timeout.
    pub timeout: Duration,
    /// Called after every claim settles, with the whole record so far —
    /// the caller persists it, and an interrupted run keeps what finished.
    pub persist: &'a mut dyn FnMut(&VerificationRecord),
    /// One line per event, for stderr.
    pub progress: &'a mut dyn FnMut(&str),
}

/// What the run did to the network, measured: the numbers the lab's own
/// provenance rule asks of any measurement — a run that cannot say what
/// it did is a run nobody can compare.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Metrics {
    /// Requests actually sent, per source family (openalex, crossref, http).
    pub requests: std::collections::BTreeMap<String, u32>,
    /// Retries spent inside the budget.
    pub retries: u32,
    /// Response bytes read.
    pub bytes_down: u64,
    /// Claims answered by the network this run.
    pub fetched: usize,
    /// Claims carried over fresh, no network paid.
    pub carried_over: usize,
    /// Claims that settled without an answer (timeout, refusal).
    pub unanswered: usize,
}

impl Metrics {
    /// One line for stderr: the run, said in numbers.
    #[must_use]
    pub fn summary(&self) -> String {
        let total: u32 = self.requests.values().sum();
        let per_source: Vec<String> = self
            .requests
            .iter()
            .map(|(source, count)| format!("{source} {count}"))
            .collect();
        format!(
            "network: {total} requests ({}) · {} retries · {} bytes down · {} fetched, {} carried over, {} unanswered",
            per_source.join(", "),
            self.retries,
            self.bytes_down,
            self.fetched,
            self.carried_over,
            self.unanswered
        )
    }
}

/// The retry budget, SRE-style: three attempts per request, and globally
/// no more than one retry per ten requests — a dead host bounds the run
/// instead of storming it.
struct Budget {
    requests: u32,
    retries: u32,
}

impl Budget {
    fn may_retry(&self) -> bool {
        self.retries * 10 < self.requests.max(10)
    }
}

fn with_budget<T>(
    budget: &mut Budget,
    bucket: &mut Bucket,
    metrics: &mut Metrics,
    source: &str,
    mut attempt: impl FnMut() -> Result<T, TransportError>,
) -> Result<T, TransportError> {
    let mut tries: u32 = 0;
    loop {
        bucket.take();
        budget.requests += 1;
        *metrics.requests.entry(source.to_owned()).or_default() += 1;
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) => {
                tries += 1;
                if tries >= 3 || !budget.may_retry() {
                    return Err(error);
                }
                budget.retries += 1;
                std::thread::sleep(Duration::from_millis(200 * u64::from(tries)));
            }
        }
    }
}

/// The note a registry's "nothing matched" is recorded with. Its first
/// words are what [`is_settled_negative`] reads, so they are fixed here.
pub const NOT_FOUND_NOTE: &str = "not found at the registries — books published before DOIs often are not registered; the entry may still be correct";

/// Whether an unanswered-looking claim is in fact a settled negative: the
/// source answered, and the answer was "no".
///
/// A registry that answers "nothing matched" has answered; a server that
/// answers 404 has answered. Neither is a transport failure, and neither
/// changes on the next run unless the entry changes or the freshness
/// window elapses. Retrying them every run cost half of a full run,
/// forever, on bibliographies where 39% of entries are not in Crossref
/// (corpus E6: run 2 spent 325 of run 1's 688 requests re-asking 382
/// claims that had been answered "not found"). A timeout, a refusal or a
/// malformed answer keeps its failure note and is retried.
#[must_use]
pub fn is_settled_negative(recorded: &VerifiedClaim) -> bool {
    let Some(note) = recorded.failure_note.as_deref() else {
        return false;
    };
    match recorded.verdict {
        Verdict::Bibliographic(BibVerdict::Unverified) => {
            note.starts_with("not found at the registries")
        }
        Verdict::Reachability(Reachability::Unreachable) => note.starts_with("answered "),
        _ => false,
    }
}

/// True when the recorded claim still answers for the document's: same
/// fields, young enough, and either answered or a settled negative.
fn still_fresh(claim: &Claim, recorded: &VerifiedClaim, now: &str, max_age_days: i64) -> bool {
    if recorded.fields != claim.fields {
        return false;
    }
    let unanswered = matches!(
        recorded.verdict,
        Verdict::Bibliographic(BibVerdict::Unverified)
            | Verdict::Reachability(Reachability::Unreachable)
    );
    if unanswered && !is_settled_negative(recorded) {
        return false; // a transport failure is retried on the next run, always
    }
    match (super::civil(now), super::civil(&recorded.fetched_at)) {
        (Some(today), Some(then)) => today - then <= max_age_days,
        _ => false,
    }
}

fn verdict_for(
    claim: &Claim,
    lookup: &Lookup,
) -> (Verdict, Vec<xtex_core::verification::FieldDiff>) {
    let mut diffs = Vec::new();
    let mut compared = 0;
    for (name, in_document) in &claim.fields {
        if !matches!(name.as_str(), "title" | "author" | "authors" | "year") {
            continue;
        }
        let source_name = if name == "author" {
            "authors"
        } else {
            name.as_str()
        };
        let Some((_, in_source)) = lookup
            .fields
            .iter()
            .find(|(n, _)| n == source_name || (source_name == "authors" && n == "author"))
        else {
            continue;
        };
        compared += 1;
        if let Some(diff) = crate::compare::diff_field(name, in_document, in_source) {
            diffs.push(diff);
        }
    }
    let verdict = if compared == 0 {
        // Nothing comparable answered: the title alone decides mismatch.
        Verdict::Bibliographic(BibVerdict::Unverified)
    } else if diffs.is_empty() {
        Verdict::Bibliographic(BibVerdict::Verified)
    } else if diffs.iter().any(|d| d.field == "title") {
        Verdict::Bibliographic(BibVerdict::Mismatch)
    } else {
        Verdict::Bibliographic(BibVerdict::Partial)
    };
    (verdict, diffs)
}

fn field<'a>(claim: &'a Claim, name: &str) -> &'a str {
    claim
        .fields
        .iter()
        .find(|(n, _)| n == name)
        .map_or("", |(_, v)| v.as_str())
}

/// Runs the verification and returns the record. See the module doc for
/// the shape; the network enters nowhere else in this repository.
#[allow(clippy::too_many_lines)]
pub fn verify(
    run: &mut Run<'_>,
    claims: &[Claim],
    previous: Option<&[u8]>,
) -> (VerificationRecord, Metrics) {
    let mut metrics = Metrics::default();
    let earlier: BTreeMap<(ClaimKind, String), VerifiedClaim> = previous
        .and_then(|bytes| parse_record(bytes).ok())
        .map(|record| {
            record
                .claims
                .into_iter()
                .map(|claim| ((claim.kind, claim.target.clone()), claim))
                .collect()
        })
        .unwrap_or_default();

    let mut record = VerificationRecord { claims: Vec::new() };
    let mut budget = Budget {
        requests: 0,
        retries: 0,
    };
    let mut openalex_bucket = Bucket::new(Duration::from_millis(120));
    let mut crossref_bucket = Bucket::new(Duration::from_millis(350));
    let mut http_bucket = Bucket::new(Duration::from_millis(150));
    let mut skipped = 0usize;

    // Partition: fresh-and-unchanged claims are carried over untouched.
    let mut pending: Vec<&Claim> = Vec::new();
    for claim in claims {
        if let Some(recorded) = earlier.get(&(claim.kind, claim.target.clone()))
            && still_fresh(claim, recorded, &run.now, run.max_age_days)
        {
            record.claims.push(recorded.clone());
            skipped += 1;
            metrics.carried_over += 1;
            continue;
        }
        pending.push(claim);
    }
    (run.progress)(&format!(
        "{} claims: {} fresh and carried over, {} to verify",
        claims.len(),
        skipped,
        pending.len()
    ));

    // Phase one: every bib entry WITH a doi, batched through OpenAlex.
    let mut answered: BTreeMap<String, Lookup> = BTreeMap::new();
    let doi_entries: Vec<&&Claim> = pending
        .iter()
        .filter(|c| c.kind == ClaimKind::BibEntry && !field(c, "doi").is_empty())
        .collect();
    for batch in doi_entries.chunks(50) {
        let dois: Vec<&str> = batch.iter().map(|c| field(c, "doi")).collect();
        match with_budget(
            &mut budget,
            &mut openalex_bucket,
            &mut metrics,
            "openalex",
            || openalex_by_doi(run.transport, &run.user_agent, run.timeout, &dois),
        ) {
            Ok(found) => {
                for (doi, lookup) in found {
                    metrics.bytes_down += lookup.bytes as u64;
                    answered.insert(doi, lookup);
                }
            }
            Err(error) => (run.progress)(&format!("openalex batch failed: {error}")),
        }
    }

    for claim in pending {
        let settled = match claim.kind {
            ClaimKind::BibEntry => {
                let doi = field(claim, "doi").to_lowercase();
                let lookup = if doi.is_empty() {
                    match with_budget(
                        &mut budget,
                        &mut crossref_bucket,
                        &mut metrics,
                        "crossref",
                        || {
                            crossref_by_query(
                                run.transport,
                                &run.user_agent,
                                run.timeout,
                                field(claim, "title"),
                                field(claim, "author"),
                                field(claim, "year"),
                            )
                        },
                    ) {
                        Ok(Some(lookup)) => {
                            metrics.bytes_down += lookup.bytes as u64;
                            Some(lookup)
                        }
                        Ok(None) => None,
                        Err(error) => {
                            record
                                .claims
                                .push(unverified(claim, run, &error.to_string()));
                            (run.persist)(&record);
                            continue;
                        }
                    }
                } else {
                    answered.remove(&doi)
                };
                match lookup {
                    Some(lookup) => {
                        let (verdict, diffs) = verdict_for(claim, &lookup);
                        VerifiedClaim {
                            kind: claim.kind,
                            target: claim.target.clone(),
                            fields: claim.fields.clone(),
                            response_hash: lookup.fingerprint,
                            source: lookup.source,
                            fetched_at: run.now.clone(),
                            verdict,
                            diffs,
                            failure_note: None,
                        }
                    }
                    None => unverified(claim, run, NOT_FOUND_NOTE),
                }
            }
            ClaimKind::Url | ClaimKind::Doi | ClaimKind::Repository => {
                let url = if claim.kind == ClaimKind::Doi {
                    format!("https://doi.org/{}", claim.target)
                } else {
                    claim.target.clone()
                };
                match with_budget(&mut budget, &mut http_bucket, &mut metrics, "http", || {
                    run.transport.get(&url, &run.user_agent, run.timeout)
                }) {
                    Ok(response) => {
                        metrics.bytes_down += response.body.len() as u64;
                        let verdict = if (300..400).contains(&response.status) {
                            Verdict::Reachability(Reachability::Redirected)
                        } else if response.status < 400 {
                            Verdict::Reachability(Reachability::Reachable)
                        } else {
                            Verdict::Reachability(Reachability::Unreachable)
                        };
                        let mut settled = VerifiedClaim {
                            kind: claim.kind,
                            target: claim.target.clone(),
                            fields: Vec::new(),
                            response_hash: crate::compare::fingerprint(&response.body),
                            source: "http".to_owned(),
                            fetched_at: run.now.clone(),
                            verdict,
                            diffs: Vec::new(),
                            failure_note: None,
                        };
                        if settled.verdict == Verdict::Reachability(Reachability::Unreachable) {
                            settled.failure_note = Some(format!("answered {}", response.status));
                            settled.response_hash = String::new();
                        }
                        settled
                    }
                    Err(error) => {
                        let mut settled = unverified(claim, run, &error.to_string());
                        settled.verdict = Verdict::Reachability(Reachability::Unreachable);
                        settled
                    }
                }
            }
        };
        if settled.failure_note.is_some() {
            metrics.unanswered += 1;
        } else {
            metrics.fetched += 1;
        }
        (run.progress)(&format!("{} · settled", settled.target));
        record.claims.push(settled);
        (run.persist)(&record);
    }
    metrics.retries = budget.retries;
    (run.progress)(&metrics.summary());
    (record, metrics)
}

fn unverified(claim: &Claim, run: &Run<'_>, note: &str) -> VerifiedClaim {
    VerifiedClaim {
        kind: claim.kind,
        target: claim.target.clone(),
        fields: claim.fields.clone(),
        response_hash: String::new(),
        source: "none".to_owned(),
        fetched_at: run.now.clone(),
        verdict: match claim.kind {
            ClaimKind::BibEntry => Verdict::Bibliographic(BibVerdict::Unverified),
            _ => Verdict::Reachability(Reachability::Unreachable),
        },
        diffs: Vec::new(),
        failure_note: Some(note.to_owned()),
    }
}

/// Writes the record the CLI persists.
#[must_use]
pub fn render(record: &VerificationRecord) -> String {
    write_record(record)
}
