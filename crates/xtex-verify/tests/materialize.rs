//! The constructive half, against a canned source: every field in the
//! produced entry is the source's, the provenance comment is dated, and
//! a DOI the source does not know is a refusal — never an invention.

use std::cell::RefCell;
use std::time::Duration;

use xtex_verify::materialize::entry_from_doi;
use xtex_verify::transport::{Response, Transport, TransportError};

struct Canned {
    status: u16,
    body: &'static str,
    asked: RefCell<Vec<String>>,
}

impl Transport for Canned {
    fn get(&self, url: &str, _agent: &str, _timeout: Duration) -> Result<Response, TransportError> {
        self.asked.borrow_mut().push(url.to_owned());
        Ok(Response {
            status: self.status,
            body: self.body.as_bytes().to_vec(),
            location: None,
        })
    }
}

const WORK: &str = r#"{"message":{
  "type":"journal-article",
  "title":["A Very Exact Result"],
  "author":[{"given":"Ada","family":"Lovelace"},{"given":"Alan","family":"Turing"}],
  "container-title":["Annals of Certainty"],
  "issued":{"date-parts":[[2021,3]]},
  "volume":"7","page":"11-29",
  "DOI":"10.1000/exact"
}}"#;

#[test]
fn every_field_in_the_entry_is_the_sources() {
    let canned = Canned {
        status: 200,
        body: WORK,
        asked: RefCell::new(Vec::new()),
    };
    let entry = entry_from_doi(
        &canned,
        "test",
        Duration::from_secs(1),
        "10.1000/exact",
        None,
        "2026-09-01T12:00:00Z",
    )
    .expect("materializes");
    assert_eq!(entry.key, "lovelace2021");
    let expected = "% transcribed from crossref on 2026-09-01 — doi:10.1000/exact\n\
@article{lovelace2021,\n\
\x20 title = {A Very Exact Result},\n\
\x20 author = {Ada Lovelace and Alan Turing},\n\
\x20 journal = {Annals of Certainty},\n\
\x20 year = {2021},\n\
\x20 volume = {7},\n\
\x20 pages = {11-29},\n\
\x20 doi = {10.1000/exact}\n\
}";
    assert_eq!(entry.bibtex, expected);
    assert!(
        canned.asked.borrow()[0].starts_with("https://api.crossref.org/works/10.1000/exact"),
        "asked {:?}",
        canned.asked.borrow()
    );
}

#[test]
fn an_unknown_doi_is_a_refusal_never_an_invention() {
    let canned = Canned {
        status: 404,
        body: "{}",
        asked: RefCell::new(Vec::new()),
    };
    let refusal = entry_from_doi(
        &canned,
        "test",
        Duration::from_secs(1),
        "10.1000/ghost",
        None,
        "2026-09-01T12:00:00Z",
    )
    .expect_err("a 404 must refuse");
    assert!(refusal.contains("knows no work"), "{refusal}");
}

#[test]
fn a_record_without_a_title_is_refused() {
    let canned = Canned {
        status: 200,
        body: r#"{"message":{"type":"journal-article","author":[{"given":"A","family":"B"}]}}"#,
        asked: RefCell::new(Vec::new()),
    };
    let refusal = entry_from_doi(
        &canned,
        "test",
        Duration::from_secs(1),
        "10.1000/untitled",
        None,
        "2026-09-01T12:00:00Z",
    )
    .expect_err("no title, no entry");
    assert!(refusal.contains("refusing to invent"), "{refusal}");
}

#[test]
fn a_proceedings_paper_wears_inproceedings_and_a_chosen_key_wins() {
    let canned = Canned {
        status: 200,
        body: r#"{"message":{
          "type":"proceedings-article",
          "title":["Measured Doors"],
          "author":[{"given":"Grace","family":"Hopper"}],
          "container-title":["Proc. of Certainty"],
          "issued":{"date-parts":[[2019]]}
        }}"#,
        asked: RefCell::new(Vec::new()),
    };
    let entry = entry_from_doi(
        &canned,
        "test",
        Duration::from_secs(1),
        "10.1000/doors",
        Some("doors2019"),
        "2026-09-01T12:00:00Z",
    )
    .expect("materializes");
    assert_eq!(entry.key, "doors2019");
    assert!(
        entry.bibtex.contains("@inproceedings{doors2019,"),
        "{}",
        entry.bibtex
    );
    assert!(
        entry.bibtex.contains("booktitle = {Proc. of Certainty}"),
        "{}",
        entry.bibtex
    );
}
