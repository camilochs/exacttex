//! The document's external claims, inventoried — deterministically, with
//! the span each claim was written at, and nothing from a comment.

use xtex_core::claims::{Claim, collect};
use xtex_core::io::{Memory, SourceLoader};
use xtex_core::verification::ClaimKind;

const ROOT: &str = "\\documentclass{article}\n\\begin{document}\n\
Data at \\url{https://example.org/data}.\n\
% \\url{https://commented.example.org} — the author's note, not a claim\n\
Code at \\href{https://github.com/knuth/tex}{the repository}.\n\
Paper at \\url{https://doi.org/10.1145/3-540}.\n\
\\input{chapter}\n\
\\bibliography{refs}\n\\end{document}\n";

const CHAPTER: &str = "More at \\url{https://example.org/appendix}.\n";

const BIB: &str = "@book{knuth1984,\n  title = {The {TeX}book},\n  author = \"Donald E. Knuth\",\n  year = 1984,\n  doi = {10.5555/1096283}\n}\n";

fn inventory() -> (xtex_core::source::Sources, Vec<Claim>) {
    let memory = Memory::new()
        .with_input("main.tex", ROOT.as_bytes().to_vec())
        .with_input("chapter.tex", CHAPTER.as_bytes().to_vec())
        .with_input("refs.bib", BIB.as_bytes().to_vec());
    let mut sources = xtex_core::source::Sources::new();
    let root = memory.load("main.tex", None, &mut sources).expect("root");
    let chapter = memory
        .load("chapter.tex", Some(root), &mut sources)
        .expect("chapter");
    let claims = collect(&mut sources, &memory, root, &[root, chapter]);
    (sources, claims)
}

fn by_kind(claims: &[Claim], kind: ClaimKind) -> Vec<&Claim> {
    claims.iter().filter(|c| c.kind == kind).collect()
}

#[test]
fn every_kind_is_inventoried_and_classified() {
    let (_, claims) = inventory();
    let urls = by_kind(&claims, ClaimKind::Url);
    assert_eq!(urls.len(), 2, "{claims:?}");
    assert!(urls.iter().any(|c| c.target == "https://example.org/data"));
    assert!(
        urls.iter()
            .any(|c| c.target == "https://example.org/appendix"),
        "the included chapter's claim counts"
    );
    let repos = by_kind(&claims, ClaimKind::Repository);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].target, "https://github.com/knuth/tex");
    let dois = by_kind(&claims, ClaimKind::Doi);
    assert_eq!(dois.len(), 1);
    assert_eq!(
        dois[0].target, "10.1145/3-540",
        "the doi.org address claims the DOI itself"
    );
}

#[test]
fn a_commented_url_claims_nothing() {
    let (_, claims) = inventory();
    assert!(
        claims.iter().all(|c| !c.target.contains("commented")),
        "{claims:?}"
    );
}

#[test]
fn a_bib_entry_arrives_with_its_fields() {
    let (_, claims) = inventory();
    let entries = by_kind(&claims, ClaimKind::BibEntry);
    assert_eq!(entries.len(), 1);
    let entry = entries[0];
    assert_eq!(entry.target, "knuth1984");
    let field = |name: &str| {
        entry
            .fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(field("title"), Some("The {TeX}book"));
    assert_eq!(
        field("author"),
        Some("Donald E. Knuth"),
        "quoted values read"
    );
    assert_eq!(field("year"), Some("1984"), "bare values read");
    assert_eq!(field("doi"), Some("10.5555/1096283"));
}

#[test]
fn every_span_lands_on_the_written_claim() {
    let (sources, claims) = inventory();
    for claim in &claims {
        let bytes = sources.get(claim.source).expect("source").bytes();
        let written = &bytes[claim.span.start()..claim.span.end()];
        let text = String::from_utf8_lossy(written);
        match claim.kind {
            ClaimKind::BibEntry => assert_eq!(text, claim.target, "the key's own token"),
            ClaimKind::Doi => assert!(text.contains(&claim.target), "{text}"),
            _ => assert_eq!(text, claim.target, "the address as written"),
        }
    }
}
