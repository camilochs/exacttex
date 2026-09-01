//! Field comparison: the document's entry against the source's record.
//!
//! Normalization is deliberately blunt for titles and years — lowercase,
//! alphanumerics, single spaces — because the diffs it produces are read by
//! a person, and a subtle matcher that silently forgives real differences
//! would be the fabricated-author-list failure all over again, wearing a
//! library.
//!
//! Author lists are the exception, and the reason is measured: on ten real
//! bibliographies (corpus E6, 2026-09-01) the blunt comparison flagged 229
//! author lists at high severity, of which about 43 differed in a family
//! name or a count. The rest were `Gr\'{e}goire` against `Grégoire`,
//! `Hoare, C. A. R.` against `C. A. R. Hoare`, `van de Wetering` split at
//! the wrong space, and lists the document cut with `and others`. A
//! diagnostic that is wrong four times in five trains its reader to
//! ignore the fifth — so a family-name comparison folds exactly those
//! shapes, and nothing else, before a diff is allowed to stand.

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

/// Latin letters with diacritics, folded to their ASCII base.
///
/// A table rather than a Unicode library: the verifier has one network
/// dependency and takes no other, and the letters that occur in author
/// names are the Latin-1 and Latin Extended-A blocks plus a few from
/// Extended-B and the Turkish dotless i. Combining marks (U+0300–U+036F)
/// are dropped, which folds a decomposed `é` the same way.
fn fold_char(character: char, out: &mut String) {
    const TABLE: &[(&str, &str)] = &[
        ("ÀÁÂÃÄÅĀĂĄǍ", "A"),
        ("àáâãäåāăąǎ", "a"),
        ("ÇĆĈĊČ", "C"),
        ("çćĉċč", "c"),
        ("ĎĐ", "D"),
        ("ďđð", "d"),
        ("ÈÉÊËĒĔĖĘĚ", "E"),
        ("èéêëēĕėęě", "e"),
        ("ĜĞĠĢ", "G"),
        ("ĝğġģ", "g"),
        ("ĤĦ", "H"),
        ("ĥħ", "h"),
        ("ÌÍÎÏĨĪĬĮİ", "I"),
        ("ìíîïĩīĭįı", "i"),
        ("Ĵ", "J"),
        ("ĵ", "j"),
        ("Ķ", "K"),
        ("ķ", "k"),
        ("ĹĻĽĿŁ", "L"),
        ("ĺļľŀł", "l"),
        ("ÑŃŅŇ", "N"),
        ("ñńņňŉ", "n"),
        ("ÒÓÔÕÖØŌŎŐǑ", "O"),
        ("òóôõöøōŏőǒ", "o"),
        ("ŔŖŘ", "R"),
        ("ŕŗř", "r"),
        ("ŚŜŞŠȘ", "S"),
        ("śŝşšș", "s"),
        ("ŢŤŦȚ", "T"),
        ("ţťŧț", "t"),
        ("ÙÚÛÜŨŪŬŮŰŲǓ", "U"),
        ("ùúûüũūŭůűųǔ", "u"),
        ("Ŵ", "W"),
        ("ŵ", "w"),
        ("ÝŶŸ", "Y"),
        ("ýÿŷ", "y"),
        ("ŹŻŽ", "Z"),
        ("źżž", "z"),
        ("Æ", "AE"),
        ("æ", "ae"),
        ("Œ", "OE"),
        ("œ", "oe"),
        ("ß", "ss"),
        ("Þ", "Th"),
        ("þ", "th"),
    ];
    if ('\u{300}'..='\u{36f}').contains(&character) {
        return;
    }
    for (from, to) in TABLE {
        if from.contains(character) {
            out.push_str(to);
            return;
        }
    }
    out.push(character);
}

