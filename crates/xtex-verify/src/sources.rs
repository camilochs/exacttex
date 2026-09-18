//! The sources a claim is verified against, and how each is spoken to.
//!
//! Techniques and limits are the documented ones (see issue #137's research
//! notes): `OpenAlex` takes DOIs in batches of up to fifty through one OR
//! filter; `Crossref` answers DOI-less entries through search-based matching
//! (`query.bibliographic`), which its own evaluation prefers over
//! field-by-field, and answers one DOI at a time on its work route, which is
//! where a disagreement about an author list goes to be confirmed; and
//! arXiv is deliberately never spoken to — its terms
//! allow one request every three seconds, so anything it indexes is
//! resolved through the others.

use std::time::Duration;

use xtex_core::json::{self, Value};

use crate::transport::{Transport, TransportError};

/// What one source lookup produced for one claim.
pub struct Lookup {
    /// The answering source's name for the record.
    pub source: String,
    /// The source's fields, normalized names (title, authors, year, doi).
    pub fields: Vec<(String, String)>,
    /// Fingerprint of the raw response.
    pub fingerprint: String,
    /// Bytes the answer weighed — for the run's network metrics.
    pub bytes: usize,
}

/// A batch of DOI lookups against `OpenAlex`: up to fifty per request via
/// the OR filter, `select` shrinking the payload to what comparison needs.
///
/// # Errors
///
/// [`TransportError`] when the request itself failed; a DOI simply absent
/// from the answer is not an error — it is missing from the map.
pub fn openalex_by_doi(
    transport: &dyn Transport,
    user_agent: &str,
    timeout: Duration,
    dois: &[&str],
) -> Result<Vec<(String, Lookup)>, TransportError> {
    let filter = dois
        .iter()
        .map(|doi| doi.to_lowercase())
        .collect::<Vec<_>>()
        .join("|");
    let url = format!(
        "https://api.openalex.org/works?filter=doi:{filter}&per-page={}&select=doi,title,display_name,publication_year,authorships",
        dois.len().max(1)
    );
    let response = transport.get(&url, user_agent, timeout)?;
    if response.status != 200 {
        return Err(TransportError::Other(format!(
            "openalex answered {}",
            response.status
        )));
    }
    let print = crate::compare::fingerprint(&response.body);
    let weight = response.body.len();
    let Some(value) = json::parse(&response.body) else {
        return Err(TransportError::Other(
            "openalex answered non-JSON".to_owned(),
        ));
    };
    let mut found = Vec::new();
    if let Some(Value::List(results)) = value.get("results") {
        for work in results {
            let Some(doi_url) = work.get("doi").and_then(Value::text) else {
                continue;
            };
            let doi = doi_url
                .strip_prefix("https://doi.org/")
                .unwrap_or(doi_url)
                .to_lowercase();
            found.push((doi, lookup_from_openalex(work, &print, weight)));
        }
    }
    Ok(found)
}

fn lookup_from_openalex(work: &Value, print: &str, bytes: usize) -> Lookup {
    let mut fields = Vec::new();
    if let Some(title) = work
        .get("title")
        .and_then(Value::text)
        .or_else(|| work.get("display_name").and_then(Value::text))
    {
        fields.push(("title".to_owned(), title.to_owned()));
    }
    if let Some(year) = work.get("publication_year").and_then(Value::integer) {
        fields.push(("year".to_owned(), year.to_string()));
    }
    if let Some(Value::List(authorships)) = work.get("authorships") {
        let names: Vec<String> = authorships
            .iter()
            .filter_map(|a| a.get("author"))
            .filter_map(|a| a.get("display_name"))
            .filter_map(Value::text)
            .map(str::to_owned)
            .collect();
        if !names.is_empty() {
            fields.push(("authors".to_owned(), names.join(" and ")));
        }
    }
    Lookup {
        source: "openalex".to_owned(),
        fields,
        fingerprint: print.to_owned(),
        bytes,
    }
}

