//! The sources a claim is verified against, and how each is spoken to.
//!
//! Techniques and limits are the documented ones (see issue #137's research
//! notes): `OpenAlex` takes DOIs in batches of up to fifty through one OR
//! filter; `Crossref` answers DOI-less entries through search-based matching
//! (`query.bibliographic`), which its own evaluation prefers over
//! field-by-field; and arXiv is deliberately never spoken to — its terms
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
        let mut fields = vec![("title".to_owned(), candidate.to_owned())];
        if let Some(doi) = item.get("DOI").and_then(Value::text) {
            fields.push(("doi".to_owned(), doi.to_owned()));
        }
        if let Some(Value::List(people)) = item.get("author") {
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
        if let Some(year_value) = item
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
        return Ok(Some(Lookup {
            source: "crossref-query".to_owned(),
            fields,
            fingerprint: print,
            bytes: weight,
        }));
    }
    Ok(None)
}