/// TeX accent and letter macros folded to plain letters: `Gr\'{e}goire`
/// and `Gr{\'e}goire` both become `Gregoire`, `{\ae}` becomes `ae`,
/// `{\o}` becomes `o`. Every other control word is dropped.
fn fold_tex(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0usize;
    while at < chars.len() {
        let character = chars[at];
        if character != '\\' {
            if character != '{' && character != '}' {
                fold_char(character, &mut out);
            }
            at += 1;
            continue;
        }
        // A control symbol: one non-letter after the backslash. The accent
        // symbols modify the next letter, which is kept; `\-` and `\&`
        // and the rest are dropped.
        let Some(&next) = chars.get(at + 1) else {
            break;
        };
        if !next.is_ascii_alphabetic() {
            at += 2;
            continue;
        }
        let mut end = at + 1;
        while end < chars.len() && chars[end].is_ascii_alphabetic() {
            end += 1;
        }
        let word: String = chars[at + 1..end].iter().collect();
        let replacement = match word.as_str() {
            "i" => "i",
            "j" => "j",
            "ae" => "ae",
            "AE" => "AE",
            "oe" => "oe",
            "OE" => "OE",
            "o" => "o",
            "O" => "O",
            "l" => "l",
            "L" => "L",
            "aa" => "aa",
            "AA" => "AA",
            "ss" => "ss",
            "dh" | "dj" => "d",
            "DH" | "DJ" => "D",
            "th" => "th",
            "TH" => "Th",
            "ng" => "n",
            "NG" => "N",
            // Everything else is dropped: the accent commands that are
            // letters (`\c{c}`, `\v{s}`, `\H{o}`, `\u{a}`, `\k{e}`, `\d{a}`,
            // `\b{a}`, `\r{a}`, `\t{oo}`), whose modified letter follows and
            // is kept, and any control word that is not a letter at all.
            _ => "",
        };
        out.push_str(replacement);
        at = end;
    }
    out
}

/// One author's family name, as a comparison key: TeX folded, diacritics
/// folded, punctuation and case dropped, suffixes removed.
///
/// `Family, Given` takes everything before the first comma; `Given Family`
/// takes the last word and every lower-case particle before it (`van de
/// Wetering`). A braced group is one word (`{Le Scao}`, `{Ortiz Su\'arez}`).
fn family_key(author: &str) -> Option<String> {
    let folded = fold_tex(author)
        .replace(
            ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'],
            "-",
        )
        .replace(['\u{2019}', '\u{2018}'], "'");
    let family: String = if let Some((before, _)) = folded.split_once(',') {
        before.to_owned()
    } else {
        let words: Vec<&str> = folded.split_whitespace().collect();
        let mut end = words.len();
        while end > 0 && is_suffix(words[end - 1]) {
            end -= 1;
        }
        if end == 0 {
            return None;
        }
        let mut start = end - 1;
        while start > 0 && is_particle(words[start - 1]) {
            start -= 1;
        }
        words[start..end].join(" ")
    };
    let mut words: Vec<&str> = family.split_whitespace().collect();
    while words.last().is_some_and(|word| is_suffix(word)) {
        words.pop();
    }
    while words.first().is_some_and(|word| is_particle(word)) {
        words.remove(0);
    }
    let key: String = words
        .join("")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    (!key.is_empty()).then_some(key)
}

fn is_suffix(word: &str) -> bool {
    matches!(
        word.trim_matches('.').to_ascii_lowercase().as_str(),
        "jr" | "sr" | "ii" | "iii" | "iv"
    )
}

fn is_particle(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "van"
            | "von"
            | "de"
            | "der"
            | "den"
            | "del"
            | "della"
            | "di"
            | "da"
            | "du"
            | "des"
            | "le"
            | "la"
            | "of"
            | "y"
            | "ter"
            | "ten"
            | "af"
            | "zu"
    ) && word.chars().next().is_some_and(char::is_lowercase)
}

/// An author list read for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorList {
    /// Family-name keys, in the order written.
    pub names: Vec<String>,
    /// Whether the list said it was cut short: `and others`, `et al.`.
    pub truncated: bool,
}

/// Reads a BibTeX author field or a source's `and`-joined list.
#[must_use]
pub fn author_list(authors: &str) -> AuthorList {
    let mut names = Vec::new();
    let mut truncated = false;
    let collapsed: String = authors.split_whitespace().collect::<Vec<_>>().join(" ");
    for author in collapsed.split(" and ") {
        let author = author.trim();
        if author.is_empty() {
            continue;
        }
        let lowered = author.to_ascii_lowercase();
        if lowered == "others"
            || lowered == "et al"
            || lowered == "et al."
            || lowered.ends_with(" et al")
            || lowered.ends_with(" et al.")
            || lowered.ends_with(" e al.")
        {
            truncated = true;
            let head = author.rsplit_once(" e").map_or("", |(head, _)| head).trim();
            if lowered.starts_with("et al") || lowered == "others" || head.is_empty() {
                continue;
            }
            if let Some(key) = family_key(head) {
                names.push(key);
            }
            continue;
        }
        if let Some(key) = family_key(author) {
            names.push(key);
        }
    }
    AuthorList { names, truncated }
}

