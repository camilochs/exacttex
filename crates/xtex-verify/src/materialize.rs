//! The provider's constructive half: a bibliography entry TRANSCRIBED
//! from its DOI's authoritative source — never composed from memory. An
//! entry nobody typed cannot carry an invented field, and the dated
//! provenance comment it ships with says where every field came from.

use std::fmt::Write as _;
use std::time::Duration;

use xtex_core::json::{self, Value};

use crate::transport::{Transport, TransportError};

/// What materializing one DOI produced.
#[derive(Debug)]
pub struct Materialized {
    /// The suggested citation key (first author's family name + year).
    pub key: String,
    /// The BibTeX entry, provenance comment included.
    pub bibtex: String,
}

fn text_of(value: &Value, name: &str) -> Option<String> {
    match value.get(name)? {
        Value::List(items) => items.first().and_then(Value::text).map(str::to_owned),
        other => other.text().map(str::to_owned),
    }
}

fn year_of(value: &Value) -> Option<i64> {
    let parts = value.get("issued")?.get("date-parts")?;
    let Value::List(list) = parts else {
        return None;
    };
    let Some(Value::List(first)) = list.first() else {
        return None;
    };
    first.first().and_then(Value::integer)
}

fn authors_of(value: &Value) -> Vec<(String, String)> {
    let Some(Value::List(people)) = value.get("author") else {
        return Vec::new();
    };
    people
        .iter()
        .filter_map(|person| {
            let given = person.get("given").and_then(Value::text).unwrap_or("");
            let family = person.get("family").and_then(Value::text).unwrap_or("");
            if given.is_empty() && family.is_empty() {
                None
            } else {
                Some((given.to_owned(), family.to_owned()))
            }
        })
        .collect()
}

/// The BibTeX entry type for a Crossref work type — the common cases
/// named, everything else an honest `@misc`.
fn entry_type(crossref_type: &str) -> (&'static str, &'static str) {
    match crossref_type {
        "journal-article" => ("article", "journal"),
        "proceedings-article" => ("inproceedings", "booktitle"),
        "book" | "monograph" | "edited-book" | "reference-book" => ("book", "publisher"),
        "book-chapter" => ("incollection", "booktitle"),
        _ => ("misc", "howpublished"),
    }
}

/// Fetches the DOI's record from Crossref and transcribes it as BibTeX.
///
/// # Errors
///
/// A transport failure, a non-200 answer, or an answer with no title —
/// each as a message naming what refused. Nothing is ever filled in from
/// anywhere but the fetched record.
pub fn entry_from_doi(
    transport: &dyn Transport,
    user_agent: &str,
    timeout: Duration,
    doi: &str,
    key_override: Option<&str>,
    now: &str,
) -> Result<Materialized, String> {
    let encoded: String = doi
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'/' | b'(' | b')' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect();
    let url = format!("https://api.crossref.org/works/{encoded}");
    let response = transport
        .get(&url, user_agent, timeout)
        .map_err(|error: TransportError| error.to_string())?;
    if response.status == 404 {
        return Err(format!("crossref knows no work under doi {doi}"));
    }
    if response.status != 200 {
        return Err(format!("crossref answered {}", response.status));
    }
    let Some(value) = json::parse(&response.body) else {
        return Err("crossref answered non-JSON".to_owned());
    };
    let Some(work) = value.get("message") else {
        return Err("crossref answered without a message".to_owned());
    };

    let Some(title) = text_of(work, "title") else {
        return Err("the record carries no title; refusing to invent one".to_owned());
    };
    let authors = authors_of(work);
    let year = year_of(work);
    let kind = work.get("type").and_then(Value::text).unwrap_or("");
    let (bib_type, venue_field) = entry_type(kind);
    let venue = text_of(work, "container-title")
        .or_else(|| text_of(work, "publisher"))
        .unwrap_or_default();

    let key = key_override.map_or_else(
        || {
            let family = authors
                .first()
                .map_or("anon", |(_, family)| family.as_str());
            let normalized: String = family
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_lowercase();
            match year {
                Some(year) => format!("{normalized}{year}"),
                None => normalized,
            }
        },
        str::to_owned,
    );

    let mut bibtex = String::new();
    let day = now.get(..10).unwrap_or(now);
    let _ = writeln!(bibtex, "% transcribed from crossref on {day} — doi:{doi}");
    let _ = writeln!(bibtex, "@{bib_type}{{{key},");
    let _ = writeln!(bibtex, "  title = {{{title}}},");
    if !authors.is_empty() {
        let joined: Vec<String> = authors
            .iter()
            .map(|(given, family)| format!("{given} {family}").trim().to_owned())
            .collect();
        let _ = writeln!(bibtex, "  author = {{{}}},", joined.join(" and "));
    }
    if !venue.is_empty() {
        let _ = writeln!(bibtex, "  {venue_field} = {{{venue}}},");
    }
    if let Some(year) = year {
        let _ = writeln!(bibtex, "  year = {{{year}}},");
    }
    for (crossref_name, bib_name) in [("volume", "volume"), ("page", "pages")] {
        if let Some(found) = text_of(work, crossref_name)
            && !found.is_empty()
        {
            let _ = writeln!(bibtex, "  {bib_name} = {{{found}}},");
        }
    }
    let _ = writeln!(bibtex, "  doi = {{{doi}}}");
    bibtex.push('}');

    Ok(Materialized { key, bibtex })
}
