//! Field comparison: the document's entry against the source's record.
//!
//! Normalization is deliberately blunt — lowercase, alphanumerics, single
//! spaces — because the diffs it produces are read by a person, and a
//! subtle matcher that silently forgives real differences would be the
//! fabricated-author-list failure all over again, wearing a library.

use xtex_core::verification::{DiffSeverity, FieldDiff};

/// Lowercased alphanumerics with single spaces; everything else folds.
#[must_use]
pub fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut space = true;
    for character in text.chars() {
        // Braces are BibTeX grouping, not separation: `The {TeX}book`
        // is one word, and a space here would split it.
        if character == '{' || character == '}' {
            continue;
        }
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                out.push(lower);
            }
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim().to_owned()
}

/// Family names from a BibTeX author field (`and`-separated, either
/// `Family, Given` or `Given Family`), normalized and sorted.
#[must_use]
pub fn family_names(authors: &str) -> Vec<String> {
    let mut names: Vec<String> = authors
        .split(" and ")
        .map(|author| {
            let author = author.trim();
            let family = author
                .split_once(',')
                .map_or_else(|| author.rsplit(' ').next().unwrap_or(author), |(f, _)| f);
            normalize(family)
        })
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_unstable();
    names
}

/// One field's verdict: equal after normalization, or a diff carrying both
/// sides. Authors are always high severity — the fabricated author list
/// behind a valid DOI is exactly the failure verification exists to catch.
#[must_use]
pub fn diff_field(name: &str, in_document: &str, in_source: &str) -> Option<FieldDiff> {
    let equal = if name == "author" || name == "authors" {
        family_names(in_document) == family_names(in_source)
    } else {
        normalize(in_document) == normalize(in_source)
    };
    if equal {
        return None;
    }
    Some(FieldDiff {
        field: if name == "author" {
            "authors".to_owned()
        } else {
            name.to_owned()
        },
        in_document: in_document.to_owned(),
        in_source: in_source.to_owned(),
        severity: if name == "author" || name == "authors" {
            DiffSeverity::High
        } else {
            DiffSeverity::Medium
        },
    })
}

/// FNV-1a, 64-bit, hex: a deterministic fingerprint for a response body.
/// Not cryptographic and not pretending to be — later runs only ask
/// "did the answer change", never "is the answer authentic".
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}