/// Family names from a BibTeX author field, normalized and sorted.
///
/// Kept for callers that want the keys; [`authors_agree`] is the
/// comparison.
#[must_use]
pub fn family_names(authors: &str) -> Vec<String> {
    let mut names = author_list(authors).names;
    names.sort_unstable();
    names
}

/// Registries cap the author lists they return; a document list longer
/// than this many names is compared only over the source's length.
const REGISTRY_CAP: usize = 50;

/// Whether two family-name keys name the same family.
///
/// Equal keys, or one a suffix of the other: `nggomez` (from `Aidan
/// N.Gomez`, the source's missing space) against `gomez`, and
/// `bianchiberthouze` against `berthouze` — a hyphenated double family
/// name against its second half. Four letters at least, so `li` inside
/// `bianchili` is not a match.
fn same_family(a: &str, b: &str) -> bool {
    a == b || (a.len() >= 4 && b.len() >= 4 && (a.ends_with(b) || b.ends_with(a)))
}

/// Whether two author lists agree once formatting is folded.
///
/// Family names only, as sets. A list that ends in `and others` must be
/// contained in the other; a source list at the registry cap is compared
/// against the same number of names from the document; otherwise the two
/// sets must match name for name.
#[must_use]
pub fn authors_agree(in_document: &str, in_source: &str) -> bool {
    let document = author_list(in_document);
    let source = author_list(in_source);
    if document.names.is_empty() && source.names.is_empty() {
        return true;
    }
    let (mut shorter, mut longer, prefix) = if document.truncated && !source.truncated {
        (document.names.clone(), source.names.clone(), false)
    } else if source.truncated && !document.truncated {
        (source.names.clone(), document.names.clone(), false)
    } else if source.names.len() >= REGISTRY_CAP && document.names.len() > source.names.len() {
        (source.names.clone(), document.names.clone(), true)
    } else if document.names.len() >= REGISTRY_CAP && source.names.len() > document.names.len() {
        (document.names.clone(), source.names.clone(), true)
    } else {
        if document.names.len() != source.names.len() {
            return false;
        }
        (document.names.clone(), source.names.clone(), false)
    };
    if prefix {
        longer.truncate(shorter.len());
    }
    // Every name of the shorter list finds its own partner in the longer.
    for name in shorter.drain(..) {
        let Some(index) = longer.iter().position(|other| same_family(&name, other)) else {
            return false;
        };
        longer.swap_remove(index);
    }
    true
}

