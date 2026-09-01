//! The claim under measurement: this verifier EXPLOITS an unstable
//! network — it finishes bounded, keeps everything it obtained, and the
//! next run completes the holes cheaply. Deterministic: failures come
//! from a seeded generator and latency is simulated, so CI measures the
//! algorithm, never the weather.

use std::cell::RefCell;
use std::time::Duration;

use xtex_core::claims::Claim;
use xtex_core::source::{Sources, Span};
use xtex_core::verification::{ClaimKind, parse_record};
use xtex_verify::run::{Run, render, verify};
use xtex_verify::transport::{Response, Transport, TransportError};

/// A deterministic linear congruential generator — the weather, seeded.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0 >> 33
    }
}

/// Fails a seeded fraction of requests; charges simulated latency for
/// every call (100ms an answer, a full timeout for a failure).
struct Flaky {
    fail_percent: u64,
    rng: RefCell<Lcg>,
    calls: RefCell<u32>,
    simulated_ms: RefCell<u64>,
    body: &'static str,
}

impl Transport for Flaky {
    fn get(&self, _url: &str, _agent: &str, timeout: Duration) -> Result<Response, TransportError> {
        *self.calls.borrow_mut() += 1;
        if self.rng.borrow_mut().next() % 100 < self.fail_percent {
            *self.simulated_ms.borrow_mut() += u64::try_from(timeout.as_millis()).unwrap_or(0);
            return Err(TransportError::Timeout);
        }
        *self.simulated_ms.borrow_mut() += 100;
        Ok(Response {
            status: 200,
            body: self.body.as_bytes().to_vec(),
            location: None,
        })
    }
}

fn urls(count: usize) -> Vec<Claim> {
    (0..count)
        .map(|index| Claim {
            kind: ClaimKind::Url,
            target: format!("https://example.org/item-{index}"),
            source: {
                let mut sources = Sources::new();
                sources.add("main.tex", b"x".to_vec())
            },
            span: Span::new(0, 1),
            fields: Vec::new(),
        })
        .collect()
}

fn run_against(flaky: &Flaky, claims: &[Claim], previous: Option<&[u8]>) -> String {
    let mut persist = |_: &xtex_core::verification::VerificationRecord| {};
    let mut progress = |_: &str| {};
    let mut run = Run {
        transport: flaky,
        user_agent: "bench".to_owned(),
        now: "2026-09-01T00:00:00Z".to_owned(),
        max_age_days: 30,
        timeout: Duration::from_millis(500),
        persist: &mut persist,
        progress: &mut progress,
    };
    let (record, metrics) = verify(&mut run, claims, previous);
    eprintln!(
        "  measured · {} claims · {} calls · {} sim-ms · {}",
        claims.len(),
        flaky.calls.borrow(),
        flaky.simulated_ms.borrow(),
        metrics.summary()
    );
    render(&record)
}

#[test]
fn an_unstable_net_is_survived_bounded_and_the_next_run_completes_the_holes() {
    let claims = urls(40);

    // Run one: 40% of requests time out.
    let stormy = Flaky {
        fail_percent: 40,
        rng: RefCell::new(Lcg(7)),
        calls: RefCell::new(0),
        simulated_ms: RefCell::new(0),
        body: "ok",
    };
    let written = run_against(&stormy, &claims, None);
    let first = parse_record(written.as_bytes()).expect("a storm still writes a valid record");
    assert_eq!(
        first.claims.len(),
        40,
        "every claim settles, answered or not"
    );
    let holes = first
        .claims
        .iter()
        .filter(|c| c.failure_note.is_some())
        .count();
    assert!(
        holes > 0,
        "the seed produced no failures; the scenario is void"
    );
    // Bounded: the retry budget keeps total calls under 1.5x the claims
    // even at 40% failure — no storm of retries.
    assert!(
        *stormy.calls.borrow() <= 60,
        "retries stormed: {} calls for 40 claims",
        stormy.calls.borrow()
    );

    // Run two: the sky clears. Only the holes pay; everything answered is
    // carried over untouched.
    let clear = Flaky {
        fail_percent: 0,
        rng: RefCell::new(Lcg(7)),
        calls: RefCell::new(0),
        simulated_ms: RefCell::new(0),
        body: "ok",
    };
    let rewritten = run_against(&clear, &claims, Some(written.as_bytes()));
    let second = parse_record(rewritten.as_bytes()).expect("parses");
    assert_eq!(
        usize::try_from(*clear.calls.borrow()).unwrap_or(usize::MAX),
        holes,
        "the second run pays exactly the holes"
    );
    assert_eq!(
        second
            .claims
            .iter()
            .filter(|c| c.failure_note.is_some())
            .count(),
        0,
        "after the second run every hole is filled"
    );
}

#[test]
fn a_dead_net_costs_bounded_simulated_time() {
    // 100% failure: the budget must keep the run's simulated network time
    // proportional to the claims, not to hope.
    let dead = Flaky {
        fail_percent: 100,
        rng: RefCell::new(Lcg(3)),
        calls: RefCell::new(0),
        simulated_ms: RefCell::new(0),
        body: "ok",
    };
    let claims = urls(20);
    let written = run_against(&dead, &claims, None);
    let record = parse_record(written.as_bytes()).expect("valid");
    assert_eq!(record.claims.len(), 20);
    // Budget: ~1 attempt per claim once retries exhaust (global 10% cap),
    // each costing one timeout. Bound: claims + allowed retries + slack.
    assert!(
        *dead.calls.borrow() <= 26,
        "a dead net was hammered: {} calls",
        dead.calls.borrow()
    );
}
