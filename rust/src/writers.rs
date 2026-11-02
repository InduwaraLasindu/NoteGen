//! Confirmed visual handwriting memory. This does not train neural weights.
use crate::{
    storage::{atomic_json, fingerprint, read_json},
    types::*,
};
use image::{DynamicImage, GrayImage, Luma};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub id: String,
    pub label: String,
    pub glyph: bool,
    pub image: String,
    pub created: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantine: Option<Quarantine>,
    /// User-confirmed capture conditions; never guessed from the image.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<String>,
    /// Explicit font-only shaping intent. Absent in legacy profiles and ordinary examples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_shape: Option<FontShape>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FontShape {
    Ligature,
    Cluster,
    Alternate { index: u8 },
}
pub(crate) fn font_scalar(ch: char) -> bool {
    !ch.is_control()
        && !ch.is_whitespace()
        && (ch as u32 & 0xffff) < 0xfffe
        && !matches!(ch as u32,0x00ad|0x061c|0x200b..=0x200f|0x202a..=0x202e|0x2060..=0x206f|0xfe00..=0xfe0f|0xfeff|0xfff9..=0xfffb|0x1bca0..=0x1bca3|0x1d173..=0x1d17a|0xe0000..=0xe0fff)
}
impl FontShape {
    fn valid(&self, label: &str, glyph: bool) -> bool {
        let count = label.chars().count();
        if !label.chars().all(|ch| {
            font_scalar(ch)
                || matches!(self, Self::Cluster)
                    && matches!(ch as u32,0x200c|0x200d|0xfe00..=0xfe0f|0xe0100..=0xe01ef)
        }) {
            return false;
        }
        match self {
            Self::Alternate { index } => glyph && count == 1 && (1..=16).contains(index),
            Self::Ligature => !glyph && (2..=16).contains(&count),
            Self::Cluster => {
                !glyph && (2..=16).contains(&count) && label.graphemes(true).count() == 1
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quarantine {
    pub reason: String,
    pub created: String,
}
impl Sample {
    pub fn active(&self) -> bool {
        self.quarantine.is_none()
    }
    pub fn default_glyph(&self) -> bool {
        self.glyph && self.font_shape.is_none()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub notes: String,
    pub revision: u64,
    pub samples: Vec<Sample>,
}
pub(crate) fn bump_revision(p: &mut Profile) -> Result<()> {
    p.revision = p.revision.checked_add(1).ok_or_else(|| {
        Fault::new(
            "NG610",
            "Writer revision counter is exhausted; restore an earlier profile snapshot",
        )
    })?;
    Ok(())
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
pub(crate) fn folder(root: &Path, id: &str) -> Result<PathBuf> {
    anyhow::ensure!(valid_id(id), Fault::new("NG610", "Invalid writer ID"));
    Ok(root.join(id))
}
pub(crate) fn validate(p: &Profile) -> Result<()> {
    anyhow::ensure!(
        p.schema == 1
            && valid_id(&p.id)
            && !p.name.trim().is_empty()
            && p.name.len() <= 120
            && p.notes.len() <= 4000
            && p.samples.len() <= 10000,
        Fault::new("NG610", "Invalid or oversized handwriting profile")
    );
    let mut seen = std::collections::HashSet::new();
    for s in &p.samples {
        anyhow::ensure!(
            valid_id(&s.id)
                && seen.insert(&s.id)
                && s.image == format!("{}.png", s.id)
                && !s.label.trim().is_empty()
                && s.label.len() <= 160
                && (!s.glyph || s.label.chars().count() == 1 && s.label.chars().all(font_scalar))
                && s.font_shape
                    .as_ref()
                    .is_none_or(|shape| shape.valid(&s.label, s.glyph))
                && s.source.len() <= 4000
                && s.conditions.len() <= 16
                && s.conditions.iter().all(|v| !v.is_empty()
                    && v.len() <= 40
                    && v.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                && s.quarantine
                    .as_ref()
                    .is_none_or(|q| !q.reason.trim().is_empty()
                        && q.reason.len() <= 2000
                        && q.created.len() <= 100),
            Fault::new("NG610", "Invalid labelled handwriting example")
        );
    }
    Ok(())
}
pub fn read(root: &Path, id: &str) -> Result<Profile> {
    let p: Profile = read_json(&folder(root, id)?.join("profile.json"), 8_000_000)?;
    validate(&p)?;
    anyhow::ensure!(p.id == id, Fault::new("NG610", "Writer ID mismatch"));
    Ok(p)
}
pub fn list(root: &Path) -> Result<Vec<Profile>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut profiles = vec![];
    for entry in fs::read_dir(root)? {
        let e = entry?;
        if e.file_type()?.is_dir()
            && let Some(id) = e.file_name().to_str()
            && valid_id(id)
        {
            profiles.push(read(root, id)?);
        }
    }
    profiles.sort_by_key(|p| p.name.to_lowercase());
    Ok(profiles)
}
pub fn create(root: &Path, name: &str) -> Result<Profile> {
    let name = name.trim();
    anyhow::ensure!(
        !name.is_empty() && name.len() <= 120,
        Fault::new("NG610", "Writer name must contain 1–120 bytes")
    );
    let id = format!(
        "writer-{}",
        &fingerprint(&(
            name,
            chrono::Utc::now().timestamp_nanos_opt(),
            std::process::id()
        ))?[..20]
    );
    let p = Profile {
        schema: 1,
        id,
        name: name.into(),
        notes: String::new(),
        revision: 0,
        samples: vec![],
    };
    let dir = folder(root, &p.id)?;
    fs::create_dir_all(&dir)?;
    atomic_json(&dir.join("profile.json"), &p)?;
    Ok(p)
}
pub fn update(root: &Path, id: &str, name: &str, notes: &str) -> Result<Profile> {
    let dir = folder(root, id)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".lock"))?;
    lock.lock()?;
    let mut p = read(root, id)?;
    p.name = name.trim().into();
    p.notes = notes.into();
    bump_revision(&mut p)?;
    validate(&p)?;
    atomic_json(&dir.join("profile.json"), &p)?;
    Ok(p)
}
pub fn remove(root: &Path, id: &str) -> Result<()> {
    read(root, id)?;
    fs::remove_dir_all(folder(root, id)?)?;
    Ok(())
}
/// Bounded decode protects the desktop and imported examples from huge images.
pub fn open_image(path: &Path, max_pixels: u64) -> Result<DynamicImage> {
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20000);
    limits.max_image_height = Some(20000);
    limits.max_alloc = Some(max_pixels.saturating_mul(8).min(320_000_000));
    reader.limits(limits);
    let (w, h) = image::image_dimensions(path)?;
    anyhow::ensure!(
        w > 0 && h > 0 && w as u64 * h as u64 <= max_pixels,
        Fault::new("NG610", "Image exceeds the configured memory limit")
    );
    Ok(reader.decode()?)
}
/// Crop ink, normalize contrast, and preserve aspect ratio within a white cell.
pub fn normalize(image: &DynamicImage, width: u32, height: u32) -> Result<GrayImage> {
    anyhow::ensure!(
        width >= 8
            && height >= 8
            && width <= 1024
            && height <= 1024
            && image.width() > 0
            && image.height() > 0,
        "Invalid handwriting dimensions"
    );
    let mut gray = image.thumbnail(2048, 2048).to_luma8();
    let (w, h) = gray.dimensions();
    let edge_mean = (0..w)
        .map(|x| gray[(x, 0)][0] as u64 + gray[(x, h - 1)][0] as u64)
        .sum::<u64>()
        / (2 * w as u64);
    if edge_mean < 128 {
        image::imageops::invert(&mut gray);
    }
    let mut hist = [0u64; 256];
    for p in gray.pixels() {
        hist[p[0] as usize] += 1;
    }
    let sum = hist
        .iter()
        .enumerate()
        .map(|(i, n)| i as u64 * n)
        .sum::<u64>();
    let total = (w as u64 * h as u64) as f64;
    let (mut count, mut partial, mut best, mut threshold) = (0f64, 0f64, 0f64, 180u8);
    for (i, n) in hist.iter().enumerate().take(245) {
        count += *n as f64;
        partial += i as f64 * *n as f64;
        if count == 0. || count == total {
            continue;
        }
        let variance = count
            * (total - count)
            * (partial / count - (sum as f64 - partial) / (total - count)).powi(2);
        if variance > best {
            best = variance;
            threshold = i as u8;
        }
    }
    threshold = threshold.clamp(60, 210);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in gray.enumerate_pixels() {
        if p[0] <= threshold {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    anyhow::ensure!(
        x0 < x1 && y0 < y1,
        Fault::new("NG610", "No visible ink found in the selected area")
    );
    let crop = image::imageops::crop_imm(&gray, x0, y0, x1 - x0, y1 - y0).to_image();
    let mut crop = crop;
    for p in crop.pixels_mut() {
        p[0] = if p[0] <= threshold { 0 } else { 255 };
    }
    let small = DynamicImage::ImageLuma8(crop)
        .thumbnail(width - 8, height - 8)
        .to_luma8();
    let mut output = GrayImage::from_pixel(width, height, Luma([255]));
    image::imageops::replace(
        &mut output,
        &small,
        ((width - small.width()) / 2) as i64,
        ((height - small.height()) / 2) as i64,
    );
    Ok(output)
}
pub fn add_example(
    root: &Path,
    id: &str,
    image: &DynamicImage,
    region: BoxRect,
    label: &str,
    glyph: bool,
    source: &str,
) -> Result<Profile> {
    add_selected_example(root, id, image, region, (label, glyph, None), source)
}
/// Confirm a complete ink shape for a ligature, cluster or explicitly selected alternate.
/// Ordinary word labels never become font substitutions automatically.
/// Ligatures contain 2–16 visible scalars; clusters contain one grapheme with
/// 2–16 scalars; alternate indexes 1–16 each label one visible scalar.
pub fn add_font_example(
    root: &Path,
    id: &str,
    image: &DynamicImage,
    region: BoxRect,
    label: &str,
    shape: FontShape,
    source: &str,
) -> Result<Profile> {
    let glyph = matches!(shape, FontShape::Alternate { .. });
    add_selected_example(root, id, image, region, (label, glyph, Some(shape)), source)
}
fn add_selected_example(
    root: &Path,
    id: &str,
    image: &DynamicImage,
    region: BoxRect,
    selection: (&str, bool, Option<FontShape>),
    source: &str,
) -> Result<Profile> {
    anyhow::ensure!(
        region.valid() && image.width() > 0 && image.height() > 0,
        Fault::new("NG610", "Select a visible handwriting region")
    );
    let (label, glyph, font_shape) = selection;
    let label = label.trim();
    anyhow::ensure!(
        !label.is_empty()
            && label.len() <= 160
            && (!glyph || label.chars().count() == 1 && label.chars().all(font_scalar))
            && font_shape
                .as_ref()
                .is_none_or(|shape| shape.valid(label, glyph)),
        Fault::new(
            "NG610",
            "Label one visible Unicode glyph, a bounded confirmed font shape, or a word/short phrase"
        )
    );
    let (x, y, w, h) = region.pixels(image.width(), image.height());
    let normalized = normalize(
        &image.crop_imm(x, y, w - x, h - y),
        if glyph { 64 } else { 320 },
        80,
    )?;
    let sample_id = if let Some(shape) = &font_shape {
        fingerprint(&(label, glyph, shape, normalized.as_raw()))?
    } else {
        fingerprint(&(label, glyph, normalized.as_raw()))?
    }[..24]
        .to_string();
    let dir = folder(root, id)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    lock.lock()?;
    let mut p = read(root, id)?;
    if p.samples.iter().any(|s| s.id == sample_id) {
        return Ok(p);
    }
    anyhow::ensure!(
        p.samples.len() < 10000,
        Fault::new("NG610", "Profile has reached 10,000 confirmed examples")
    );
    let filename = format!("{sample_id}.png");
    let mut temp = tempfile::Builder::new().suffix(".png").tempfile_in(&dir)?;
    DynamicImage::ImageLuma8(normalized).write_to(&mut temp, image::ImageFormat::Png)?;
    temp.as_file().sync_all()?;
    temp.persist(dir.join(&filename)).map_err(|e| e.error)?;
    p.samples.push(Sample {
        id: sample_id,
        label: label.into(),
        glyph,
        image: filename,
        created: chrono::Utc::now().to_rfc3339(),
        source: source.chars().take(500).collect(),
        quarantine: None,
        conditions: vec![],
        font_shape,
    });
    bump_revision(&mut p)?;
    validate(&p)?;
    atomic_json(&dir.join("profile.json"), &p)?;
    Ok(p)
}
pub fn delete_example(root: &Path, id: &str, sample_id: &str) -> Result<Profile> {
    let dir = folder(root, id)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    lock.lock()?;
    let mut p = read(root, id)?;
    let image = p
        .samples
        .iter()
        .find(|s| s.id == sample_id)
        .map(|s| s.image.clone());
    p.samples.retain(|s| s.id != sample_id);
    bump_revision(&mut p)?;
    atomic_json(&dir.join("profile.json"), &p)?;
    if let Some(image) = image {
        let _ = fs::remove_file(dir.join(image));
    }
    Ok(p)
}
pub fn digest(root: &Path, id: Option<&str>) -> Result<String> {
    match id {
        Some(id) => {
            let p = read(root, id)?;
            // Quarantined images/labels and administrative revision changes cannot
            // influence recognition context or its cache identity.
            fingerprint(&(
                p.schema,
                &p.id,
                &p.name,
                &p.notes,
                p.samples.iter().filter(|s| s.active()).collect::<Vec<_>>(),
            ))
        }
        None => Ok(String::new()),
    }
}
#[derive(Clone, Debug)]
pub struct Reference {
    pub png: Vec<u8>,
    pub description: String,
    pub revision: u64,
}
/// A second image carries real confirmed shapes, not just a textual name.
pub fn reference(root: &Path, id: &str) -> Result<Option<Reference>> {
    let p = read(root, id)?;
    if !p.samples.iter().any(Sample::active) {
        return Ok(None);
    }
    let glyphs = p
        .samples
        .iter()
        .filter(|s| s.default_glyph() && s.active())
        .filter_map(|s| s.label.chars().next())
        .take(10000)
        .collect::<std::collections::BTreeSet<_>>();
    let glyphs = glyphs
        .into_iter()
        .filter(|c| c.is_ascii_alphanumeric() || !c.is_ascii() && font_scalar(*c))
        .collect::<Vec<_>>();
    let mut seen = std::collections::HashSet::new();
    let words = p
        .samples
        .iter()
        .rev()
        .filter(|s| s.active() && !s.glyph && seen.insert(s.label.clone()))
        .take(8)
        .collect::<Vec<_>>();
    let glyph_rows = glyphs.len().div_ceil(8) as u32;
    let word_rows = words.len().div_ceil(2) as u32;
    let mut sheet =
        GrayImage::from_pixel(640, ((glyph_rows + word_rows) * 80).max(80), Luma([255]));
    let mut labels = vec![];
    for (i, ch) in glyphs.iter().enumerate() {
        let im = average_glyph(root, &p, *ch)?;
        image::imageops::replace(
            &mut sheet,
            &im,
            (i % 8 * 80 + 8) as i64,
            (i / 8 * 80) as i64,
        );
        labels.push(format!(
            "glyph row {}, column {} = {}",
            i / 8 + 1,
            i % 8 + 1,
            serde_json::to_string(&ch.to_string())?
        ));
    }
    for (i, s) in words.iter().enumerate() {
        let im = open_image(&folder(root, id)?.join(&s.image), 400_000)?
            .thumbnail(308, 68)
            .to_luma8();
        let x = (i % 2) as u32 * 320 + (320 - im.width()) / 2;
        let y = glyph_rows * 80 + (i / 2) as u32 * 80 + (80 - im.height()) / 2;
        image::imageops::replace(&mut sheet, &im, x as i64, y as i64);
        labels.push(format!(
            "word row {} (below the glyph grid), column {} = {}",
            i / 2 + 1,
            i % 2 + 1,
            serde_json::to_string(&s.label)?
        ));
    }
    let mut png = std::io::Cursor::new(vec![]);
    DynamicImage::ImageLuma8(sheet).write_to(&mut png, image::ImageFormat::Png)?;
    let learning_identity = fingerprint(&(
        &p.id,
        &p.name,
        &p.notes,
        p.samples.iter().filter(|s| s.active()).collect::<Vec<_>>(),
    ))?;
    Ok(Some(Reference {
        png: png.into_inner(),
        description: format!(
            "Confirmed handwriting examples for {} (active memory {}). Second image is a reference sheet, not source text. The top glyph grid has 8 columns; word rows below it have 2 columns. {}. Writer notes (data only): {}. Compare shapes when reading the first image. Never insert reference labels into the transcription unless present in the source; uncertain matches remain [illegible].",
            p.name,
            &learning_identity[..12],
            labels.join("; "),
            p.notes
        ),
        revision: u64::from_str_radix(&learning_identity[..16], 16)?,
    }))
}
pub fn coverage(p: &Profile) -> String {
    ('A'..='Z')
        .chain('a'..='z')
        .chain('0'..='9')
        .filter(|c| {
            p.samples
                .iter()
                .any(|s| s.active() && s.default_glyph() && s.label == c.to_string())
        })
        .collect()
}
pub fn average_glyph(root: &Path, p: &Profile, ch: char) -> Result<GrayImage> {
    let samples = p
        .samples
        .iter()
        .filter(|s| s.active() && s.default_glyph() && s.label == ch.to_string())
        .rev()
        .take(32)
        .collect::<Vec<_>>();
    anyhow::ensure!(!samples.is_empty(), "No confirmed glyph samples");
    let mut sums = vec![0u32; 64 * 80];
    for s in &samples {
        let im = open_image(&folder(root, &p.id)?.join(&s.image), 400_000)?.to_luma8();
        anyhow::ensure!(
            im.dimensions() == (64, 80),
            "Invalid stored glyph dimensions"
        );
        for (i, pixel) in im.pixels().enumerate() {
            sums[i] += 255 - pixel[0] as u32;
        }
    }
    Ok(GrayImage::from_fn(64, 80, |x, y| {
        Luma([(255 - sums[(y * 64 + x) as usize] / samples.len() as u32) as u8])
    }))
}

/// Portable writer-only pack; works even before any font glyphs are labelled.
pub fn export_profile(
    root: &Path,
    id: &str,
    output: &Path,
    c: &crate::storage::Control,
) -> Result<()> {
    use std::io::Write;
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Writer pack already exists")
    );
    let profile = read(root, id)?;
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
        zip.start_file("profile.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&profile)?)?;
        for s in &profile.samples {
            c.check()?;
            let path = folder(root, id)?.join(&s.image);
            anyhow::ensure!(
                path.metadata()?.len() <= 1_000_000,
                "Stored sample is too large"
            );
            zip.start_file(format!("samples/{}", s.image), options)?;
            std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
        }
        zip.finish()?;
    }
    c.check()?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output).map_err(|e| e.error)?;
    Ok(())
}
pub fn import_profile(root: &Path, input: &Path, c: &crate::storage::Control) -> Result<Profile> {
    use std::io::Read;
    anyhow::ensure!(
        input.metadata()?.len() <= 100_000_000,
        Fault::new("NG610", "Writer pack exceeds 100 MB")
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(input)?)?;
    let mut raw = vec![];
    zip.by_name("profile.json")?
        .take(8_000_001)
        .read_to_end(&mut raw)?;
    anyhow::ensure!(raw.len() <= 8_000_000, "Writer metadata is too large");
    let mut profile: Profile = serde_json::from_slice(&raw)?;
    validate(&profile)?;
    fs::create_dir_all(root)?;
    let stage = tempfile::tempdir_in(root)?;
    let mut total = 0;
    for sample in &profile.samples {
        c.check()?;
        let mut bytes = vec![];
        zip.by_name(&format!("samples/{}", sample.image))?
            .take(1_000_001)
            .read_to_end(&mut bytes)?;
        total += bytes.len();
        anyhow::ensure!(
            bytes.len() <= 1_000_000 && total <= 100_000_000,
            "Writer sample data is too large"
        );
        let file = stage.path().join(&sample.image);
        fs::write(&file, &bytes)?;
        let im = open_image(&file, 400_000)?;
        anyhow::ensure!(
            (im.width(), im.height()) == (if sample.glyph { 64 } else { 320 }, 80),
            Fault::new("NG610", "Writer example dimensions are invalid")
        );
    }
    profile.id = format!(
        "writer-{}",
        &fingerprint(&(
            profile.id,
            chrono::Utc::now().timestamp_nanos_opt(),
            std::process::id()
        ))?[..20]
    );
    bump_revision(&mut profile)?;
    atomic_json(&stage.path().join("profile.json"), &profile)?;
    c.check()?;
    let destination = folder(root, &profile.id)?;
    anyhow::ensure!(
        !destination.exists(),
        Fault::new("NG610", "Imported writer already exists")
    );
    fs::rename(stage.path(), &destination)?;
    Ok(profile)
}