/// One field's verdict: equal after normalization, or a diff carrying both
/// sides. Authors are always high severity — the fabricated author list
/// behind a valid DOI is exactly the failure verification exists to catch —
/// and a diff stands only if it survives the folding in [`authors_agree`].
#[must_use]
pub fn diff_field(name: &str, in_document: &str, in_source: &str) -> Option<FieldDiff> {
    let equal = if name == "author" || name == "authors" {
        authors_agree(in_document, in_source)
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

#[cfg(test)]
mod tests {
    //! Every case below is quoted from corpus E6's `out/diffs-all.csv`
    //! (2026-09-01): the document's author field against the registry's
    //! list, with the paper and key it came from.

    use super::*;

    #[test]
    fn tex_accents_and_diacritics_fold_to_the_same_key() {
        // 2412.19463 BBF21 / BGZ10 / Omer05 / Sab18; 2211.05100 OSCAR.
        for (document, source) in [
            (
                "Barbosa, Manuel and Barthe, Gilles and Fan, Xiong and Gr\\'{e}goire, Benjamin",
                "Manuel Barbosa and Gilles Barthe and Xiong Fan and Benjamin Grégoire",
            ),
            (
                "Barthe, Gilles\nand Gr{\\'e}goire, Benjamin\nand Zanella B{\\'e}guelin, Santiago",
                "Gilles Barthe and Benjamin Grégoire and Santiago Zanella Béguelin",
            ),
            ("{\\\"O}mer, Bernhard", "Bernhard Ömer"),
            (
                "Sabry, Amr\nand Valiron, Beno{\\^i}t\nand Vizzotto, Juliana Kaizer",
                "Amr Sabry and Benoît Valiron and Juliana Kaizer Vizzotto",
            ),
            (
                "Pedro Javier {Ortiz Su{\\'a}rez} and Beno{\\^\\i}t Sagot and Laurent Romary",
                "Pedro Ortiz Suárez and Benoît Sagot and Laurent Romary",
            ),
            (
                "Alejandro D{\\'{\\i}}az{-}Caro and\nMauricio Guillermo and\nAlexandre Miquel and\nBeno{\\^{\\i}}t Valiron",
                "Alejandro Diaz-Caro and Mauricio Guillermo and Alexandre Miquel and Benoit Valiron",
            ),
            // 2301.01148 park2019critical: `Kj{\ae}rgaard` against `Kjærgaard`.
            ("Kj{\\ae}rgaard, Mikkel Baun", "Mikkel Baun Kjærgaard"),
            // 2212.13570 Pelliccione2017: a braced family with `\AA`.
            ("{Magnus {\\AA}gren}, S.", "S. Magnus Ågren"),
            // 2304.01373 ghorbani2021scaling: the Turkish dotless i.
            ("Firat, Orhan", "Orhan Fırat"),
        ] {
            assert!(authors_agree(document, source), "{document} vs {source}");
        }
    }

    #[test]
    fn name_order_initials_particles_and_apostrophes_do_not_differ() {
        for (document, source) in [
            // 2412.19463 Hoare93: `Family, Initials` against `Initials Family`.
            (
                "Hoare, C. A. R.\nand Jifeng, He\nand Sampaio, A.",
                "C. A. R. Hoare and Jifeng, He and Augusto Sampaio",
            ),
            // 2412.19463 zx_extract / DKP20: particles kept together.
            (
                "de Beaudrap, Niel and Kissinger, Aleks and van de Wetering, John",
                "Niel de Beaudrap and Aleks Kissinger and John van de Wetering",
            ),
            // 2211.05100 scao2022what: a braced `{Le Scao}` against `Le Scao`.
            (
                "Teven {Le Scao} and Thomas Wang",
                "Teven Le Scao and Thomas Wang",
            ),
            // 2412.19463 Panan06: the typographic apostrophe.
            (
                "Ellie D'Hondt and\nPrakash Panangaden",
                "Ellie D’Hondt and Prakash Panangaden",
            ),
            // 2412.19463 Mathcomp: the SOURCE in `Family, Given`.
            (
                "Assia Mahboubi and\n Enrico Tassi",
                "Assia Mahboubi and Tassi, Enrico",
            ),
            // 2301.00152 content_sel_summ: a suffix.
            ("Daum{\\'e} III, Hal", "Hal Daumé"),
            // 2401.00908 su2023roformer: `Ahmed Murtadha` against the
            // source's `Murtadha, Ahmed` — the comma form is read as written.
            (
                "Ahmed Murtadha and Yunfeng Liu",
                "Murtadha, Ahmed and Yunfeng Liu",
            ),
            // 2412.19463 RPZ17: a middle initial the document lacks.
            (
                "Robert Rand and Jennifer Paykin",
                "Robert W. Rand and Jennifer Paykin",
            ),
            // 2412.19463 LZW19: tabs and line breaks between names.
            (
                "Liu, Junyi\n\tand Zhan, Bohua\n\tand Wang, Shuling",
                "Junyi Liu and Bohua Zhan and Shuling Wang",
            ),
        ] {
            assert!(authors_agree(document, source), "{document} vs {source}");
        }
    }

    #[test]
    fn hyphens_and_a_missing_space_at_the_source_are_folded() {
        // 2211.05100 alyafeai2021masader: `AlShaibani` against `Al-shaibani`.
        assert!(authors_agree(
            "Maged Saeed AlShaibani",
            "Maged S. Al-shaibani"
        ));
        // 2301.00304 Mathur_2020: a double family name against its half.
        assert!(authors_agree("Nadia Berthouze", "Nadia Bianchi‐Berthouze"));
        // 2211.05100 vaswani2017attention: `Aidan N.Gomez` at the source.
        assert!(authors_agree("Gomez, Aidan N", "Aidan N.Gomez"));
        // 2412.19463 rand2019reqwire: `{-}` and a Unicode hyphen.
        assert!(authors_agree("Dong{-}Ho Lee", "Dong‐Ho Lee"));
    }

    #[test]
    fn a_list_cut_with_others_is_a_prefix_not_a_difference() {
        // 2301.00304 Park_2020: `Daniel S. Park et al` against the full list.
        assert!(authors_agree(
            "Daniel S. Park et al",
            "Daniel S. Park and Yu Zhang and Ye Jia and Wei Han"
        ));
        assert!(authors_agree(
            "Vaswani, Ashish and Shazeer, Noam and others",
            "Ashish Vaswani and Noam Shazeer and Niki Parmar"
        ));
        // The cut list must still be contained: a wrong name in it differs.
        assert!(!authors_agree(
            "Vaswani, Ashish and Fabricated, X and others",
            "Ashish Vaswani and Noam Shazeer and Niki Parmar"
        ));
        // A source cut at the registry cap is compared over its own length.
        let fifty: Vec<String> = (0..50).map(|i| format!("Given Family{i}")).collect();
        let document = format!("{} and Given Extra", fifty.join(" and "));
        assert!(authors_agree(&document, &fifty.join(" and ")));
    }

    #[test]
    fn substantive_differences_survive_the_folding() {
        // 2304.01373 dai2021knowledge: the paper omits Baobao Chang.
        assert!(!authors_agree(
            "Dai, Damai and Dong, Li and Hao, Yaru and Sui, Zhifang and Wei, Furu",
            "Damai Dai and Dong Li and Yaru Hao and Zhifang Sui and Baobao Chang and Furu Wei"
        ));
        // 2211.05100 mann1947controlling: the wrong authors entirely.
        assert!(!authors_agree(
            "Mann, H and Whitney, D",
            "Yoav Benjamini and Yosef Hochberg"
        ));
        // 2301.00304 panayotov2015librispeech: one author of four, no `others`.
        assert!(!authors_agree(
            "Vassil Panayotov",
            "Vassil Panayotov and Guoguo Chen and Daniel Povey and Sanjeev Khudanpur"
        ));
        // 2401.00908 kenton2019bert: a mangled field.
        assert!(!authors_agree(
            "Kenton, Jacob Devlin Ming-Wei Chang and Toutanova, Lee Kristina",
            "Jacob Devlin and Ming-Wei Chang and Kenton Lee and Kristina Toutanova"
        ));
        // The canned fabrication the verifier tests use.
        assert!(!authors_agree(
            "Leslie Lamport",
            "Leslie Lamport and X. Fabricated"
        ));
        // Short keys never match by suffix: `Li` is not `Bianchili`.
        assert!(!authors_agree("Li, Wei", "Wei Bianchili"));
        // 2401.00908 su2023roformer, 2212.14404 xing_normalized_2015: a
        // name written `Bo Wen` in one place and `Wen Bo` in the other has
        // no comma to say which word is the family. That is reported, not
        // guessed — E6 classed it substantive too.
        assert!(!authors_agree("Bo Wen", "Wen Bo"));
        assert!(!authors_agree("Xing, Chao", "Xing Chao"));
    }

    #[test]
    fn the_diff_keeps_both_sides_verbatim_at_high_severity() {
        let diff = diff_field("author", "Mann, H and Whitney, D", "Yoav Benjamini").unwrap();
        assert_eq!(diff.field, "authors");
        assert_eq!(diff.in_document, "Mann, H and Whitney, D");
        assert_eq!(diff.severity, DiffSeverity::High);
        assert!(diff_field("author", "Gr\\'{e}goire, Benjamin", "Benjamin Grégoire").is_none());
        assert!(diff_field("title", "The {TeX}book", "The TeXbook").is_none());
    }

    #[test]
    fn family_names_are_the_sorted_keys() {
        assert_eq!(
            family_names("Zanella B{\\'e}guelin, Santiago and Barthe, Gilles"),
            ["barthe", "zanellabeguelin"]
        );
    }
}