/// One DOI straight to Crossref's work route: the registry the publisher
/// deposited into, asked by identifier rather than by search.
///
/// This is the confirmation path, not the first question. `OpenAlex` takes
/// fifty DOIs per request and Crossref takes one, so asking here for every
/// entry would multiply a run's network cost; [`crate::run::verify`] calls
/// it only when the aggregator's answer disagreed about the author list,
/// which is the one field that can turn a correct entry into a hard error.
///
/// # Errors
///
/// [`TransportError`] when the request failed. `Ok(None)` when Crossref
/// answered without a usable work — a 404 is such an answer: the DOI is
/// simply not registered there, and an unconfirmable diff stands as it is.
pub fn crossref_by_doi(
    transport: &dyn Transport,
    user_agent: &str,
    timeout: Duration,
    doi: &str,
) -> Result<Option<Lookup>, TransportError> {
    let encoded: String = doi
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'/' | b'-' | b'_' | b'(' | b')')
            {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    let url = format!("https://api.crossref.org/works/{encoded}");
    let response = transport.get(&url, user_agent, timeout)?;
    if response.status != 200 {
        return Ok(None);
    }
    let print = crate::compare::fingerprint(&response.body);
    let weight = response.body.len();
    let Some(value) = json::parse(&response.body) else {
        return Ok(None);
    };
    let Some(work) = value.get("message") else {
        return Ok(None);
    };
    let lookup = lookup_from_crossref(work, &print, weight, "crossref");
    if lookup.fields.is_empty() {
        return Ok(None);
    }
    Ok(Some(lookup))
}

/// A DOI-less entry, matched the search-based way: the reference as one
/// string to Crossref's `query.bibliographic`, top row compared by
/// normalized title.
///
/// # Errors
///
/// [`TransportError`] when the request failed; `Ok(None)` when Crossref
/// answered but nothing matched by title.
pub fn crossref_by_query(
    transport: &dyn Transport,
    user_agent: &str,
    timeout: Duration,
    title: &str,
    authors: &str,
    year: &str,
) -> Result<Option<Lookup>, TransportError> {
    let query = format!("{title} {authors} {year}");
    let encoded: String = query
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_string()
            } else {
                "+".to_owned()
            }
        })
        .collect();
    let url = format!(
        "https://api.crossref.org/works?query.bibliographic={encoded}&rows=2&select=DOI,title,author,issued"
    );
    let response = transport.get(&url, user_agent, timeout)?;
    if response.status != 200 {
        return Err(TransportError::Other(format!(
            "crossref answered {}",
            response.status
        )));
    }
    let print = crate::compare::fingerprint(&response.body);
    let weight = response.body.len();
    let Some(value) = json::parse(&response.body) else {
        return Err(TransportError::Other(
            "crossref answered non-JSON".to_owned(),
        ));
    };
    let items = value.get("message").and_then(|m| m.get("items")).cloned();
    let Some(Value::List(items)) = items else {
        return Ok(None);
    };
    for item in &items {
        let Some(Value::List(titles)) = item.get("title") else {
            continue;
        };
        let Some(candidate) = titles.first().and_then(Value::text) else {
            continue;
        };
        if crate::compare::normalize(candidate) != crate::compare::normalize(title) {
            continue;
        }
        return Ok(Some(lookup_from_crossref(
            item,
            &print,
            weight,
            "crossref-query",
        )));
    }
    Ok(None)
}

/// One Crossref work object as a [`Lookup`]. Shared by the two routes that
/// answer with one: the search above and [`crossref_by_doi`], which read
/// the same fields out of the same shape.
fn lookup_from_crossref(work: &Value, print: &str, bytes: usize, source: &str) -> Lookup {
    let mut fields = Vec::new();
    if let Some(Value::List(titles)) = work.get("title")
        && let Some(title) = titles.first().and_then(Value::text)
    {
        fields.push(("title".to_owned(), title.to_owned()));
    }
    if let Some(doi) = work.get("DOI").and_then(Value::text) {
        fields.push(("doi".to_owned(), doi.to_owned()));
    }
    if let Some(Value::List(people)) = work.get("author") {
        let names: Vec<String> = people
            .iter()
            .map(|person| {
                let given = person.get("given").and_then(Value::text).unwrap_or("");
                let family = person.get("family").and_then(Value::text).unwrap_or("");
                format!("{given} {family}").trim().to_owned()
            })
            .filter(|name| !name.is_empty())
            .collect();
        if !names.is_empty() {
            fields.push(("authors".to_owned(), names.join(" and ")));
        }
    }
    if let Some(year_value) = work
        .get("issued")
        .and_then(|issued| issued.get("date-parts"))
        .and_then(|parts| match parts {
            Value::List(list) => list.first().cloned(),
            _ => None,
        })
        .and_then(|first| match first {
            Value::List(list) => list.first().and_then(Value::integer),
            _ => None,
        })
    {
        fields.push(("year".to_owned(), year_value.to_string()));
    }
    Lookup {
        source: source.to_owned(),
        fields,
        fingerprint: print.to_owned(),
        bytes,
    }
}
