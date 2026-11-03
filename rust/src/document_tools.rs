//! Conservative document proposals. Recognized text is evidence, never an instruction.
use crate::{
    review,
    storage::{Control, Project, fingerprint},
    types::*,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRef {
    pub page: usize,
    /// Zero-based index in the current recognition/review snapshot.
    pub block_index: usize,
    pub source_image: String,
    pub region: Option<BoxRect>,
    pub excerpt: String,
    pub block_fingerprint: String,
}
fn source(page: &Page, index: usize) -> Result<SourceRef> {
    let b = &page.content.blocks[index];
    Ok(SourceRef {
        page: page.page,
        block_index: index,
        source_image: page.image.clone(),
        region: b.bbox.clone(),
        excerpt: b.plain().chars().take(240).collect(),
        block_fingerprint: fingerprint(b)?,
    })
}
fn analysis_page(project: &Project, number: usize, c: &Control) -> Result<Page> {
    crate::redactions::analysis_page(project, &project.load(number)?, c)
}
fn textual(b: &Block) -> bool {
    matches!(b.kind.as_str(), "text" | "heading" | "list")
}
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptHint {
    pub page: usize,
    pub scripts: BTreeMap<String, usize>,
    pub direction: String,
    pub state: String,
    pub language_override: Option<String>,
    pub note: String,
}
fn script(ch: char) -> Option<&'static str> {
    if !ch.is_alphabetic() {
        return None;
    }
    Some(match ch as u32 {
        0x0041..=0x005a | 0x0061..=0x007a | 0x00c0..=0x024f | 0x1e00..=0x1eff => "Latin",
        0x0590..=0x05ff | 0xfb1d..=0xfb4f => "Hebrew",
        0x0600..=0x06ff | 0x0750..=0x077f | 0x08a0..=0x08ff | 0xfb50..=0xfdff | 0xfe70..=0xfeff => {
            "Arabic"
        }
        0x0d80..=0x0dff => "Sinhala",
        0x0b80..=0x0bff => "Tamil",
        0x0900..=0x097f | 0xa8e0..=0xa8ff => "Devanagari",
        0x3040..=0x30ff
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xac00..=0xd7af
        | 0x20000..=0x2fa1f => "CJK",
        _ => "Other",
    })
}
/// A manual correction changes the OCR-language setting, independently of UI language.
pub fn language_override(settings: &Settings, languages: &str) -> Result<Settings> {
    let mut next = settings.clone();
    next.languages = languages.into();
    next.validate()?;
    Ok(next)
}
pub fn script_hints(
    project: &Project,
    override_languages: Option<&str>,
    c: &Control,
) -> Result<Vec<ScriptHint>> {
    if let Some(languages) = override_languages {
        language_override(&project.manifest.settings, languages)?;
    }
    let mut hints = vec![];
    for n in &project.manifest.pages {
        c.check()?;
        let page = analysis_page(project, *n, c)?;
        let mut counts = BTreeMap::new();
        for ch in page.content.plain().chars() {
            if let Some(name) = script(ch) {
                *counts.entry(name.to_string()).or_default() += 1;
            }
        }
        let rtl = counts.keys().any(|s| s == "Arabic" || s == "Hebrew");
        let ltr = counts.keys().any(|s| s != "Arabic" && s != "Hebrew");
        let state = if counts.is_empty() {
            "no_text"
        } else if counts.len() == 1 && counts.contains_key("Other") {
            "unknown"
        } else if counts.len() > 1 {
            "mixed_scripts"
        } else {
            "recognized_script"
        };
        hints.push(ScriptHint{page:page.page,direction:if rtl&&ltr{"mixed"}else if rtl{"rtl"}else if ltr{"ltr"}else{"unknown"}.into(),scripts:counts,state:state.into(),language_override:override_languages.map(str::to_owned),note:"Based only on the Unicode characters in recognized text. This does not detect a script from an image or identify a language; missing/incorrect recognition can give the wrong hint. Numerals do not establish reading direction. Choose OCR language codes manually when needed.".into()});
    }
    Ok(hints)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecurringProposal {
    pub id: String,
    pub kind: String,
    pub normalized_text: String,
    pub occurrences: Vec<SourceRef>,
    pub note: String,
}
pub fn recurring_elements(project: &Project, c: &Control) -> Result<Vec<RecurringProposal>> {
    let page_number = Regex::new(
        r"(?i)^\s*(?:page\s+)?(?:[-–]\s*)?\d{1,4}(?:\s*(?:/|of)\s*\d{1,4})?(?:\s*[-–])?\s*$",
    )?;
    let mut groups: BTreeMap<(String, String), Vec<SourceRef>> = BTreeMap::new();
    let mut count = 0;
    for n in &project.manifest.pages {
        c.check()?;
        let page = analysis_page(project, *n, c)?;
        for (index, b) in page.content.blocks.iter().enumerate() {
            if !textual(b) || b.text.trim().is_empty() || b.text.len() > 400 {
                continue;
            }
            let Some(region) = &b.bbox else {
                continue;
            };
            let zone = if region.bottom <= 0.15 {
                "header"
            } else if region.top >= 0.85 {
                "footer"
            } else {
                continue;
            };
            let number = page_number.is_match(&b.text);
            let key = if number {
                "<page-number>".into()
            } else {
                normalize(&b.text)
            };
            let kind = if number {
                format!("{zone}_page_number")
            } else {
                zone.into()
            };
            groups
                .entry((kind, key))
                .or_default()
                .push(source(&page, index)?);
            count += 1;
            anyhow::ensure!(
                count <= 50_000,
                Fault::new(
                    "NG400",
                    "Too many recurring-element candidates; split the project"
                )
            );
        }
    }
    let mut proposals = vec![];
    for ((kind, text), occurrences) in groups {
        if occurrences
            .iter()
            .map(|s| s.page)
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        {
            continue;
        }
        proposals.push(RecurringProposal{id:fingerprint(&(&kind,&text,&occurrences))?,kind,normalized_text:text,occurrences,note:"Repeated text in actual source boxes within the top/bottom 15% of at least two pages. This is a proposal, not a confirmed header/footer. Keep or remove only selected groups; removal is one undoable review revision.".into()});
    }
    Ok(proposals)
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecurringAction {
    Keep,
    Remove,
}
pub fn apply_recurring(
    project: &Project,
    proposal_ids: &[String],
    action: RecurringAction,
    c: &Control,
) -> Result<Option<u64>> {
    c.check()?;
    let wanted = proposal_ids.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        !wanted.is_empty() && wanted.len() == proposal_ids.len(),
        Fault::new("NG500", "Choose distinct recurring-element proposal IDs")
    );
    let proposals = recurring_elements(project, c)?;
    let chosen = proposals
        .iter()
        .filter(|p| wanted.contains(&p.id))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        chosen.len() == wanted.len(),
        Fault::new(
            "NG500",
            "Recurring-element preview is stale or a selected group is missing"
        )
    );
    if action == RecurringAction::Keep {
        return Ok(None);
    }
    let mut selected: BTreeMap<usize, BTreeMap<usize, String>> = BTreeMap::new();
    for p in chosen {
        for s in &p.occurrences {
            selected
                .entry(s.page)
                .or_default()
                .insert(s.block_index, s.block_fingerprint.clone());
        }
    }
    let mut changes = vec![];
    for (page, blocks) in selected {
        c.check()?;
        let before = project.load(page)?;
        for (index, expected) in &blocks {
            anyhow::ensure!(
                before
                    .content
                    .blocks
                    .get(*index)
                    .is_some_and(|b| fingerprint(b).is_ok_and(|s| &s == expected)),
                Fault::new(
                    "NG500",
                    "Source changed after the recurring-element preview"
                )
            );
        }
        let mut after = before.clone();
        after.content.blocks = after
            .content
            .blocks
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !blocks.contains_key(index))
            .map(|(_, b)| b)
            .collect();
        after.reviewed = false;
        changes.push((before, after));
    }
    c.check()?;
    Ok(Some(review::commit_pages(
        project,
        &changes,
        "Remove selected recurring headers/footers",
    )?))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceLink {
    pub marker: String,
    pub source: SourceRef,
    pub candidates: Vec<SourceRef>,
    pub status: String,
    pub note: String,
}
fn marker_key(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '0'..='9' => Some(c),
            '⁰' => Some('0'),
            '¹' => Some('1'),
            '²' => Some('2'),
            '³' => Some('3'),
            '⁴' => Some('4'),
            '⁵' => Some('5'),
            '⁶' => Some('6'),
            '⁷' => Some('7'),
            '⁸' => Some('8'),
            '⁹' => Some('9'),
            '*' | '†' | '‡' => Some(c),
            _ => None,
        })
        .collect()
}
pub fn reference_links(project: &Project, c: &Control) -> Result<Vec<ReferenceLink>> {
    let definition = Regex::new(r"^\s*(\[\d{1,4}\]|\d{1,4}[.)]|[⁰¹²³⁴⁵⁶⁷⁸⁹]+|[*†‡])\s+\S")?;
    let marker = Regex::new(r"\[\d{1,4}\]|[⁰¹²³⁴⁵⁶⁷⁸⁹]+|[*†‡]")?;
    let mut definitions: BTreeMap<String, Vec<SourceRef>> = BTreeMap::new();
    let mut mentions = vec![];
    let mut total = 0;
    for n in &project.manifest.pages {
        c.check()?;
        let page = analysis_page(project, *n, c)?;
        for (index, b) in page
            .content
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| textual(b))
        {
            let prefix = definition.captures(&b.text).and_then(|m| m.get(1));
            if let Some(prefix) = prefix {
                let candidates = definitions.entry(marker_key(prefix.as_str())).or_default();
                if candidates.len() < 8 {
                    candidates.push(source(&page, index)?);
                }
            }
            for found in marker.find_iter(&b.text) {
                if prefix.is_some_and(|p| found.start() >= p.start() && found.end() <= p.end()) {
                    continue;
                }
                mentions.push((marker_key(found.as_str()), source(&page, index)?));
                total += 1;
                anyhow::ensure!(
                    total <= 10_000,
                    Fault::new("NG400", "Too many citation markers; split the project")
                );
            }
        }
    }
    Ok(mentions.into_iter().map(|(marker,source)|{let candidates=definitions.get(&marker).cloned().unwrap_or_default();let status=match candidates.len(){0=>"unresolved",1=>"candidate",_=>"ambiguous"};ReferenceLink{marker,source,candidates,status:status.into(),note:"Matching recognized numeric/symbol markers only, retaining at most eight definition candidates per marker. Candidates may be footnotes, citations or unrelated numbering; no link is forced, and repeated definitions remain ambiguous. Author-year and semantic bibliography matching are not inferred.".into()}}).collect())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsistencySuggestion {
    pub kind: String,
    pub key: String,
    pub variants: BTreeMap<String, Vec<SourceRef>>,
    pub note: String,
}
fn add_variant(
    groups: &mut BTreeMap<(String, String), BTreeMap<String, Vec<SourceRef>>>,
    kind: &str,
    key: String,
    value: String,
    source: SourceRef,
) {
    let occurrences = groups
        .entry((kind.into(), key))
        .or_default()
        .entry(value)
        .or_default();
    if occurrences.len() < 20
        && !occurrences
            .iter()
            .any(|s| s.page == source.page && s.block_index == source.block_index)
    {
        occurrences.push(source);
    }
}
pub fn consistency_suggestions(
    project: &Project,
    c: &Control,
) -> Result<Vec<ConsistencySuggestion>> {
    let words = Regex::new(r"\b[A-Z][A-Za-z.']{2,29}\b")?;
    let units = Regex::new(r"(?i)\b([+-]?\d+(?:[.,]\d+)?)\s*(kg|mg|g|km|cm|mm|m|ml|l)\b")?;
    let identifiers =
        Regex::new(r"(?i)\b(ID|Ref|Invoice|Account)\s*[:#-]?\s*([A-Z0-9][A-Z0-9_-]{1,40})\b")?;
    let mut groups = BTreeMap::new();
    let mut names: BTreeMap<String, Vec<SourceRef>> = BTreeMap::new();
    let mut count = 0;
    for n in &project.manifest.pages {
        c.check()?;
        let page = analysis_page(project, *n, c)?;
        for (index, b) in page.content.blocks.iter().enumerate() {
            let text = b.plain();
            let origin = source(&page, index)?;
            for found in words.find_iter(&text) {
                let term = found.as_str().trim_end_matches('.');
                let key = term.replace('.', "").to_lowercase();
                add_variant(
                    &mut groups,
                    "capitalization_or_abbreviation",
                    key,
                    term.into(),
                    origin.clone(),
                );
                if term.chars().count() >= 4 && term.chars().skip(1).all(char::is_lowercase) {
                    let matches = names.entry(term.into()).or_default();
                    if matches.len() < 20
                        && !matches
                            .iter()
                            .any(|s| s.page == origin.page && s.block_index == origin.block_index)
                    {
                        matches.push(origin.clone());
                    }
                }
                count += 1;
            }
            for found in units.captures_iter(&text) {
                let unit = found[2].to_lowercase();
                let family = match unit.as_str() {
                    "kg" | "mg" | "g" => "mass",
                    "km" | "cm" | "mm" | "m" => "length",
                    _ => "volume",
                };
                add_variant(
                    &mut groups,
                    "unit_variation",
                    format!("{} {family}", &found[1]),
                    found[0].into(),
                    origin.clone(),
                );
                count += 1;
            }
            for found in identifiers.captures_iter(&text) {
                add_variant(
                    &mut groups,
                    "labelled_identifier_variation",
                    found[1].to_lowercase(),
                    found[2].into(),
                    origin.clone(),
                );
                count += 1;
            }
            anyhow::ensure!(
                count <= 50_000 && names.len() <= 2000,
                Fault::new(
                    "NG400",
                    "Consistency preview exceeds its bounded term budget; split the project"
                )
            );
        }
    }
    let mut suggestions = vec![];
    for ((kind, key), variants) in groups {
        if variants.len() > 1
            && variants
                .values()
                .flatten()
                .map(|s| s.page)
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        {
            suggestions.push(ConsistencySuggestion{kind,key,variants,note:"Recognized variants occur across pages. They may be intentional differences in meaning or formatting. Compare the linked source regions; no replacement is proposed or applied automatically.".into()});
        }
    }
    anyhow::ensure!(
        suggestions.len() <= 1000,
        Fault::new(
            "NG400",
            "Too many consistency suggestions; use a smaller project"
        )
    );
    let names = names.into_iter().collect::<Vec<_>>();
    for (index, (left, locations)) in names.iter().enumerate() {
        c.check()?;
        for (right, other) in names.iter().skip(index + 1) {
            if left.len().abs_diff(right.len()) > 1
                || locations
                    .iter()
                    .all(|a| other.iter().all(|b| a.page == b.page))
            {
                continue;
            }
            let score = strsim::normalized_levenshtein(left, right);
            if (0.8..1.0).contains(&score) {
                suggestions.push(ConsistencySuggestion{kind:"similar_capitalized_terms".into(),key:format!("{left} / {right}"),variants:BTreeMap::from([(left.clone(),locations.clone()),(right.clone(),other.clone())]),note:"Similar spellings of capitalized recognized terms; these are not confirmed names and may be different words. Review source regions without forcing consistency.".into()});
                anyhow::ensure!(
                    suggestions.len() <= 1000,
                    Fault::new(
                        "NG400",
                        "Too many consistency suggestions; use a smaller project"
                    )
                );
            }
        }
    }
    Ok(suggestions)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TechnicalIssue {
    pub code: String,
    pub source: SourceRef,
    pub message: String,
    pub reader_evidence: Vec<String>,
}
fn normalized_notation(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '⁰' | '₀' => '0',
            '¹' | '₁' => '1',
            '²' | '₂' => '2',
            '³' | '₃' => '3',
            '⁴' | '₄' => '4',
            '⁵' | '₅' => '5',
            '⁶' | '₆' => '6',
            '⁷' | '₇' => '7',
            '⁸' | '₈' => '8',
            '⁹' | '₉' => '9',
            _ => ch,
        })
        .filter(|c| !c.is_whitespace())
        .collect()
}
fn intersects(a: &BoxRect, b: &BoxRect) -> bool {
    (a.right.min(b.right) - a.left.max(b.left)) > 0.
        && (a.bottom.min(b.bottom) - a.top.max(b.top)) > 0.
}
type NotationEvidence = (BoxRect, String, String);
fn notation_candidates(
    page: &Page,
    c: &Control,
) -> Result<BTreeMap<String, Vec<NotationEvidence>>> {
    anyhow::ensure!(
        page.candidates.len() <= 1024,
        Fault::new("NG400", "Too many reader candidates for a notation preview")
    );
    let mut index: BTreeMap<String, Vec<NotationEvidence>> = BTreeMap::new();
    let mut count = 0usize;
    for candidate in &page.candidates {
        c.check()?;
        candidate.content.validate()?;
        count += candidate.content.blocks.len();
        anyhow::ensure!(
            count <= 10_000,
            Fault::new(
                "NG400",
                "Notation preview exceeds its reader-block budget; split the project"
            )
        );
        for block in &candidate.content.blocks {
            let Some(region) = &block.bbox else {
                continue;
            };
            let text = block.plain();
            if text.chars().any(|ch| "⁰¹²³⁴⁵⁶⁷⁸⁹₀₁₂₃₄₅₆₇₈₉".contains(ch))
            {
                index.entry(normalized_notation(&text)).or_default().push((
                    region.clone(),
                    text,
                    candidate.model_id.chars().take(100).collect(),
                ));
            }
        }
    }
    Ok(index)
}
pub fn technical_checks(project: &Project, c: &Control) -> Result<Vec<TechnicalIssue>> {
    let mut issues = vec![];
    let incomplete = Regex::new(r"[\^_]\s*(?:$|\{\s*\})")?;
    let chemical_index = Regex::new(
        r"(?:H|C|N|O|S|P|F|Cl|Na|K|Ca|Mg|Al|Fe|Cu|Zn|Ag|Br|I|Li|Mn|Cr|Co|Ni|Si)[_0-9₀-₉]",
    )?;
    for n in &project.manifest.pages {
        c.check()?;
        let page = analysis_page(project, *n, c)?;
        let candidates = notation_candidates(&page, c)?;
        for (index, b) in page.content.blocks.iter().enumerate() {
            c.check()?;
            let text = b.plain();
            let mut stack = vec![];
            let mut bad = false;
            let mut dollars = 0;
            let mut escape = false;
            for ch in text.chars() {
                if escape {
                    escape = false;
                    continue;
                }
                if ch == '\\' {
                    escape = true;
                    continue;
                }
                match ch {
                    '(' | '[' | '{' => stack.push(ch),
                    ')' | ']' | '}' => {
                        let expected = match ch {
                            ')' => '(',
                            ']' => '[',
                            _ => '{',
                        };
                        if stack.pop() != Some(expected) {
                            bad = true;
                        }
                    }
                    '$' => dollars += 1,
                    _ => {}
                }
            }
            if bad || !stack.is_empty() || dollars % 2 != 0 {
                issues.push(TechnicalIssue{code:"unbalanced_notation".into(),source:source(&page,index)?,message:"Unbalanced parentheses, brackets, braces or unescaped math delimiters. Compare with the scan; ordinary prose, chemistry and equations can use these marks differently.".into(),reader_evidence:vec![]});
            }
            if incomplete.is_match(&text) {
                issues.push(TechnicalIssue{code:"incomplete_script_marker".into(),source:source(&page,index)?,message:"A superscript/subscript marker appears empty. Review the source; no missing exponent, charge or chemical index is invented.".into(),reader_evidence:vec![]});
            }
            if b.kind == "equation"
                && text.len() <= 4096
                && chemical_index.is_match(&text)
                && (!text.contains('=') || text.contains("=>"))
            {
                let reaction = text.contains("->") || text.contains(['→', '⇌', '⇋']);
                let check = if reaction {
                    crate::chemistry::reaction(&text).and_then(|report| { anyhow::ensure!(report.balanced, Fault::new("NG400","Element counts or explicit ionic charges do not balance across this reaction"));Ok(()) })
                } else {
                    crate::chemistry::formula(&text).map(|_| ())
                };
                if let Err(error) = check {
                    issues.push(TechnicalIssue {code:"chemical_notation_review".into(),source:source(&page,index)?,message:format!("Possible chemical notation: {error}. Compare symbols, indices, coefficients and charges with the source. No text was changed."),reader_evidence:vec![]});
                }
            }
            if let Some(region) = &b.bbox {
                let normalized = normalized_notation(&text);
                let mut evidence = BTreeSet::new();
                for (candidate_region, alternative, reader) in
                    candidates.get(&normalized).into_iter().flatten()
                {
                    if alternative != &text && intersects(region, candidate_region) {
                        evidence.insert(reader.clone());
                    }
                }
                if !evidence.is_empty() {
                    issues.push(TechnicalIssue{code:"script_typography_disagreement".into(),source:source(&page,index)?,message:"An overlapping reader candidate retains superscript/subscript digits while the selected text uses different typography. Verify exponent/chemical-index placement against the scan; equality after digit normalization is not proof of correctness.".into(),reader_evidence:evidence.into_iter().collect()});
                }
            }
            anyhow::ensure!(
                issues.len() <= 5000,
                Fault::new(
                    "NG400",
                    "Too many technical notation issues; split the project"
                )
            );
        }
    }
    Ok(issues)
}
