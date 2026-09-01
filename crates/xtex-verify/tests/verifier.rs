//! The verifier against a canned transport: zero live network, and the
//! properties the issue names — incremental by fingerprint, bounded
//! retries, a record that keeps what finished.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

use xtex_core::claims::Claim;
use xtex_core::source::{Sources, Span};
use xtex_core::verification::{BibVerdict, ClaimKind, Reachability, Verdict, parse_record};
use xtex_verify::run::{Run, render, verify};
use xtex_verify::transport::{Response, Transport, TransportError};

/// One scripted answer: status and body, or a named failure.
type Script = Result<(u16, &'static str), &'static str>;

/// Answers from a script keyed by a URL fragment, counting every call.
struct Canned {
    answers: Vec<(&'static str, Script)>,
    calls: RefCell<Vec<String>>,
}

impl Transport for Canned {
    fn get(&self, url: &str, _agent: &str, _timeout: Duration) -> Result<Response, TransportError> {
        self.calls.borrow_mut().push(url.to_owned());
        for (fragment, script) in &self.answers {
            if url.contains(fragment) {
                return match script {
                    Ok((status, body)) => Ok(Response {
                        status: *status,
                        body: body.as_bytes().to_vec(),
                        location: None,
                    }),
                    Err("timeout") => Err(TransportError::Timeout),
                    Err(other) => Err(TransportError::Other((*other).to_owned())),
                };
            }
        }
        Err(TransportError::Other(format!("unscripted url: {url}")))
    }
}

fn sources_stub() -> Sources {
    let mut sources = Sources::new();
    sources.add("main.tex", b"stub".to_vec());
    sources
}

fn bib(key: &str, fields: &[(&str, &str)]) -> Claim {
    let sources = sources_stub();
    let _ = &sources;
    Claim {
        kind: ClaimKind::BibEntry,
        target: key.to_owned(),
        source: {
            let mut s = Sources::new();
            s.add("main.tex", b"x".to_vec())
        },
        span: Span::new(0, 1),
        fields: fields
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

fn url_claim(target: &str) -> Claim {
    Claim {
        kind: ClaimKind::Url,
        target: target.to_owned(),
        source: {
            let mut s = Sources::new();
            s.add("main.tex", b"x".to_vec())
        },
        span: Span::new(0, 1),
        fields: Vec::new(),
    }
}

const OPENALEX_TWO: &str = r#"{"results":[
  {"doi":"https://doi.org/10.1/good","title":"The TeXbook","publication_year":1984,
   "authorships":[{"author":{"display_name":"Donald E. Knuth"}}]},
  {"doi":"https://doi.org/10.1/partial","title":"LaTeX","publication_year":1994,
   "authorships":[{"author":{"display_name":"Leslie Lamport"}},{"author":{"display_name":"X. Fabricated"}}]}
]}"#;

fn claims_set() -> Vec<Claim> {
    vec![
        bib(
            "good",
            &[
                ("title", "The {TeX}book"),
                ("author", "Donald E. Knuth"),
                ("year", "1984"),
                ("doi", "10.1/good"),
            ],
        ),
        bib(
            "partial",
            &[
                ("title", "LaTeX"),
                ("author", "Leslie Lamport"),
                ("year", "1994"),
                ("doi", "10.1/partial"),
            ],
        ),
        url_claim("https://example.org/alive"),
        url_claim("https://example.org/dead"),
    ]
}

fn run_with(canned: &Canned, claims: &[Claim], previous: Option<&[u8]>) -> (String, Vec<String>) {
    let mut persisted = Vec::new();
    let mut persist = |record: &xtex_core::verification::VerificationRecord| {
        persisted.push(render(record));
    };
    let mut progress = |_: &str| {};
    let mut run = Run {
        transport: canned,
        user_agent: "test".to_owned(),
        now: "2026-09-01T00:00:00Z".to_owned(),
        max_age_days: 30,
        timeout: Duration::from_millis(10),
        persist: &mut persist,
        progress: &mut progress,
    };
    let (record, _) = verify(&mut run, claims, previous);
    (render(&record), canned.calls.borrow().clone())
}

#[test]
fn a_mixed_run_settles_every_claim_with_its_own_verdict() {
    let canned = Canned {
        answers: vec![
            ("openalex", Ok((200, OPENALEX_TWO))),
            ("alive", Ok((200, "ok"))),
            ("dead", Err("timeout")),
        ],
        calls: RefCell::new(Vec::new()),
    };
    let (written, calls) = run_with(&canned, &claims_set(), None);
    let record = parse_record(written.as_bytes()).expect("the run writes a valid record");
    let by_target: BTreeMap<&str, &_> = record
        .claims
        .iter()
        .map(|c| (c.target.as_str(), c))
        .collect();
    assert_eq!(
        by_target["good"].verdict,
        Verdict::Bibliographic(BibVerdict::Verified)
    );
    assert_eq!(
        by_target["partial"].verdict,
        Verdict::Bibliographic(BibVerdict::Partial)
    );
    assert!(
        by_target["partial"]
            .diffs
            .iter()
            .any(|d| d.field == "authors"),
        "the fabricated co-author surfaces: {:?}",
        by_target["partial"].diffs
    );
    assert_eq!(
        by_target["https://example.org/alive"].verdict,
        Verdict::Reachability(Reachability::Reachable)
    );
    let dead = by_target["https://example.org/dead"];
    assert_eq!(
        dead.verdict,
        Verdict::Reachability(Reachability::Unreachable)
    );
    assert!(
        dead.failure_note
            .as_deref()
            .unwrap_or("")
            .contains("timed out"),
        "{:?}",
        dead.failure_note
    );
    // ONE OpenAlex request carried both DOIs: the batch is real.
    assert_eq!(
        calls.iter().filter(|c| c.contains("openalex")).count(),
        1,
        "{calls:?}"
    );
}

#[test]
fn a_second_unchanged_run_touches_no_network_for_the_settled() {
    let canned = Canned {
        answers: vec![
            ("openalex", Ok((200, OPENALEX_TWO))),
            ("alive", Ok((200, "ok"))),
            ("dead", Ok((200, "revived"))),
        ],
        calls: RefCell::new(Vec::new()),
    };
    let claims = claims_set();
    let (written, _) = run_with(&canned, &claims, None);

    let second = Canned {
        answers: vec![
            ("openalex", Ok((200, OPENALEX_TWO))),
            ("alive", Ok((200, "ok"))),
            ("dead", Ok((200, "still here"))),
        ],
        calls: RefCell::new(Vec::new()),
    };
    let (rewritten, calls) = run_with(&second, &claims, Some(written.as_bytes()));
    // Everything answered stays carried over; nothing was refetched.
    assert!(
        calls.is_empty(),
        "an unchanged fresh run refetched: {calls:?}"
    );
    let record = parse_record(rewritten.as_bytes()).expect("parses");
    assert_eq!(record.claims.len(), claims.len());
}

#[test]
fn an_edited_entry_is_refetched_and_the_rest_stay_skipped() {
    let canned = Canned {
        answers: vec![
            ("openalex", Ok((200, OPENALEX_TWO))),
            ("alive", Ok((200, "ok"))),
            ("dead", Ok((200, "ok"))),
        ],
        calls: RefCell::new(Vec::new()),
    };
    let mut claims = claims_set();
    let (written, _) = run_with(&canned, &claims, None);

    // The author edits one entry's title.
    claims[0].fields[0].1 = "The TeXbook, annotated".to_owned();
    let second = Canned {
        answers: vec![("openalex", Ok((200, OPENALEX_TWO)))],
        calls: RefCell::new(Vec::new()),
    };
    let (_, calls) = run_with(&second, &claims, Some(written.as_bytes()));
    assert_eq!(calls.len(), 1, "only the edited entry pays: {calls:?}");
    assert!(calls[0].contains("openalex"));
}

#[test]
fn the_record_keeps_what_finished_as_it_goes() {
    // The persist callback sees the record after every settled claim: an
    // interrupted run keeps everything before the interruption.
    let canned = Canned {
        answers: vec![
            ("openalex", Ok((200, OPENALEX_TWO))),
            ("alive", Ok((200, "ok"))),
            ("dead", Err("timeout")),
        ],
        calls: RefCell::new(Vec::new()),
    };
    let mut snapshots = Vec::new();
    let mut persist = |record: &xtex_core::verification::VerificationRecord| {
        snapshots.push(record.claims.len());
    };
    let mut progress = |_: &str| {};
    let mut run = Run {
        transport: &canned,
        user_agent: "test".to_owned(),
        now: "2026-09-01T00:00:00Z".to_owned(),
        max_age_days: 30,
        timeout: Duration::from_millis(10),
        persist: &mut persist,
        progress: &mut progress,
    };
    let (record, _) = verify(&mut run, &claims_set(), None);
    assert_eq!(record.claims.len(), 4);
    assert_eq!(
        snapshots,
        vec![1, 2, 3, 4],
        "one snapshot per settled claim"
    );
}

#[test]
fn retries_are_bounded_by_the_budget() {
    let canned = Canned {
        answers: vec![
            ("openalex", Err("connection refused")),
            ("alive", Err("connection refused")),
            ("dead", Err("connection refused")),
        ],
        calls: RefCell::new(Vec::new()),
    };
    let (written, calls) = run_with(&canned, &claims_set(), None);
    let record = parse_record(written.as_bytes()).expect("still a valid record");
    assert_eq!(
        record.claims.len(),
        4,
        "every claim settles even when the net is down"
    );
    for claim in &record.claims {
        assert!(
            claim.failure_note.is_some(),
            "an unanswered claim carries its note: {claim:?}"
        );
    }
    // Hard ceiling: at most 3 attempts per request, and the global budget
    // (1 retry per 10 requests) tightens it further.
    assert!(calls.len() <= 3 * 3, "retries stormed: {}", calls.len());
}

#[test]
fn the_run_measures_its_own_network() {
    let canned = Canned {
        answers: vec![
            ("openalex", Ok((200, OPENALEX_TWO))),
            ("alive", Ok((200, "ok"))),
            ("dead", Err("timeout")),
        ],
        calls: RefCell::new(Vec::new()),
    };
    let mut persist = |_: &xtex_core::verification::VerificationRecord| {};
    let mut progress = |_: &str| {};
    let mut run = Run {
        transport: &canned,
        user_agent: "test".to_owned(),
        now: "2026-09-01T00:00:00Z".to_owned(),
        max_age_days: 30,
        timeout: Duration::from_millis(10),
        persist: &mut persist,
        progress: &mut progress,
    };
    let (_, metrics) = verify(&mut run, &claims_set(), None);
    assert_eq!(metrics.requests.get("openalex"), Some(&1), "{metrics:?}");
    assert_eq!(metrics.fetched, 3, "{metrics:?}");
    assert_eq!(metrics.unanswered, 1, "{metrics:?}");
    assert_eq!(metrics.carried_over, 0);
    assert!(metrics.bytes_down > 0, "{metrics:?}");
    let line = metrics.summary();
    assert!(line.contains("fetched"), "{line}");
}
