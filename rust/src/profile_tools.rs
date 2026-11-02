//! Inspectable handwriting-profile tools. Suggestions never change recognition context.
use crate::{
    fontpack::{self, FontOptions},
    storage::{Control, atomic_json, fingerprint, hash_file, read_json},
    types::*,
    writers::{self, Profile},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::Path,
};

pub const CAPTURE_CONDITIONS: &[&str] = &[
    "clear",
    "cursive",
    "crowded",
    "blur",
    "slanted",
    "ruled_paper",
    "shadow",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoverageReport {
    pub active_samples: usize,
    pub quarantined_samples: usize,
    pub glyph_counts: BTreeMap<String, usize>,
    pub observed_characters: BTreeMap<String, usize>,
    pub missing_letters: String,
    pub missing_digits: String,
    pub confirmed_conditions: BTreeMap<String, usize>,
    pub missing_conditions: Vec<String>,
    pub suggestions: Vec<String>,
}
pub fn coaching(root: &Path, id: &str) -> Result<CoverageReport> {
    let profile = writers::read(root, id)?;
    let mut report = CoverageReport {
        active_samples: 0,
        quarantined_samples: 0,
        glyph_counts: BTreeMap::new(),
        observed_characters: BTreeMap::new(),
        missing_letters: String::new(),
        missing_digits: String::new(),
        confirmed_conditions: BTreeMap::new(),
        missing_conditions: vec![],
        suggestions: vec![],
    };
    for sample in &profile.samples {
        if !sample.active() {
            report.quarantined_samples += 1;
            continue;
        }
        report.active_samples += 1;
        if sample.default_glyph() {
            *report.glyph_counts.entry(sample.label.clone()).or_default() += 1;
        }
        for ch in sample
            .label
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<BTreeSet<_>>()
        {
            *report
                .observed_characters
                .entry(ch.to_string())
                .or_default() += 1;
        }
        for condition in &sample.conditions {
            *report
                .confirmed_conditions
                .entry(condition.clone())
                .or_default() += 1;
        }
    }
    report.missing_letters = ('A'..='Z')
        .chain('a'..='z')
        .filter(|c| !report.glyph_counts.contains_key(&c.to_string()))
        .collect();
    report.missing_digits = ('0'..='9')
        .filter(|c| !report.glyph_counts.contains_key(&c.to_string()))
        .collect();
    report.missing_conditions = CAPTURE_CONDITIONS
        .iter()
        .filter(|v| !report.confirmed_conditions.contains_key(**v))
        .map(|v| v.to_string())
        .collect();
    if !report.missing_digits.is_empty() {
        report.suggestions.push(format!("Collect confirmed isolated digits: {}. Preserve decimal points and signs in separate phrase examples.", report.missing_digits));
    }
    if !report.missing_letters.is_empty() {
        report.suggestions.push(format!("Collect isolated uppercase and lowercase forms: {}. Word labels help recognition but do not supply isolated font glyphs.", report.missing_letters));
    }
    if !report.missing_conditions.is_empty() {
        report.suggestions.push(format!("Add examples from these writing/capture conditions and label the conditions yourself: {}.", report.missing_conditions.join(", ")));
    }
    report.suggestions.push("Coverage counts confirmed labels, not recognition accuracy. Evaluate on held-out notes; more examples do not guarantee better results.".into());
    Ok(report)
}

fn edit_sample(
    root: &Path,
    id: &str,
    sample_id: &str,
    change: impl FnOnce(&mut writers::Sample) -> Result<()>,
) -> Result<Profile> {
    let _lock = crate::writer_history::lock(root, id)?;
    let mut p = writers::read(root, id)?;
    let before = p.clone();
    let sample = p
        .samples
        .iter_mut()
        .find(|s| s.id == sample_id)
        .ok_or_else(|| Fault::new("NG610", "Handwriting example not found"))?;
    change(sample)?;
    writers::bump_revision(&mut p)?;
    writers::validate(&p)?;
    crate::writer_history::commit_locked(
        root,
        &before,
        &p,
        "Edit learned example conditions or quarantine",
    )?;
    Ok(p)
}
pub fn set_conditions(
    root: &Path,
    id: &str,
    sample_id: &str,
    conditions: &[String],
) -> Result<Profile> {
    edit_sample(root, id, sample_id, |s| {
        s.conditions = conditions
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(())
    })
}
pub fn quarantine(root: &Path, id: &str, sample_id: &str, reason: &str) -> Result<Profile> {
    anyhow::ensure!(
        !reason.trim().is_empty() && reason.len() <= 2000,
        Fault::new("NG610", "Enter a quarantine reason of 1–2000 bytes")
    );
    edit_sample(root, id, sample_id, |s| {
        s.quarantine = Some(writers::Quarantine {
            reason: reason.trim().into(),
            created: chrono::Utc::now().to_rfc3339(),
        });
        Ok(())
    })
}
pub fn restore_example(root: &Path, id: &str, sample_id: &str) -> Result<Profile> {
    edit_sample(root, id, sample_id, |s| {
        s.quarantine = None;
        Ok(())
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfusionExercise {
    pub first: String,
    pub second: String,
    pub first_examples: Vec<String>,
    pub second_examples: Vec<String>,
    pub prompt: String,
}
pub fn confusion_exercises(root: &Path, id: &str) -> Result<Vec<ConfusionExercise>> {
    let p = writers::read(root, id)?;
    Ok([("r","n"), ("1","l"), ("0","O"), ("5","S"), ("2","Z"), ("m","rn")].into_iter().map(|(a,b)| {
        let ids = |label: &str| p.samples.iter().filter(|s| s.active() && s.label.contains(label)).map(|s| s.id.clone()).take(8).collect::<Vec<_>>();
        let first = ids(a); let second = ids(b);
        ConfusionExercise {
            first: a.into(), second: b.into(), first_examples: first.clone(), second_examples: second.clone(),
            prompt: format!("Compare confirmed '{a}' and '{b}' samples ({} and {} available). Collect a clearly labelled sample of each in the same writing conditions, then compare them in words and numbers. This is a common ambiguity exercise, not a measured error claim.", first.len(), second.len()),
        }
    }).collect())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlossarySuggestion {
    pub term: String,
    pub example_ids: Vec<String>,
    pub reason: String,
}
pub fn glossary_suggestions(root: &Path, id: &str) -> Result<Vec<GlossarySuggestion>> {
    let p = writers::read(root, id)?;
    let mut terms: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for sample in p.samples.iter().filter(|s| s.active() && !s.glyph) {
        for term in sample
            .label
            .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        {
            let term = term.trim_matches(['-', '\'']);
            if (3..=60).contains(&term.chars().count()) && term.chars().any(char::is_alphabetic) {
                terms
                    .entry(term.into())
                    .or_default()
                    .insert(sample.id.clone());
            }
        }
    }
    Ok(terms.into_iter().map(|(term, ids)| GlossarySuggestion {
        term, example_ids: ids.into_iter().collect(),
        reason: "Present in a confirmed phrase label. Preview only: add it to project context only after you approve it.".into(),
    }).collect())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub writer_id: String,
    pub revision: u64,
    pub created: String,
    pub archive_sha256: String,
    pub profile_digest: String,
}
fn snapshot_dir(root: &Path, id: &str, version: &str) -> Result<std::path::PathBuf> {
    anyhow::ensure!(
        writers::valid_id(version),
        Fault::new("NG610", "Invalid profile version ID")
    );
    Ok(writers::folder(root, id)?.join("versions").join(version))
}
pub fn snapshot(root: &Path, id: &str, name: &str, c: &Control) -> Result<Snapshot> {
    c.check()?;
    anyhow::ensure!(
        !name.trim().is_empty() && name.len() <= 120,
        Fault::new("NG610", "Enter a version name of 1–120 bytes")
    );
    let dir = writers::folder(root, id)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    lock.lock()?;
    let profile = writers::read(root, id)?;
    let versions = dir.join("versions");
    fs::create_dir_all(&versions)?;
    let stage = tempfile::tempdir_in(&versions)?;
    let archive = stage.path().join("profile.zip");
    writers::export_profile(root, id, &archive, c)?;
    anyhow::ensure!(
        archive.metadata()?.len() <= 100_000_000,
        Fault::new(
            "NG610",
            "Profile version exceeds the 100 MB portable import limit; use a smaller selected profile"
        )
    );
    let digest = fingerprint(&profile)?;
    let version = Snapshot {
        schema: 1,
        id: format!(
            "version-{}",
            &fingerprint(&(name, &digest, chrono::Utc::now().timestamp_nanos_opt()))?[..24]
        ),
        name: name.trim().into(),
        writer_id: id.into(),
        revision: profile.revision,
        created: chrono::Utc::now().to_rfc3339(),
        archive_sha256: hash_file(&archive, c)?,
        profile_digest: digest,
    };
    atomic_json(&stage.path().join("snapshot.json"), &version)?;
    c.check()?;
    let destination = snapshot_dir(root, id, &version.id)?;
    anyhow::ensure!(
        !destination.exists(),
        Fault::new("NG401", "Profile version already exists")
    );
    fs::rename(stage.path(), destination)?;
    Ok(version)
}
pub fn snapshots(root: &Path, id: &str) -> Result<Vec<Snapshot>> {
    writers::read(root, id)?;
    let dir = writers::folder(root, id)?.join("versions");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut versions = vec![];
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("version-") || !writers::valid_id(&name) {
            continue;
        }
        let version: Snapshot = read_json(&entry.path().join("snapshot.json"), 20000)?;
        anyhow::ensure!(
            version.schema == 1 && version.id == name && version.writer_id == id,
            Fault::new("NG610", "Profile version metadata mismatch")
        );
        versions.push(version);
    }
    versions.sort_by(|a, b| a.created.cmp(&b.created));
    Ok(versions)
}
fn snapshot_profile(
    root: &Path,
    id: &str,
    version: &str,
    c: &Control,
) -> Result<(Profile, std::path::PathBuf)> {
    let dir = snapshot_dir(root, id, version)?;
    let metadata: Snapshot = read_json(&dir.join("snapshot.json"), 20000)?;
    anyhow::ensure!(
        metadata.schema == 1 && metadata.id == version && metadata.writer_id == id,
        Fault::new("NG610", "Profile version metadata mismatch")
    );
    let archive = dir.join("profile.zip");
    anyhow::ensure!(
        metadata.archive_sha256 == hash_file(&archive, c)?,
        Fault::new("NG610", "Profile version archive checksum mismatch")
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
    let mut bytes = vec![];
    zip.by_name("profile.json")?
        .take(8_000_001)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 8_000_000,
        Fault::new("NG610", "Oversized profile version")
    );
    let profile: Profile = serde_json::from_slice(&bytes)?;
    writers::validate(&profile)?;
    anyhow::ensure!(
        profile.id == id && fingerprint(&profile)? == metadata.profile_digest,
        Fault::new("NG610", "Profile version content mismatch")
    );
    Ok((profile, archive))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionComparison {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub name_changed: bool,
    pub notes_changed: bool,
}
pub fn compare_snapshots(
    root: &Path,
    id: &str,
    left: &str,
    right: &str,
    c: &Control,
) -> Result<VersionComparison> {
    let (a, _) = snapshot_profile(root, id, left, c)?;
    let (b, _) = snapshot_profile(root, id, right, c)?;
    let first = a
        .samples
        .iter()
        .map(|s| (&s.id, s))
        .collect::<BTreeMap<_, _>>();
    let second = b
        .samples
        .iter()
        .map(|s| (&s.id, s))
        .collect::<BTreeMap<_, _>>();
    let mut result = VersionComparison {
        added: vec![],
        removed: vec![],
        changed: vec![],
        name_changed: a.name != b.name,
        notes_changed: a.notes != b.notes,
    };
    for (id, sample) in &second {
        if let Some(old) = first.get(id) {
            if fingerprint(old)? != fingerprint(sample)? {
                result.changed.push((*id).clone());
            }
        } else {
            result.added.push((*id).clone());
        }
    }
    for id in first.keys().filter(|id| !second.contains_key(*id)) {
        result.removed.push((*id).clone());
    }
    Ok(result)
}
pub fn branch(
    root: &Path,
    id: &str,
    version: Option<&str>,
    name: &str,
    c: &Control,
) -> Result<Profile> {
    anyhow::ensure!(
        !name.trim().is_empty() && name.len() <= 120,
        Fault::new("NG610", "Enter a branch name of 1–120 bytes")
    );
    let temporary = tempfile::tempdir()?;
    let archive = if let Some(version) = version {
        snapshot_profile(root, id, version, c)?.1
    } else {
        let archive = temporary.path().join("profile.zip");
        writers::export_profile(root, id, &archive, c)?;
        archive
    };
    // Import stages all images and assigns a new identity; the source is unchanged.
    let profile = writers::import_profile(root, &archive, c)?;
    writers::update(root, &profile.id, name, &profile.notes)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShareSelection {
    pub sample_ids: Vec<String>,
    pub include_name: bool,
    pub include_notes: bool,
    pub include_sources: bool,
    pub include_dates: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharePreview {
    pub writer_id: String,
    pub revision: u64,
    pub files: Vec<String>,
    pub labels: Vec<String>,
    pub selected_samples: usize,
    pub image_bytes: u64,
    pub included_metadata: Vec<String>,
    pub warning: String,
    pub confirmation_token: String,
}
fn selected_profile(root: &Path, id: &str, selection: &ShareSelection) -> Result<Profile> {
    let mut p = writers::read(root, id)?;
    let ids = selection.sample_ids.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        !ids.is_empty()
            && ids.len() == selection.sample_ids.len()
            && ids
                .iter()
                .all(|id| p.samples.iter().any(|s| &s.id == *id && s.active())),
        Fault::new(
            "NG610",
            "Choose distinct active example IDs; quarantined or missing examples cannot be shared"
        )
    );
    p.samples.retain(|s| ids.contains(&s.id));
    if !selection.include_name {
        p.name = "Shared writer".into();
    }
    if !selection.include_notes {
        p.notes.clear();
    }
    for sample in &mut p.samples {
        if !selection.include_sources {
            sample.source.clear();
        }
        if !selection.include_dates {
            sample.created.clear();
        }
    }
    Ok(p)
}
pub fn share_preview(
    root: &Path,
    id: &str,
    selection: &ShareSelection,
    c: &Control,
) -> Result<SharePreview> {
    c.check()?;
    let p = selected_profile(root, id, selection)?;
    let mut bytes = 0;
    let mut hashes = vec![];
    let mut files = vec![
        "profile.json".into(),
        "privacy-preview.json".into(),
        "README.txt".into(),
    ];
    for sample in &p.samples {
        c.check()?;
        let path = writers::folder(root, id)?.join(&sample.image);
        let length = path.metadata()?.len();
        anyhow::ensure!(
            length <= 1_000_000,
            Fault::new("NG610", "Stored sample is too large")
        );
        bytes += length;
        hashes.push(hash_file(&path, c)?);
        files.push(format!("samples/{}", sample.image));
    }
    let mut included = vec![
        "writer and sample IDs, schema and revision".into(),
        "confirmed labels, glyph flags and explicit font-shape metadata".into(),
        "selected handwriting images".into(),
        "user-confirmed capture conditions".into(),
    ];
    for (flag, name) in [
        (selection.include_name, "writer name"),
        (selection.include_notes, "writer notes"),
        (selection.include_sources, "source descriptions"),
        (selection.include_dates, "sample dates"),
    ] {
        if flag {
            included.push(name.into());
        }
    }
    let preview = SharePreview {
        writer_id: id.into(),revision:p.revision,files,labels:p.samples.iter().map(|s|s.label.clone()).collect(),selected_samples:p.samples.len(),image_bytes:bytes,included_metadata:included,
        warning:"This local ZIP contains selected handwriting, labels and the listed metadata. No upload is performed. Inspect this preview before creating or manually sharing it.".into(),
        confirmation_token:fingerprint(&(&p,selection,hashes))?,
    };
    let metadata = serde_json::to_vec(&p)?;
    let bound = bytes
        + metadata.len() as u64
        + serde_json::to_vec(&preview)?.len() as u64
        + preview.warning.len() as u64
        + preview.files.len() as u64 * 256;
    anyhow::ensure!(
        metadata.len() <= 8_000_000 && bound <= 100_000_000,
        Fault::new(
            "NG610",
            "Selected profile exceeds the portable import limit; select fewer examples"
        )
    );
    Ok(preview)
}
pub fn export_selected(
    root: &Path,
    id: &str,
    selection: &ShareSelection,
    preview_token: &str,
    output: &Path,
    c: &Control,
) -> Result<()> {
    c.check()?;
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Writer pack already exists")
    );
    anyhow::ensure!(
        output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip")),
        Fault::new("NG610", "Choose a .zip writer pack")
    );
    let dir = writers::folder(root, id)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    lock.lock()?;
    let preview = share_preview(root, id, selection, c)?;
    anyhow::ensure!(
        preview_token == preview.confirmation_token,
        Fault::new(
            "NG610",
            "Sharing selection or profile changed; inspect a fresh privacy preview"
        )
    );
    let profile = selected_profile(root, id, selection)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = zip::ZipWriter::new(&mut temp);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("profile.json", serde_json::to_vec_pretty(&profile)?),
            ("privacy-preview.json", serde_json::to_vec_pretty(&preview)?),
            ("README.txt", preview.warning.as_bytes().to_vec()),
        ] {
            zip.start_file(name, options)?;
            zip.write_all(&data)?;
        }
        for sample in &profile.samples {
            c.check()?;
            zip.start_file(format!("samples/{}", sample.image), options)?;
            std::io::copy(&mut fs::File::open(dir.join(&sample.image))?, &mut zip)?;
        }
        zip.finish()?;
    }
    c.check()?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output)
        .map_err(|_| Fault::new("NG401", "Writer pack collision or unavailable folder"))?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProofSummary {
    pub glyph_count: usize,
    pub missing_letters_and_digits: String,
}
pub fn font_proof_sheet(
    root: &Path,
    id: &str,
    output: &Path,
    options: &FontOptions,
    c: &Control,
) -> Result<ProofSummary> {
    c.check()?;
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Font proof sheet already exists")
    );
    anyhow::ensure!(
        output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("html")),
        Fault::new("NG620", "Choose an .html proof sheet")
    );
    let stage = tempfile::tempdir()?;
    let pack = stage.path().join("font.zip");
    let count = fontpack::export(root, id, &pack, options, c)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(pack)?)?;
    let mut font = vec![];
    zip.by_name("Handwriting.ttf")?.read_to_end(&mut font)?;
    // Use the exact snapshot embedded beside this font, rather than rereading a
    // profile that another confirmation/quarantine action might have changed.
    let profile: writers::Profile = serde_json::from_reader(zip.by_name("profile.json")?)?;
    let coverage: fontpack::FontCoverage =
        serde_json::from_reader(zip.by_name("font-coverage.json")?)?;
    let glyphs = coverage.default_glyphs.chars().collect::<BTreeSet<_>>();
    let missing = ('A'..='Z')
        .chain('a'..='z')
        .chain('0'..='9')
        .filter(|c| !glyphs.contains(c))
        .collect::<String>();
    let mut html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Font proof sheet</title><style>@font-face{{font-family:Proof;src:url(data:font/ttf;base64,{}) format('truetype')}}body{{font:16px/1.6 system-ui;max-width:60rem;margin:2rem auto;padding:1rem}}.sample{{font:32px/1.8 Proof,monospace;white-space:pre-wrap}}.missing{{font-family:system-ui;color:#a00000;text-decoration:underline}}.practice{{border-bottom:1px dashed #888;min-height:3rem}}@media print{{body{{margin:0}}.practice{{break-inside:avoid}}}}</style></head><body><h1>{}</h1><p>{count} confirmed glyphs/shapes. Missing standalone letters/numbers: <strong>{}</strong>.</p><p>Red underlined text uses a fallback font because this handwriting font has no confirmed standalone glyph or complete sequence for it. Print using the browser's Print command. The font is embedded; no network connection is used. Only explicitly confirmed ligatures, complete clusters and alternate forms are substituted; contextual joining and missing forms are not inferred.</p>",
        base64::engine::general_purpose::STANDARD.encode(font),
        crate::word::xml(&options.family),
        crate::word::xml(&missing)
    );
    let sample_html = |text: &str| {
        let mut html = String::new();
        let mut remaining = text;
        while !remaining.is_empty() {
            if let Some(rule) = coverage
                .shapes
                .iter()
                .filter(|rule| {
                    !matches!(rule.shape, writers::FontShape::Alternate { .. })
                        && remaining.starts_with(&rule.label)
                })
                .max_by_key(|rule| rule.label.len())
            {
                html.push_str(&crate::word::xml(&rule.label));
                remaining = &remaining[rule.label.len()..];
            } else {
                let ch = remaining.chars().next().expect("nonempty remainder");
                let escaped = crate::word::xml(&ch.to_string());
                if ch.is_whitespace() || glyphs.contains(&ch) {
                    html.push_str(&escaped);
                } else {
                    html.push_str(&format!("<span class=\"missing\">{escaped}</span>"));
                }
                remaining = &remaining[ch.len_utf8()..];
            }
        }
        html
    };
    html.push_str(&format!("<p>{}</p><p>Unconfirmed standalone components (visible boxes when no confirmed sequence applies): <strong>{}</strong></p>",crate::word::xml(&coverage.note),crate::word::xml(&coverage.component_placeholders)));
    if !coverage.shapes.is_empty() {
        html.push_str("<h2>Confirmed shaping forms</h2>");
    }
    for rule in &coverage.shapes {
        c.check()?;
        let (feature, value) = match rule.shape {
            writers::FontShape::Alternate { index } => ("salt", index),
            _ => (rule.feature.as_str(), 1),
        };
        html.push_str(&format!("<p>Confirmed {} · glyph {} · {}={value}</p><p class=\"sample\" style=\"font-feature-settings:'{feature}' {value}\">{}</p>",crate::word::xml(&rule.label),rule.glyph_id,crate::word::xml(feature),crate::word::xml(&rule.label)));
        if let writers::FontShape::Alternate { index } = rule.shape {
            html.push_str(&format!("<p>Equivalent stylistic set ss{index:02}</p><p class=\"sample\" style=\"font-feature-settings:'ss{index:02}' 1\">{}</p>",crate::word::xml(&rule.label)));
        }
    }
    html.push_str(&format!(
        "<h2>Words and sentences</h2><p class=\"sample\">{}</p>",
        sample_html("The quick brown fox jumps over the lazy dog.\n0123456789 1.25 -10 +20")
    ));
    for sample in profile
        .samples
        .iter()
        .filter(|s| s.active() && !s.glyph && s.font_shape.is_none())
        .take(12)
    {
        html.push_str(&format!(
            "<p class=\"sample\">{}</p>",
            sample_html(&sample.label)
        ));
    }
    html.push_str("<h2>Printable practice</h2>");
    for ch in glyphs {
        c.check()?;
        html.push_str(&format!(
            "<div class=\"practice\"><span class=\"sample\">{} {} {}</span></div>",
            crate::word::xml(&ch.to_string()),
            crate::word::xml(&ch.to_string()),
            crate::word::xml(&ch.to_string())
        ));
    }
    html.push_str("</body></html>\n");
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(html.as_bytes())?;
    c.check()?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output)
        .map_err(|_| Fault::new("NG401", "Font proof sheet collision or unavailable folder"))?;
    Ok(ProofSummary {
        glyph_count: count,
        missing_letters_and_digits: missing,
    })
}
