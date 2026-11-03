//! Opt-in local glyph comparison. A shape match is never writer identification.
use crate::{
    assistance::WriterOptions,
    storage::{Control, fingerprint, hash_file},
    types::*,
    writers,
};
use image::{DynamicImage, GrayImage};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SuggestOptions {
    pub enabled: bool,
    pub max_results: usize,
}
impl Default for SuggestOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            max_results: 5,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileSuggestion {
    pub writer_id: String,
    pub name: String,
    pub revision: u64,
    pub profile_fingerprint: String,
    pub ink_overlap_similarity: f64,
    pub best_sample_id: String,
    pub examples_compared: usize,
    pub example_hashes: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuggestionReport {
    pub schema: u32,
    pub enabled: bool,
    pub state: String,
    pub known_label: Option<String>,
    pub root_identity: String,
    pub query_fingerprint: String,
    pub suggestions: Vec<ProfileSuggestion>,
    pub warnings: Vec<String>,
    pub note: String,
    pub confirmation_token: String,
}
fn report_token(report: &SuggestionReport) -> Result<String> {
    fingerprint(&(
        report.schema,
        report.enabled,
        &report.state,
        &report.known_label,
        &report.root_identity,
        &report.query_fingerprint,
        &report.suggestions,
        &report.warnings,
        &report.note,
    ))
}
fn ink_overlap(a: &GrayImage, b: &GrayImage) -> Option<f64> {
    let mut intersection = 0u64;
    let mut union = 0u64;
    for (first, second) in a.pixels().zip(b.pixels()) {
        let x = 255 - u64::from(first[0]);
        let y = 255 - u64::from(second[0]);
        intersection += x.min(y);
        union += x.max(y);
    }
    if union == 0 {
        None
    } else {
        Some(intersection as f64 / union as f64)
    }
}
pub fn suggest(
    root: &Path,
    image: &DynamicImage,
    known_label: Option<&str>,
    options: &SuggestOptions,
    c: &Control,
) -> Result<SuggestionReport> {
    c.check()?;
    let mut report=SuggestionReport{schema:1,enabled:options.enabled,state:"disabled".into(),known_label:known_label.map(str::to_owned),root_identity:String::new(),query_fingerprint:String::new(),suggestions:vec![],warnings:vec![],note:"Local opt-in comparison of one explicitly labelled glyph with active confirmed samples. Similarity is an uncalibrated ink-overlap measure, not identity confidence, recognition accuracy or proof that two notes share a writer. Common glyphs and many examples can produce misleading matches. No profile is selected or settings changed automatically; explicitly confirm a proposed writer after comparing source examples.".into(),confirmation_token:String::new()};
    if !options.enabled {
        report.confirmation_token = report_token(&report)?;
        return Ok(report);
    }
    anyhow::ensure!(
        (1..=10).contains(&options.max_results),
        Fault::new("NG610", "Choose 1–10 profile suggestions")
    );
    let Some(label) = known_label.map(str::trim).filter(|s| !s.is_empty()) else {
        report.state = "known_label_required".into();
        report.confirmation_token = report_token(&report)?;
        return Ok(report);
    };
    if label.chars().count() != 1 || label.chars().all(char::is_whitespace) || label.len() > 8 {
        report.state = "single_glyph_required".into();
        report.confirmation_token = report_token(&report)?;
        return Ok(report);
    }
    report.known_label = Some(label.into());
    anyhow::ensure!(
        image.width() > 0
            && image.height() > 0
            && image.width() <= 20000
            && image.height() <= 20000
            && u64::from(image.width()) * u64::from(image.height()) <= 40_000_000,
        Fault::new("NG610", "Glyph crop exceeds the bounded image dimensions")
    );
    let query = writers::normalize(image, 64, 80)?;
    report.query_fingerprint = fingerprint(&(label, query.as_raw()))?;
    if !root.exists() {
        report.state = "no_matching_confirmed_glyphs".into();
        report.confirmation_token = report_token(&report)?;
        return Ok(report);
    }
    crate::watch::no_links(root)?;
    let root = root.canonicalize()?;
    report.root_identity = fingerprint(&root.to_string_lossy())?;
    let entries = fs::read_dir(&root)?.collect::<std::io::Result<Vec<_>>>()?;
    anyhow::ensure!(
        entries.len() <= 1000,
        Fault::new(
            "NG610",
            "Profile suggestions are limited to 1,000 local directory entries"
        )
    );
    for entry in entries {
        c.check()?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !writers::valid_id(&id) {
            continue;
        }
        let profile = match writers::read(&root, &id) {
            Ok(p) => p,
            Err(error) => {
                report
                    .warnings
                    .push(format!("Skipped unreadable profile {id}: {error}"));
                continue;
            }
        };
        let samples = profile
            .samples
            .iter()
            .rev()
            .filter(|s| s.active() && s.glyph && s.label == label)
            .take(32)
            .collect::<Vec<_>>();
        if samples.is_empty() {
            continue;
        }
        let mut best: Option<(f64, String)> = None;
        let mut hashes = BTreeMap::new();
        for sample in samples {
            c.check()?;
            let path = root.join(&id).join(&sample.image);
            let read = || -> Result<(GrayImage, String)> {
                crate::watch::no_links(&path)?;
                let image = writers::open_image(&path, 400_000)?.to_luma8();
                anyhow::ensure!(
                    image.dimensions() == (64, 80),
                    Fault::new("NG610", "Confirmed glyph has invalid stored dimensions")
                );
                Ok((image, hash_file(&path, c)?))
            };
            let (image, hash) = match read() {
                Ok(v) => v,
                Err(error) => {
                    c.check()?;
                    report.warnings.push(format!(
                        "Skipped unavailable glyph {} in {id}: {error}",
                        sample.id
                    ));
                    continue;
                }
            };
            if let Some(score) = ink_overlap(&query, &image) {
                hashes.insert(sample.id.clone(), hash);
                if best.as_ref().is_none_or(|(current, _)| score > *current) {
                    best = Some((score, sample.id.clone()));
                }
            }
        }
        if let Some((score, best_sample_id)) = best {
            report.suggestions.push(ProfileSuggestion {
                writer_id: id,
                name: profile.name.clone(),
                revision: profile.revision,
                profile_fingerprint: fingerprint(&profile)?,
                ink_overlap_similarity: score,
                best_sample_id,
                examples_compared: hashes.len(),
                example_hashes: hashes,
            });
        }
    }
    report.suggestions.sort_by(|a, b| {
        b.ink_overlap_similarity
            .total_cmp(&a.ink_overlap_similarity)
            .then_with(|| a.writer_id.cmp(&b.writer_id))
    });
    report.suggestions.truncate(options.max_results);
    report.state = if report.suggestions.is_empty() {
        "no_matching_confirmed_glyphs"
    } else {
        "candidate_matches"
    }
    .into();
    report.confirmation_token = report_token(&report)?;
    Ok(report)
}
/// Returns an explicit selection for the caller to apply. This does not write settings.
pub fn confirm(
    root: &Path,
    report: &SuggestionReport,
    selected_id: &str,
    confirmation_token: &str,
    c: &Control,
) -> Result<WriterOptions> {
    c.check()?;
    anyhow::ensure!(
        report.schema == 1
            && report.enabled
            && report.state == "candidate_matches"
            && confirmation_token == report.confirmation_token
            && report_token(report)? == report.confirmation_token,
        Fault::new(
            "NG610",
            "Profile suggestion confirmation is missing, disabled or stale"
        )
    );
    crate::watch::no_links(root)?;
    let root = root.canonicalize()?;
    anyhow::ensure!(
        report.root_identity == fingerprint(&root.to_string_lossy())?,
        Fault::new(
            "NG610",
            "Profile suggestions belong to another local collection"
        )
    );
    let proposal = report
        .suggestions
        .iter()
        .find(|p| p.writer_id == selected_id)
        .ok_or_else(|| Fault::new("NG610", "Explicitly choose one of the proposed writer IDs"))?;
    let profile = writers::read(&root, selected_id)?;
    anyhow::ensure!(
        profile.revision == proposal.revision
            && fingerprint(&profile)? == proposal.profile_fingerprint,
        Fault::new(
            "NG610",
            "Writer profile changed after the suggestion; compare it again"
        )
    );
    let label = report
        .known_label
        .as_deref()
        .ok_or_else(|| Fault::new("NG610", "Suggested glyph label is missing"))?;
    anyhow::ensure!(
        proposal.example_hashes.len() == proposal.examples_compared
            && !proposal.example_hashes.is_empty()
            && proposal
                .example_hashes
                .contains_key(&proposal.best_sample_id)
            && proposal.ink_overlap_similarity.is_finite()
            && (0.0..=1.0).contains(&proposal.ink_overlap_similarity),
        Fault::new("NG610", "Invalid profile suggestion evidence")
    );
    for (id, expected) in &proposal.example_hashes {
        c.check()?;
        let sample = profile
            .samples
            .iter()
            .find(|s| &s.id == id && s.active() && s.glyph && s.label == label)
            .ok_or_else(|| Fault::new("NG610", "A proposed example is missing or quarantined"))?;
        let path = root.join(selected_id).join(&sample.image);
        crate::watch::no_links(&path)?;
        anyhow::ensure!(
            hash_file(&path, c)? == *expected,
            Fault::new(
                "NG610",
                "A confirmed example image changed after the suggestion"
            )
        );
    }
    Ok(WriterOptions {
        root,
        id: Some(selected_id.into()),
        use_examples: true,
    })
}
