//! Everything a document asserts about the world outside it.
//!
//! The deterministic half of external verification: no network, no verdicts
//! — just the inventory of claims, each with the span it was written at, so
//! any later finding lands where the author can act on it. Recognition is
//! bounded by the scanner's readable regions: a commented `\\url` is the
//! author's note and claims nothing, the same rule every reader follows.

use crate::io::SourceLoader;
use crate::source::{SourceId, Sources, Span};
use crate::verification::ClaimKind;

/// One assertion the document makes about the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// What kind of thing is claimed to exist.
    pub kind: ClaimKind,
    /// The claim's target: a citation key, an address, or a DOI.
    pub target: String,
    /// The file the claim was written in.
    pub source: SourceId,
    /// Where in that file.
    pub span: Span,
    /// For bibliography entries: the entry's fields (lowercased names,
    /// braces stripped) — the document's side of a verification diff.
    pub fields: Vec<(String, String)>,
}

/// Hosts whose addresses are source repositories rather than plain pages.
const FORGES: &[&str] = &[
    "github.com",
    "gitlab.com",
    "bitbucket.org",
    "codeberg.org",
    "sr.ht",
];

fn classify(address: &str) -> (ClaimKind, String) {
    if let Some(rest) = address
        .strip_prefix("https://doi.org/")
        .or_else(|| address.strip_prefix("http://doi.org/"))
        .or_else(|| address.strip_prefix("https://dx.doi.org/"))
    {
        return (ClaimKind::Doi, rest.to_owned());
    }
    let host = address
        .strip_prefix("https://")
        .or_else(|| address.strip_prefix("http://"))
        .unwrap_or(address)
        .split('/')
        .next()
        .unwrap_or("");
    if FORGES
        .iter()
        .any(|forge| host == *forge || host.ends_with(&format!(".{forge}")))
    {
        return (ClaimKind::Repository, address.to_owned());
    }
    (ClaimKind::Url, address.to_owned())
}

/// Collects the project's external claims: every declared bibliography
/// entry with its fields, and every `\\url{…}` / `\\href{…}{…}` target,
/// classified (a doi.org address is a DOI claim, a forge address is a
/// repository claim).
#[must_use]
pub fn collect(
    sources: &mut Sources,
    loader: &impl SourceLoader,
    root: SourceId,
    files: &[SourceId],
) -> Vec<Claim> {
    let mut claims = Vec::new();

    for &id in files {
        let Some(bytes) = sources.get(id).map(|s| s.bytes().to_vec()) else {
            continue;
        };
        for region in crate::scanner::readable_for(&bytes, &["url", "href"]) {
            let slice = &bytes[region.start()..region.end()];
            let Some(rest) = slice
                .strip_prefix(b"\\url{")
                .or_else(|| slice.strip_prefix(b"\\href{"))
            else {
                continue;
            };
            let Some(close) = rest.iter().position(|b| *b == b'}') else {
                continue;
            };
            let address = String::from_utf8_lossy(&rest[..close]).trim().to_owned();
            if address.is_empty() {
                continue;
            }
            let start = region.start() + (slice.len() - rest.len());
            #[allow(clippy::cast_possible_truncation)] // a source past 4GB is not a source
            let span = Span::new(start as u32, (start + close) as u32);
            let (kind, target) = classify(&address);
            claims.push(Claim {
                kind,
                target,
                source: id,
                span,
                fields: Vec::new(),
            });
        }
    }

    // The declared bibliographies: one claim per entry, fields attached.
    let declared = crate::bibliography::declared_in(sources, root);
    for resource in &declared.resources {
        let Ok(bib) = loader.load(&resource.name, Some(root), sources) else {
            continue;
        };
        let Some(bytes) = sources.get(bib).map(|s| s.bytes().to_vec()) else {
            continue;
        };
        let Some(keys) = crate::bibliography::keys_in_bib(&bytes) else {
            continue;
        };
        for key in keys {
            let Some(span) = crate::bibliography::entry_span_in_bib(&bytes, &key) else {
                continue;
            };
            let fields = crate::bibliography::entry_fields(&bytes, &key).unwrap_or_default();
            claims.push(Claim {
                kind: ClaimKind::BibEntry,
                target: key,
                source: bib,
                span,
                fields,
            });
        }
    }
    claims
}

/// Renders claims as JSON: kind, target, file, span, and any fields.
#[must_use]
pub fn to_json(sources: &Sources, claims: &[Claim]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("{\"claims\":[");
    for (index, claim) in claims.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"kind\":");
        let kind = match claim.kind {
            ClaimKind::BibEntry => "bib-entry",
            ClaimKind::Url => "url",
            ClaimKind::Doi => "doi",
            ClaimKind::Repository => "repository",
        };
        crate::json::write_text(kind, &mut out);
        out.push_str(",\"target\":");
        crate::json::write_text(&claim.target, &mut out);
        let file = sources.get(claim.source).map_or("", |s| s.name());
        out.push_str(",\"file\":");
        crate::json::write_text(file, &mut out);
        let _ = write!(
            out,
            ",\"offset\":{},\"length\":{}",
            claim.span.start(),
            claim.span.len()
        );
        out.push_str(",\"fields\":{");
        for (findex, (name, value)) in claim.fields.iter().enumerate() {
            if findex > 0 {
                out.push(',');
            }
            crate::json::write_text(name, &mut out);
            out.push(':');
            crate::json::write_text(value, &mut out);
        }
        out.push_str("}}");
    }
    out.push_str("]}");
    out
}
