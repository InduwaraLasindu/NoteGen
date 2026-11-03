//! Image-based script suggestions and source-bound, explicitly corrected routes.
//! OSD confidence scores are native scores, never calibrated probabilities.
use crate::{
    geometry,
    redactions::SourceIdentity,
    storage::{Control, Project, fingerprint},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ScriptObservation {
    pub script: Option<String>,
    pub script_confidence: Option<f32>,
    pub orientation_degrees: Option<u16>,
    pub orientation_confidence: Option<f32>,
    pub rotate_degrees: u16,
    pub suggested_languages: Vec<String>,
    pub note: String,
}
impl ScriptObservation {
    pub fn unavailable() -> Self {
        Self{note:"Image-based script detection unavailable or too few characters. Keep the selected OCR languages or correct them manually. No language was changed.".into(),..Default::default()}
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.script.as_ref().is_none_or(|s| !s.is_empty()
                && s.len() <= 64
                && s.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ' ')))
                && self
                    .script_confidence
                    .is_none_or(|v| v.is_finite() && (0.0..=1_000_000.0).contains(&v))
                && self
                    .orientation_confidence
                    .is_none_or(|v| v.is_finite() && (0.0..=1_000_000.0).contains(&v))
                && self
                    .orientation_degrees
                    .is_none_or(|v| [0, 90, 180, 270].contains(&v))
                && [0, 90, 180, 270].contains(&self.rotate_degrees)
                && self.suggested_languages.len() <= 5
                && self.note.len() <= 4000,
            fail("Invalid script observation")
        );
        if !self.suggested_languages.is_empty() {
            crate::languages::selection(&self.suggested_languages)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoundScriptObservation {
    pub source: SourceIdentity,
    pub observation: ScriptObservation,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegionLayout {
    Paragraph,
    Line,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BlockTarget {
    pub fingerprint: String,
    pub index_hint: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RegionLanguage {
    pub id: String,
    pub source: SourceIdentity,
    pub region: BoxRect,
    pub languages: String,
    pub layout: RegionLayout,
    /// A matching whole block may be replaced; arbitrary/cross-block regions
    /// contribute alternatives rather than silently dropping unselected content.
    pub target: Option<BlockTarget>,
    pub note: String,
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG740", message).into()
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub fn validate_page(page: &Page) -> Result<()> {
    if let Some(detection) = &page.script_detection {
        geometry::validate_source(&detection.source)?;
        detection.observation.validate()?;
        anyhow::ensure!(
            detection.source.page == page.page,
            fail("Script detection belongs to a different source page")
        );
    }
    anyhow::ensure!(
        page.region_languages.len() <= 64,
        fail("Too many explicit region language selections")
    );
    let mut ids = BTreeSet::new();
    for selection in &page.region_languages {
        geometry::validate_source(&selection.source)?;
        let codes = selection
            .languages
            .split('+')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        crate::languages::selection(&codes)?;
        anyhow::ensure!(
            selection.source.page == page.page
                && selection.region.valid()
                && hash(&selection.id)
                && ids.insert(&selection.id)
                && selection.note.len() <= 2000
                && selection
                    .target
                    .as_ref()
                    .is_none_or(|target| hash(&target.fingerprint) && target.index_hint < 500),
            fail("Invalid source-bound region language selection")
        );
    }
    Ok(())
}
/// Parse only bounded native OSD fields. Missing scores remain missing.
pub fn parse_osd(text: &str) -> Result<ScriptObservation> {
    anyhow::ensure!(text.len() <= 16000, fail("OSD output exceeds its budget"));
    let mut result = ScriptObservation::unavailable();
    let mut seen = BTreeSet::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if ![
            "Script",
            "Script confidence",
            "Orientation in degrees",
            "Orientation confidence",
            "Rotate",
        ]
        .contains(&key)
        {
            continue;
        }
        anyhow::ensure!(seen.insert(key), fail("Duplicate OSD field"));
        match key {
            "Script" => result.script = Some(value.into()),
            "Script confidence" => {
                result.script_confidence = Some(
                    value
                        .parse()
                        .map_err(|_| fail("Invalid OSD script score"))?,
                )
            }
            "Orientation confidence" => {
                result.orientation_confidence = Some(
                    value
                        .parse()
                        .map_err(|_| fail("Invalid OSD orientation score"))?,
                )
            }
            "Orientation in degrees" => {
                result.orientation_degrees =
                    Some(value.parse().map_err(|_| fail("Invalid OSD orientation"))?)
            }
            "Rotate" => {
                result.rotate_degrees = value.parse().map_err(|_| fail("Invalid OSD rotation"))?
            }
            _ => {}
        }
    }
    if let Some(script) = &result.script {
        result.suggested_languages = match script.as_str() {
            "Latin" => vec!["eng".into()],
            "Sinhala" => vec!["sin".into()],
            "Tamil" => vec!["tam".into()],
            "Arabic" => vec!["ara".into()],
            "Hebrew" => vec!["heb".into()],
            _ => vec![],
        };
        result.note="Image-based Tesseract OSD suggestion. Native scores are not probabilities or measured accuracy. Script does not uniquely identify a language; Latin includes many languages, with English only the available catalog suggestion. Short/mixed/handwritten regions may be wrong. No selection is changed automatically.".into();
    }
    result.validate()?;
    Ok(result)
}
pub fn bind_detection(
    source: &SourceIdentity,
    observation: ScriptObservation,
) -> Result<BoundScriptObservation> {
    geometry::validate_source(source)?;
    observation.validate()?;
    Ok(BoundScriptObservation {
        source: source.clone(),
        observation,
    })
}
pub fn select_region(
    project: &Project,
    expected: &Page,
    region: BoxRect,
    languages: &str,
    layout: RegionLayout,
    target: Option<usize>,
    c: &Control,
) -> Result<RegionLanguage> {
    anyhow::ensure!(region.valid(), fail("Choose valid page source bounds"));
    let codes = languages.split('+').map(str::to_owned).collect::<Vec<_>>();
    let languages = crate::languages::selection(&codes)?;
    let current = project.load(expected.page)?;
    anyhow::ensure!(
        fingerprint(&current)? == fingerprint(expected)?,
        fail("Page changed before language selection")
    );
    let (source, _) = crate::redactions::recognition_view(project, expected, c)?;
    let target = target
        .map(|index| -> Result<BlockTarget> {
            let block = expected
                .content
                .blocks
                .get(index)
                .ok_or_else(|| fail("Selected block no longer exists"))?;
            anyhow::ensure!(
                block.bbox.as_ref() == Some(&region),
                fail("Whole-block language replacement requires its exact source bounds")
            );
            Ok(BlockTarget {
                fingerprint: fingerprint(block)?,
                index_hint: index,
            })
        })
        .transpose()?;
    let id = fingerprint(&(1, &source, &region, &languages, &layout, &target))?;
    Ok(RegionLanguage{id,source,region,languages,layout,target,note:"Explicit source-bound OCR language selection; it is background routing, not transcription evidence.".into()})
}
pub fn commit_selections(
    project: &Project,
    expected: &Page,
    selections: Vec<RegionLanguage>,
    c: &Control,
) -> Result<u64> {
    let (source, _) = crate::redactions::recognition_view(project, expected, c)?;
    anyhow::ensure!(
        selections
            .iter()
            .all(|selection| selection.source == source),
        fail("Language selections refer to changed source pixels")
    );
    let mut next = expected.clone();
    next.region_languages = selections;
    validate_page(&next)?;
    c.check()?;
    crate::review::commit(project, expected, &next, "Correct region OCR languages")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Suggestion {
    pub block: usize,
    pub region: BoxRect,
    pub detection: ScriptObservation,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingSuggestions {
    pub source: SourceIdentity,
    pub page_fingerprint: String,
    pub regions: Vec<Suggestion>,
    pub unprobed: usize,
    pub note: String,
}
/// Explicit bounded paragraph inspection, before its selected-language OCR pass.
/// No downloads, settings changes, checkpoint writes or script-to-language claims.
pub fn suggest_regions(
    project: &Project,
    expected: &Page,
    settings: &Settings,
    max_regions: usize,
    c: &Control,
) -> Result<RoutingSuggestions> {
    use std::time::{Duration, Instant};
    anyhow::ensure!(
        (1..=16).contains(&max_regions),
        fail("Inspect 1–16 paragraph regions at a time")
    );
    anyhow::ensure!(
        fingerprint(&project.load(expected.page)?)? == fingerprint(expected)?,
        fail("Page changed before script inspection")
    );
    let (source, image) = crate::redactions::recognition_view(project, expected, c)?;
    let safe = crate::redactions::recognition_settings(settings, &expected.confirmed_redactions)?;
    let eligible = expected
        .content
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            !matches!(block.kind.as_str(), "figure" | "table") && block.bbox.is_some()
        })
        .collect::<Vec<_>>();
    let start = Instant::now();
    let mut regions = vec![];
    for (index, block) in eligible.iter().take(max_regions) {
        c.check()?;
        let Some(remaining) = Duration::from_secs(60).checked_sub(start.elapsed()) else {
            break;
        };
        if remaining < Duration::from_millis(100) {
            break;
        }
        let requested = block.bbox.as_ref().expect("eligible source box");
        let (l, t, r, b) = requested.pixels(image.width(), image.height());
        let region = requested.clone();
        let crop = image.crop_imm(l, t, r - l, b - t);
        let detection = crate::scan::detect_script_with_timeout(
            &crop,
            &safe,
            c,
            remaining.min(Duration::from_secs(20)),
        )?;
        regions.push(Suggestion{block:*index,region,detection,note:"Correct the selected OCR language(s) before routing. Multiple languages can be combined within this paragraph; the script suggestion alone cannot identify a language or prove the resource is installed.".into()});
    }
    c.check()?;
    let fresh = geometry::source_identity(&project.image(expected.page), expected.page, c)?;
    anyhow::ensure!(
        fresh == source,
        fail("Source changed during script inspection")
    );
    let unprobed = eligible.len() - regions.len();
    Ok(RoutingSuggestions{source,page_fingerprint:fingerprint(expected)?,regions,unprobed,note:"At most 16 regions / 60 seconds. OSD runs on already masked pixels, never rotates the immutable source and may be unavailable for short/handwritten samples. Unprobed regions keep existing routing. Manual combined languages support mixed writing within a paragraph.".into()})
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RereadPreview {
    pub source: SourceIdentity,
    pub before_fingerprint: String,
    pub after: Page,
    pub token: String,
    pub note: String,
}
fn preview_token(source: &SourceIdentity, before: &str, after: &Page) -> Result<String> {
    fingerprint(&(1, source, before, after))
}
/// Serial native OCR for explicit, source-bound selections. Every crop is taken
/// from the masked full page, and all word/cell/line coordinates return to page space.
pub fn reread_preview(
    project: &Project,
    expected: &Page,
    selections: &[RegionLanguage],
    settings: &Settings,
    c: &Control,
) -> Result<RereadPreview> {
    use std::collections::BTreeMap;
    anyhow::ensure!(
        !selections.is_empty()
            && selections.len() <= 16
            && expected.candidates.len() + selections.len() <= 1024,
        fail("Choose 1–16 routes within the reader evidence budget")
    );
    let before_fingerprint = fingerprint(expected)?;
    anyhow::ensure!(
        fingerprint(&project.load(expected.page)?)? == before_fingerprint,
        fail("Page changed before regional OCR")
    );
    let (source, image) = crate::redactions::recognition_view(project, expected, c)?;
    let mut next = expected.clone();
    let mut replacements = BTreeMap::<usize, Vec<Block>>::new();
    let mut ids = BTreeSet::new();
    let safe = crate::redactions::recognition_settings(settings, &expected.confirmed_redactions)?;
    for selection in selections {
        c.check()?;
        anyhow::ensure!(
            selection.source == source && selection.region.valid() && ids.insert(&selection.id),
            fail("Route source changed, is invalid or was selected twice")
        );
        let mut validated = expected.clone();
        validated.region_languages = vec![selection.clone()];
        validate_page(&validated)?;
        let mut selected = safe.clone();
        selected.languages = selection.languages.clone();
        let (l, t, r, b) = selection.region.pixels(image.width(), image.height());
        let view = BoxRect {
            left: l as f32 / image.width() as f32,
            top: t as f32 / image.height() as f32,
            right: r as f32 / image.width() as f32,
            bottom: b as f32 / image.height() as f32,
        };
        let crop = image.crop_imm(l, t, r - l, b - t);
        let layout = match selection.layout {
            RegionLayout::Paragraph => crate::scan::PrintedLayout::Paragraph,
            RegionLayout::Line => crate::scan::PrintedLayout::Line,
        };
        let mut content = crate::scan::printed_with_layout(&crop, &selected, c, layout)?;
        if !expected.confirmed_redactions.is_empty() {
            content = crate::redactions::enforce_view(
                &content,
                &source,
                &expected.confirmed_redactions,
                Some(&view),
                c,
            )?
            .0;
        }
        geometry::map_content(&mut content, &view, &source)?;
        if !expected.confirmed_redactions.is_empty() {
            content = crate::redactions::enforce_view(
                &content,
                &source,
                &expected.confirmed_redactions,
                None,
                c,
            )?
            .0;
            // Whole-page enforcement also creates placeholders for covers outside
            // this crop. A regional candidate only claims its actual source view.
            content.blocks.retain_mut(|block| {
                let Some(bounds) = &block.bbox else {
                    return false;
                };
                let clipped = BoxRect {
                    left: bounds.left.max(view.left),
                    top: bounds.top.max(view.top),
                    right: bounds.right.min(view.right),
                    bottom: bounds.bottom.min(view.bottom),
                };
                if !clipped.valid() {
                    return false;
                }
                block.bbox = Some(clipped);
                true
            });
        }
        if let Some(target) = &selection.target {
            let index = if expected
                .content
                .blocks
                .get(target.index_hint)
                .is_some_and(|block| {
                    fingerprint(block).is_ok_and(|hash| hash == target.fingerprint)
                }) {
                target.index_hint
            } else {
                let matches = expected
                    .content
                    .blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(index, block)| {
                        fingerprint(block)
                            .ok()
                            .filter(|hash| hash == &target.fingerprint)
                            .map(|_| index)
                    })
                    .collect::<Vec<_>>();
                anyhow::ensure!(
                    matches.len() == 1,
                    fail("Language target block changed or became ambiguous")
                );
                matches[0]
            };
            let original = &expected.content.blocks[index];
            anyhow::ensure!(
                original.bbox.as_ref() == Some(&selection.region)
                    && original.kind != "table"
                    && original.kind != "figure",
                fail(
                    "Only an exact whole text/paragraph block can be replaced; tables and cross-block crops contribute evidence only"
                )
            );
            anyhow::ensure!(
                !replacements.contains_key(&index),
                fail(
                    "Multiple routes would replace the same paragraph; combine its languages or choose separate source blocks"
                )
            );
            if content
                .blocks
                .iter()
                .any(|block| block.kind != "figure" && !block.plain().trim().is_empty())
            {
                let mut blocks = content.blocks.clone();
                for block in &mut blocks {
                    block.uncertain = true;
                    if original.plain() != block.plain() && original.plain().len() < 120000 {
                        block.alternatives.push(original.plain());
                    }
                }
                replacements.insert(index, blocks);
            }
        }
        next.candidates.push(Candidate {
            model_id: format!("tesseract:{}", selection.languages),
            family: "tesseract".into(),
            view: format!("language_region:{}", selection.id),
            region: Some(view),
            content,
            reliability: 0.65,
        });
    }
    let mut blocks = vec![];
    for (index, block) in expected.content.blocks.iter().enumerate() {
        if let Some(replacement) = replacements.remove(&index) {
            blocks.extend(replacement);
        } else {
            blocks.push(block.clone());
        }
    }
    next.content.blocks = blocks;
    // Masked views contain no private auxiliary precision or hidden alternatives.
    if !expected.confirmed_redactions.is_empty() {
        next.content = crate::redactions::enforce_view(
            &next.content,
            &source,
            &expected.confirmed_redactions,
            None,
            c,
        )?
        .0;
    }
    for selection in selections {
        next.region_languages.retain(|old| old.id != selection.id);
        next.region_languages.push(selection.clone());
    }
    next.reviewed = false;
    next.edited = true;
    next.agreement = 0.;
    next.content.summary = next.content.plain().chars().take(2000).collect();
    next.source_warnings.push("Regional language OCR changed/adds evidence. Its Tesseract passes are one reader family, not independent agreement; compare the source before approval.".into());
    next.validate()?;
    let fresh = geometry::source_identity(&project.image(expected.page), expected.page, c)?;
    anyhow::ensure!(
        fresh == source && fingerprint(&project.load(expected.page)?)? == before_fingerprint,
        fail("Page/source changed while recognizing language regions")
    );
    let token = preview_token(&source, &before_fingerprint, &next)?;
    Ok(RereadPreview{source,before_fingerprint,after:next,token,note:"Preview only. Whole selected paragraphs with usable OCR may be replaced; previous readings remain immutable evidence/history. Arbitrary, table and cross-block regions only add alternatives. No checkpoint changes until explicit apply.".into()})
}
pub fn apply_preview(
    project: &Project,
    expected: &Page,
    preview: &RereadPreview,
    token: &str,
    c: &Control,
) -> Result<u64> {
    let (source, _) = crate::redactions::recognition_view(project, expected, c)?;
    anyhow::ensure!(
        source == preview.source
            && fingerprint(expected)? == preview.before_fingerprint
            && token == preview.token
            && token == preview_token(&source, &preview.before_fingerprint, &preview.after)?
            && preview.after.page == expected.page
            && preview.after.image == expected.image
            && preview.after.confirmed_redactions == expected.confirmed_redactions
            && fingerprint(&preview.after.manual_layers)? == fingerprint(&expected.manual_layers)?,
        fail("Confirm the exact fresh regional recognition preview")
    );
    let retained_candidates = &preview.after.candidates[..expected
        .candidates
        .len()
        .min(preview.after.candidates.len())];
    anyhow::ensure!(
        preview.after.candidates.len() >= expected.candidates.len()
            && fingerprint(&retained_candidates)? == fingerprint(&expected.candidates)?,
        fail("Regional recognition cannot discard existing reader evidence")
    );
    preview.after.validate()?;
    c.check()?;
    crate::review::commit(
        project,
        expected,
        &preview.after,
        "Apply region language OCR",
    )
}
