//! Confirmed source masks and newly built sharing copies. Originals are never altered.
//! Dark-cover observations are proposals, not a claim that hidden writing exists.
use crate::{
    consensus::normalize,
    storage::{Control, Manifest, Project, fingerprint},
    types::*,
    watch::no_links,
};
use image::{DynamicImage, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use zip::{ZipWriter, write::SimpleFileOptions};

pub const MAX_MASKS_PER_PAGE: usize = 128;
pub const READER_NOTICE: &str = "Confirmed opaque source masks must remain [redacted]. Do not infer, reconstruct or guess covered text. Return normalized source boxes for every visible block; ungrounded readings will be withheld.";
pub const EXPORT_NOTICE: &str = "REDACTED EXPORT: confirmed source masks are applied. Private source links, reader evidence, history and correction-import metadata are omitted. Review all remaining visible content before sharing.";
const EXPORT_MARKER: &str = "redacted-export.json";
const MAX_IMAGE_BYTES: u64 = 200_000_000;
const MAX_SHARE_BYTES: u64 = 4_000_000_000;
pub const LIMITS: &str = "Masks require explicit human confirmation. Dark rectangles may be legitimate figures; no hidden text is inferred. All ungrounded readings and whole overlapping blocks/tables are withheld rather than reconstructed. Sharing copies omit private evidence, history, settings and annotation metadata. Known-text leak checks cannot identify unknown personal information or prove that unrelated visible content is safe to share. Review every page of the newly built copy. Originals and their private history remain unchanged; this is not secure erasure.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub page: usize,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    PersonalInformation,
    AccountInformation,
    Address,
    ConfirmedOpaqueCover,
    Other,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MaskSpec {
    pub region: BoxRect,
    pub reason: Reason,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedMask {
    pub id: String,
    pub source: SourceIdentity,
    pub region: BoxRect,
    pub reason: Reason,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationPreview {
    pub source: SourceIdentity,
    pub page_fingerprint: String,
    pub proposals: Vec<MaskSpec>,
    pub token: String,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoverSuggestion {
    pub region: BoxRect,
    pub dark_fraction: f64,
    pub note: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnforcementReport {
    pub withheld_blocks: usize,
    pub ungrounded_blocks: usize,
    pub retained_blocks: usize,
    pub placeholders: usize,
    pub note: String,
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG500", message).into()
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= 20000
        && height <= 20000
        && u64::from(width) * u64::from(height) <= 40_000_000
}
pub fn validate_metadata(masks: &[ConfirmedMask]) -> Result<()> {
    anyhow::ensure!(
        masks.len() <= MAX_MASKS_PER_PAGE,
        fail("Too many confirmed masks")
    );
    let mut ids = BTreeSet::new();
    for mask in masks {
        anyhow::ensure!(
            hash_valid(&mask.id)
                && hash_valid(&mask.source.sha256)
                && (1..=3000).contains(&mask.source.page)
                && dimensions(mask.source.width, mask.source.height)
                && mask.region.valid()
                && ids.insert(&mask.id),
            fail("Invalid or duplicate confirmed mask metadata")
        );
    }
    if let Some(first) = masks.first() {
        anyhow::ensure!(
            masks.iter().all(|mask| mask.source == first.source),
            fail("Confirmed masks disagree about the source identity")
        );
    }
    Ok(())
}
pub fn validate_source(masks: &[ConfirmedMask], identity: &SourceIdentity) -> Result<()> {
    validate_metadata(masks)?;
    anyhow::ensure!(
        masks.iter().all(|mask| &mask.source == identity),
        fail("Confirmed redaction source changed; masks require explicit reconfirmation")
    );
    Ok(())
}
/// Drop free context and writer memory on masked requests. Callers must also
/// omit unfiltered previous-page/evidence strings and use only effective views.
pub fn recognition_settings(settings: &Settings, masks: &[ConfirmedMask]) -> Result<Settings> {
    validate_metadata(masks)?;
    Ok(if masks.is_empty() {
        settings.clone()
    } else {
        private_recognition_settings(settings)
    })
}
pub fn private_recognition_settings(settings: &Settings) -> Settings {
    let mut safe = settings.clone();
    safe.subject.clear();
    safe.assistance = crate::assistance::Options {
        ask_questions: false,
        max_questions: 0,
        ..Default::default()
    };
    safe.writer.use_examples = false;
    safe.writer.id = None;
    safe.writer_reference = None;
    safe.experience.live_preview = false;
    safe.experience.warm_reader = false;
    safe
}
fn read_source(
    project: &Project,
    page: &Page,
    c: &Control,
) -> Result<(SourceIdentity, DynamicImage)> {
    c.check()?;
    page.validate()?;
    no_links(&project.root)?;
    let path = project.image(page.page);
    no_links(&path)?;
    let mut file = fs::File::open(&path)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        fail("Mask source must be a regular file")
    );
    let mut bytes = vec![];
    let mut chunk = [0u8; 65536];
    loop {
        c.check()?;
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        anyhow::ensure!(
            bytes.len() as u64 + count as u64 <= MAX_IMAGE_BYTES,
            fail("Mask source exceeds the image byte limit")
        );
        bytes.extend_from_slice(&chunk[..count]);
    }
    no_links(&path)?;
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    anyhow::ensure!(
        dimensions(width, height),
        fail("Mask source exceeds image dimensions")
    );
    let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20000);
    limits.max_image_height = Some(20000);
    limits.max_alloc = Some(320_000_000);
    reader.limits(limits);
    let decoded = reader.decode()?;
    c.check()?;
    Ok((
        SourceIdentity {
            page: page.page,
            sha256: hex::encode(Sha256::digest(&bytes)),
            width,
            height,
        },
        decoded,
    ))
}
pub fn source_identity(project: &Project, page: &Page, c: &Control) -> Result<SourceIdentity> {
    Ok(read_source(project, page, c)?.0)
}
pub fn recognition_view(
    project: &Project,
    page: &Page,
    c: &Control,
) -> Result<(SourceIdentity, DynamicImage)> {
    let (identity, original) = read_source(project, page, c)?;
    let image = mask_view(&original, &identity, &page.confirmed_redactions, None, c)?;
    Ok((identity, image))
}
/// Visible blocks retain their CANONICAL indices. Never write a compacted
/// effective page back over the canonical review checkpoint.
pub fn visible_blocks(project: &Project, page: &Page, c: &Control) -> Result<Vec<(usize, Block)>> {
    if page.confirmed_redactions.is_empty() {
        return Ok(page.content.blocks.iter().cloned().enumerate().collect());
    }
    let identity = source_identity(project, page, c)?;
    validate_source(&page.confirmed_redactions, &identity)?;
    let mut visible = vec![];
    for (index, block) in page.content.blocks.iter().enumerate() {
        let Some(bounds) = &block.bbox else { continue };
        if page
            .confirmed_redactions
            .iter()
            .any(|mask| intersection(bounds, &mask.region).is_some())
        {
            continue;
        }
        let mut block = block.clone();
        block.alternatives.clear();
        visible.push((index, block));
    }
    Ok(visible)
}
fn current(project: &Project, expected: &Page) -> Result<()> {
    no_links(&project.root)?;
    no_links(&project.page_path(expected.page))?;
    no_links(
        &project
            .root
            .join(format!("page-{:05}.edits.json", expected.page)),
    )?;
    anyhow::ensure!(
        fingerprint(&project.load(expected.page)?)? == fingerprint(expected)?,
        fail("Page changed after the redaction preview")
    );
    Ok(())
}
pub fn preview_confirmation(
    project: &Project,
    expected: &Page,
    proposals: &[MaskSpec],
    c: &Control,
) -> Result<ConfirmationPreview> {
    current(project, expected)?;
    anyhow::ensure!(
        !proposals.is_empty()
            && proposals.len() + expected.confirmed_redactions.len() <= MAX_MASKS_PER_PAGE
            && proposals.iter().all(|p| p.region.valid()),
        fail("Choose bounded valid source mask proposals")
    );
    let source = source_identity(project, expected, c)?;
    validate_source(&expected.confirmed_redactions, &source)?;
    let page_fingerprint = fingerprint(expected)?;
    let token = fingerprint(&(1, &source, &page_fingerprint, proposals))?;
    Ok(ConfirmationPreview {
        source,
        page_fingerprint,
        proposals: proposals.to_vec(),
        token,
        note: LIMITS.into(),
    })
}
/// Returns replacement metadata only. Persist via the ordinary review transaction
/// so explicit add/remove operations remain undoable with the private original.
pub fn confirm(
    project: &Project,
    expected: &Page,
    preview: &ConfirmationPreview,
    token: &str,
    selected: &[usize],
    c: &Control,
) -> Result<Vec<ConfirmedMask>> {
    let fresh = preview_confirmation(project, expected, &preview.proposals, c)?;
    anyhow::ensure!(
        hash_valid(token)
            && token == fresh.token
            && token == preview.token
            && preview.source == fresh.source
            && preview.page_fingerprint == fresh.page_fingerprint
            && !selected.is_empty()
            && selected.len() <= preview.proposals.len(),
        fail("Confirm a fresh exact mask preview and explicit selection")
    );
    let mut masks = expected.confirmed_redactions.clone();
    let mut seen = BTreeSet::new();
    for index in selected {
        c.check()?;
        anyhow::ensure!(seen.insert(index), fail("Duplicate mask selection"));
        let proposal = preview
            .proposals
            .get(*index)
            .ok_or_else(|| fail("Unknown mask selection"))?;
        masks.push(ConfirmedMask {
            id: fingerprint(&(&fresh.token, index))?,
            source: fresh.source.clone(),
            region: proposal.region.clone(),
            reason: proposal.reason,
        });
    }
    validate_metadata(&masks)?;
    Ok(masks)
}
fn intersection(a: &BoxRect, b: &BoxRect) -> Option<BoxRect> {
    let region = BoxRect {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    region.valid().then_some(region)
}
fn local_masks(masks: &[ConfirmedMask], view: &BoxRect) -> Result<Vec<BoxRect>> {
    anyhow::ensure!(view.valid(), fail("Invalid redaction source view"));
    Ok(masks
        .iter()
        .filter_map(|mask| {
            intersection(&mask.region, view).map(|region| BoxRect {
                left: ((region.left - view.left) / (view.right - view.left)).clamp(0., 1.),
                top: ((region.top - view.top) / (view.bottom - view.top)).clamp(0., 1.),
                right: ((region.right - view.left) / (view.right - view.left)).clamp(0., 1.),
                bottom: ((region.bottom - view.top) / (view.bottom - view.top)).clamp(0., 1.),
            })
        })
        .collect())
}
/// Opaque RGB output also flattens transparency, removing hidden RGB channels.
pub fn mask_view(
    image: &DynamicImage,
    identity: &SourceIdentity,
    masks: &[ConfirmedMask],
    source_view: Option<&BoxRect>,
    c: &Control,
) -> Result<DynamicImage> {
    c.check()?;
    anyhow::ensure!(
        dimensions(image.width(), image.height()),
        fail("Invalid redaction view dimensions")
    );
    validate_source(masks, identity)?;
    if source_view.is_none() {
        anyhow::ensure!(
            image.width() == identity.width && image.height() == identity.height,
            fail("Full mask view dimensions changed")
        );
    }
    let view = source_view.cloned().unwrap_or_else(BoxRect::full);
    let regions = local_masks(masks, &view)?;
    let rgba = image.to_rgba8();
    let mut output = RgbImage::new(image.width(), image.height());
    for y in 0..image.height() {
        c.check()?;
        for x in 0..image.width() {
            let p = rgba[(x, y)];
            let alpha = u32::from(p[3]);
            output[(x, y)] = Rgb([0, 1, 2]
                .map(|i| ((u32::from(p[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8));
        }
    }
    for region in regions {
        let (left, top, right, bottom) = region.pixels(image.width(), image.height());
        for y in top..bottom {
            c.check()?;
            for x in left..right {
                output[(x, y)] = Rgb([0, 0, 0]);
            }
        }
    }
    Ok(DynamicImage::ImageRgb8(output))
}
/// Source-view boxes and returned block boxes use the reader's normalized view.
/// Readers without usable locations cannot safely retain text on a masked page.
pub fn enforce_view(
    content: &PageContent,
    identity: &SourceIdentity,
    masks: &[ConfirmedMask],
    source_view: Option<&BoxRect>,
    c: &Control,
) -> Result<(PageContent, EnforcementReport)> {
    c.check()?;
    content.validate()?;
    validate_source(masks, identity)?;
    if masks.is_empty() {
        return Ok((content.clone(), EnforcementReport::default()));
    }
    let regions = local_masks(masks, &source_view.cloned().unwrap_or_else(BoxRect::full))?;
    let mut report = EnforcementReport {
        note: LIMITS.into(),
        ..Default::default()
    };
    let mut blocks = vec![];
    for block in &content.blocks {
        c.check()?;
        let Some(bounds) = &block.bbox else {
            report.ungrounded_blocks += 1;
            report.withheld_blocks += 1;
            continue;
        };
        if regions
            .iter()
            .any(|mask| intersection(bounds, mask).is_some())
        {
            report.withheld_blocks += 1;
            continue;
        }
        let mut visible = block.clone();
        visible.alternatives.clear();
        blocks.push(visible);
        report.retained_blocks += 1;
    }
    for region in regions {
        blocks.push(Block {
            bbox: Some(region),
            ..Block::text("[redacted]")
        });
        report.placeholders += 1;
    }
    let result = PageContent {
        blocks,
        warnings: vec!["CONFIRMED REDACTIONS: opaque source masks are retained as [redacted]. Ungrounded readings and whole overlapping blocks/tables are withheld. Review visible content before sharing.".into()],
        summary: String::new(),
    };
    result.validate()?;
    Ok((result, report))
}
/// Both private original and mask-bound identity are verified before obtaining
/// an effective view. Call before exporting, searching, clipboard or rereading.
pub fn effective_page(
    project: &Project,
    page: &Page,
    c: &Control,
) -> Result<(Page, DynamicImage, EnforcementReport)> {
    let (identity, original) = read_source(project, page, c)?;
    let image = mask_view(&original, &identity, &page.confirmed_redactions, None, c)?;
    let (content, report) = enforce_view(
        &page.content,
        &identity,
        &page.confirmed_redactions,
        None,
        c,
    )?;
    let mut effective = page.clone();
    effective.content = content;
    if !page.confirmed_redactions.is_empty() {
        effective.candidates.clear();
        effective.manual_layers.clear();
        effective.source_warnings.clear();
    }
    Ok((effective, image, report))
}

pub fn has_masks(project: &Project, c: &Control) -> Result<bool> {
    for number in &project.manifest.pages {
        c.check()?;
        let base = project.page_path(*number);
        let edits = project.root.join(format!("page-{number:05}.edits.json"));
        if fs::symlink_metadata(&base)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            && fs::symlink_metadata(&edits)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        if !load_page(project, *number)?.confirmed_redactions.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn is_export_projection(project: &Project) -> bool {
    project.root.join(EXPORT_MARKER).is_file()
}

/// An export-only allowlist. The private project, history and confirmed masks
/// remain unchanged. Never publish this temporary project as a reusable workspace.
pub struct ExportProjection {
    project: Project,
    _directory: tempfile::TempDir,
    bindings: Vec<(usize, String, SourceIdentity)>,
    manifest_digest: String,
}
impl ExportProjection {
    pub fn project(&self) -> &Project {
        &self.project
    }
    pub fn new(original: &Project, c: &Control) -> Result<Option<Self>> {
        if !has_masks(original, c)? {
            return Ok(None);
        }
        let directory = tempfile::Builder::new()
            .prefix("notegen-redacted-export-")
            .tempdir()?;
        let settings = Settings {
            server: "runtime/llama-server.exe".into(),
            tesseract: "tesseract".into(),
            ..Default::default()
        };
        let mut project = Project::open(
            directory.path().join("project"),
            Manifest {
                schema: 2,
                input_sha256: fingerprint(&(1, "redacted export"))?,
                fingerprint: fingerprint(&(1, "public export projection"))?,
                pages: original.manifest.pages.clone(),
                models: vec![],
                settings,
            },
        )?;
        let mut bindings = vec![];
        let mut fragments = BTreeSet::new();
        let mut source_hashes = vec![];
        let mut bytes = 0u64;
        for number in &original.manifest.pages {
            c.check()?;
            let private = load_page(original, *number)?;
            let (identity, image) = read_source(original, &private, c)?;
            let masked = mask_view(&image, &identity, &private.confirmed_redactions, None, c)?;
            let (mut content, _) = enforce_view(
                &private.content,
                &identity,
                &private.confirmed_redactions,
                None,
                c,
            )?;
            fragments.extend(hidden_fragments(&private, c)?);
            anyhow::ensure!(
                fragments.len() <= 10000
                    && fragments.iter().map(String::len).sum::<usize>() <= 8_000_000,
                fail("Export known-private text exceeds the bounded leak-check budget")
            );
            content.summary.clear();
            content.warnings = vec![EXPORT_NOTICE.into()];
            for block in &mut content.blocks {
                block.alternatives.clear();
            }
            let public = Page {
                page: *number,
                image: format!("page-{number:05}.png"),
                content,
                ..Default::default()
            };
            masked.save_with_format(project.image(*number), image::ImageFormat::Png)?;
            project.save(&public)?;
            source_hashes.push((
                *number,
                crate::storage::hash_file(&project.image(*number), c)?,
            ));
            bytes = bytes
                .saturating_add(fs::metadata(project.image(*number))?.len())
                .saturating_add(fs::metadata(project.page_path(*number))?.len());
            anyhow::ensure!(
                bytes <= MAX_SHARE_BYTES,
                fail("Redacted export projection exceeds 4 GB; export fewer pages")
            );
            bindings.push((*number, fingerprint(&private)?, identity));
        }
        let matcher =
            regex::RegexSetBuilder::new(fragments.iter().map(|value| regex::escape(value)))
                .size_limit(32_000_000)
                .dfa_size_limit(8_000_000)
                .build()
                .map_err(|_| {
                    fail("Known-private export text exceeds the bounded matcher budget")
                })?;
        for number in &project.manifest.pages {
            check_public_fields(&project.load(*number)?.content, &matcher, c)?;
        }
        project.manifest.input_sha256 = fingerprint(&(1, "masked source images", source_hashes))?;
        project.manifest.fingerprint =
            fingerprint(&(1, "redacted export", &project.manifest.input_sha256))?;
        crate::storage::atomic_json(&project.root.join("project.json"), &project.manifest)?;
        crate::storage::atomic_json(
            &project.root.join(EXPORT_MARKER),
            &serde_json::json!({"schema":1,"notice":EXPORT_NOTICE}),
        )?;
        if original.root.join("input-invalidated.json").is_file() {
            crate::storage::atomic_json(
                &project.root.join("input-invalidated.json"),
                &serde_json::json!({"reason":"Source invalidated; verify preserved visible pages."}),
            )?;
        }
        let projection = Self {
            project,
            _directory: directory,
            bindings,
            manifest_digest: fingerprint(&original.manifest)?,
        };
        projection.verify_unchanged(original, c)?;
        Ok(Some(projection))
    }
    pub fn verify_unchanged(&self, original: &Project, c: &Control) -> Result<()> {
        no_links(&original.root.join("project.json"))?;
        let manifest: Manifest =
            crate::storage::read_json(&original.root.join("project.json"), 1_000_000)?;
        anyhow::ensure!(
            fingerprint(&manifest)? == self.manifest_digest,
            fail("Project identity changed while exporting")
        );
        for (number, digest, identity) in &self.bindings {
            c.check()?;
            let page = load_page(original, *number)?;
            anyhow::ensure!(
                fingerprint(&page)? == *digest && source_identity(original, &page, c)? == *identity,
                fail("Private source or masks changed while exporting; export again")
            );
        }
        Ok(())
    }
}
/// The caller must guard the destination against the ORIGINAL project first.
/// A masked output is published only after source/mask identities are rechecked.
pub(crate) fn export_projected<T>(
    original: &Project,
    target: &Path,
    c: &Control,
    write: impl FnOnce(&Project, &Path) -> Result<T>,
) -> Result<T> {
    c.check()?;
    anyhow::ensure!(!target.exists(), fail("Output already exists"));
    let parent = crate::exports::output_parent(original, target)?;
    fs::create_dir_all(&parent)?;
    let parent = crate::exports::output_parent(original, &parent.join("export"))?;
    let target = parent.join(
        target
            .file_name()
            .ok_or_else(|| fail("Choose an output filename"))?,
    );
    let Some(projection) = ExportProjection::new(original, c)? else {
        return write(original, &target);
    };
    let staging = tempfile::Builder::new()
        .prefix(".notegen-redacted-output-")
        .tempdir_in(&parent)?;
    let staged = staging.path().join(
        target
            .file_name()
            .ok_or_else(|| fail("Choose an output filename"))?,
    );
    let result = write(projection.project(), &staged)?;
    projection.verify_unchanged(original, c)?;
    c.check()?;
    crate::exports::output_parent(original, &target)?;
    tempfile::TempPath::try_from_path(staged)?
        .persist_noclobber(&target)
        .map_err(|error| error.error)?;
    Ok(result)
}

/// Conservative dark, nearly solid connected rectangles only. A figure, chart
/// bar or decoration can meet these conditions: nothing is confirmed or masked.
pub fn suggest_opaque_covers(image: &DynamicImage, c: &Control) -> Result<Vec<CoverSuggestion>> {
    c.check()?;
    anyhow::ensure!(
        dimensions(image.width(), image.height()),
        fail("Invalid cover-proposal image dimensions")
    );
    let small = image.thumbnail(512, 512).to_rgba8();
    let (width, height) = (small.width() as usize, small.height() as usize);
    let dark = small
        .pixels()
        .map(|p| p[3] >= 250 && p[0] <= 32 && p[1] <= 32 && p[2] <= 32)
        .collect::<Vec<_>>();
    let mut visited = vec![false; dark.len()];
    let mut proposals = vec![];
    for start in 0..dark.len() {
        if start % width == 0 {
            c.check()?;
        }
        if !dark[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        let mut queue = vec![start];
        let mut cursor = 0;
        let (mut left, mut right, mut top, mut bottom) = (width, 0, height, 0);
        while cursor < queue.len() {
            if cursor % 4096 == 0 {
                c.check()?;
            }
            let index = queue[cursor];
            cursor += 1;
            let (x, y) = (index % width, index / width);
            left = left.min(x);
            right = right.max(x + 1);
            top = top.min(y);
            bottom = bottom.max(y + 1);
            for neighbor in [
                (x > 0).then(|| index - 1),
                (x + 1 < width).then_some(index + 1),
                (y > 0).then(|| index - width),
                (y + 1 < height).then_some(index + width),
            ]
            .into_iter()
            .flatten()
            {
                if dark[neighbor] && !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push(neighbor);
                }
            }
        }
        let area = (right - left) * (bottom - top);
        let density = queue.len() as f64 / area as f64;
        if right - left >= 12
            && bottom - top >= 3
            && right - left >= (bottom - top) * 2
            && area <= width * height / 2
            && density >= 0.98
        {
            proposals.push(CoverSuggestion {
                region: BoxRect {
                    left: left as f32 / width as f32,
                    top: top as f32 / height as f32,
                    right: right as f32 / width as f32,
                    bottom: bottom as f32 / height as f32,
                },
                dark_fraction: density,
                note: "Possible opaque dark rectangle; may be a legitimate figure or decoration. Compare with the scan and explicitly confirm a mask. No hidden text was inferred.".into(),
            });
        }
        if proposals.len() == 64 {
            proposals.last_mut().unwrap().note.push_str(" Proposal limit reached: only the first 64 candidates are listed; inspect the remaining source manually.");
            break;
        }
    }
    Ok(proposals)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharePagePreview {
    pub source: SourceIdentity,
    pub page_fingerprint: String,
    pub masks: usize,
    pub withheld_blocks: usize,
    pub ungrounded_blocks: usize,
    pub visible_blocks: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharePreview {
    pub pages: Vec<SharePagePreview>,
    pub token: String,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareReceipt {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub pages: usize,
    pub masks: usize,
    pub withheld_blocks: usize,
    pub checked_private_fragments: usize,
    pub note: String,
}
#[derive(Serialize)]
struct PublicReport {
    schema: u32,
    kind: &'static str,
    pages: usize,
    masks: usize,
    withheld_blocks: usize,
    checked_private_fragments: usize,
    omitted: &'static str,
    limits: &'static str,
}
fn load_page(project: &Project, number: usize) -> Result<Page> {
    no_links(&project.root)?;
    no_links(&project.page_path(number))?;
    no_links(&project.root.join(format!("page-{number:05}.edits.json")))?;
    project.load(number)
}
pub fn preview_share(
    project: &Project,
    selected_pages: &[usize],
    c: &Control,
) -> Result<SharePreview> {
    c.check()?;
    let selected = selected_pages.iter().copied().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        !selected.is_empty()
            && selected.len() == selected_pages.len()
            && selected.len() <= 3000
            && selected.iter().all(|n| project.manifest.pages.contains(n)),
        fail("Choose distinct existing pages for the sharing copy")
    );
    let mut pages = vec![];
    let mut bytes = 0;
    for number in project
        .manifest
        .pages
        .iter()
        .filter(|n| selected.contains(n))
    {
        c.check()?;
        let page = load_page(project, *number)?;
        let (identity, _) = read_source(project, &page, c)?;
        bytes += fs::metadata(project.image(*number))?.len();
        anyhow::ensure!(
            bytes <= MAX_SHARE_BYTES,
            fail("Sharing sources exceed the 4 GB budget")
        );
        let (_, report) = enforce_view(
            &page.content,
            &identity,
            &page.confirmed_redactions,
            None,
            c,
        )?;
        pages.push(SharePagePreview {
            source: identity,
            page_fingerprint: fingerprint(&page)?,
            masks: page.confirmed_redactions.len(),
            withheld_blocks: report.withheld_blocks,
            ungrounded_blocks: report.ungrounded_blocks,
            visible_blocks: if page.confirmed_redactions.is_empty() {
                page.content.blocks.len()
            } else {
                report.retained_blocks
            },
        });
    }
    anyhow::ensure!(
        pages.len() == selected.len(),
        fail("Project page list changed or is duplicated")
    );
    let token = fingerprint(&(1, fingerprint(&project.manifest)?, &pages))?;
    Ok(SharePreview {
        pages,
        token,
        note: LIMITS.into(),
    })
}
fn fields(block: &Block) -> impl Iterator<Item = &str> {
    [
        block.text.as_str(),
        block.description.as_str(),
        block.interpretation.as_str(),
    ]
    .into_iter()
    .chain(block.rows.iter().flatten().map(String::as_str))
    .chain(block.alternatives.iter().map(String::as_str))
}
fn hidden_fragments(page: &Page, c: &Control) -> Result<BTreeSet<String>> {
    let mut fragments = BTreeSet::new();
    let mut bytes = 0;
    for block in &page.content.blocks {
        c.check()?;
        if !page.confirmed_redactions.is_empty()
            && block.bbox.as_ref().is_none_or(|bounds| {
                page.confirmed_redactions
                    .iter()
                    .any(|mask| intersection(bounds, &mask.region).is_some())
            })
        {
            for value in fields(block) {
                let value = normalize(value);
                if !value.is_empty() && value != "[redacted]" && fragments.insert(value.clone()) {
                    bytes += value.len();
                    anyhow::ensure!(
                        fragments.len() <= 10000 && bytes <= 8_000_000,
                        fail("Known-private text exceeds the bounded leak-check budget")
                    );
                }
            }
        }
    }
    Ok(fragments)
}
fn check_public_fields(
    content: &PageContent,
    fragments: &regex::RegexSet,
    c: &Control,
) -> Result<()> {
    // Check user-derived strings of every length. Fixed application boilerplate
    // is not evidence that a short secret such as "1" was copied from the scan.
    for block in &content.blocks {
        c.check()?;
        for value in fields(block) {
            let value = normalize(value);
            if value == "[redacted]" {
                continue;
            }
            anyhow::ensure!(
                !fragments.is_match(&value),
                fail(
                    "Sharing leak check found withheld text in visible content; review the source masks and public reading"
                )
            );
        }
    }
    Ok(())
}
fn safe_target(project: &Project, target: &Path) -> Result<PathBuf> {
    no_links(&project.root)?;
    no_links(target)?;
    let target = std::path::absolute(target)?;
    let parent = target
        .parent()
        .ok_or_else(|| fail("Choose a sharing ZIP filename"))?
        .canonicalize()?;
    anyhow::ensure!(
        parent.is_dir() && !parent.starts_with(project.root.canonicalize()?) && !target.exists(),
        fail("Sharing output must be a new file outside the private project")
    );
    Ok(parent.join(
        target
            .file_name()
            .ok_or_else(|| fail("Choose a sharing ZIP filename"))?,
    ))
}
/// Build a fresh allowlisted public project; never copy a private project tree.
/// Publication is a final no-clobber rename after rechecking the complete preview.
pub fn export_share(
    project: &Project,
    preview: &SharePreview,
    token: &str,
    target: &Path,
    c: &Control,
) -> Result<ShareReceipt> {
    c.check()?;
    let selected = preview
        .pages
        .iter()
        .map(|p| p.source.page)
        .collect::<Vec<_>>();
    let fresh = preview_share(project, &selected, c)?;
    anyhow::ensure!(
        hash_valid(token) && token == preview.token && token == fresh.token,
        fail("Sharing preview is stale or was not explicitly confirmed")
    );
    // Do not trust counts or sources from a caller-edited preview.
    anyhow::ensure!(
        fingerprint(&preview.pages)? == fingerprint(&fresh.pages)?,
        fail("Sharing preview metadata changed")
    );
    let target = safe_target(project, target)?;
    let parent = target.parent().unwrap();
    let staging = tempfile::Builder::new()
        .prefix(".notegen-share-")
        .tempdir_in(parent)?;
    let public_root = staging.path().join("project");
    let mut settings = Settings {
        server: "runtime/llama-server.exe".into(),
        tesseract: "tesseract".into(),
        ..Default::default()
    };
    settings.assistance.ask_questions = false;
    settings.assistance.web_lookup = false;
    settings.experience.check_updates = false;
    settings.experience.live_preview = false;
    settings.writer.use_examples = false;
    let public = Project::open(
        public_root.clone(),
        Manifest {
            schema: 2,
            input_sha256: fingerprint(&(1, "masked sharing source", &fresh.token))?,
            fingerprint: fingerprint(&(1, "sanitized sharing copy", &fresh.token))?,
            pages: selected.clone(),
            models: vec![],
            settings,
        },
    )?;
    let mut all_fragments = BTreeSet::new();
    let mut entries = vec![(
        "project/project.json".to_string(),
        public_root.join("project.json"),
    )];
    let mut staged_bytes = fs::metadata(public_root.join("project.json"))?.len();
    for item in &fresh.pages {
        c.check()?;
        let page = load_page(project, item.source.page)?;
        anyhow::ensure!(
            fingerprint(&page)? == item.page_fingerprint,
            fail("Page changed while building sharing copy")
        );
        let (identity, original) = read_source(project, &page, c)?;
        anyhow::ensure!(
            identity == item.source,
            fail("Source changed while building sharing copy")
        );
        let fragments = hidden_fragments(&page, c)?;
        all_fragments.extend(fragments);
        anyhow::ensure!(
            all_fragments.len() <= 10000
                && all_fragments.iter().map(String::len).sum::<usize>() <= 8_000_000,
            fail("Sharing leak-check text exceeds the document budget")
        );
        let image = mask_view(&original, &identity, &page.confirmed_redactions, None, c)?;
        let (content, _) = enforce_view(
            &page.content,
            &identity,
            &page.confirmed_redactions,
            None,
            c,
        )?;
        let number = page.page;
        let image_path = public.image(number);
        image.save_with_format(&image_path, image::ImageFormat::Png)?;
        c.check()?;
        let mut shared = Page {
            page: number,
            image: format!("page-{number:05}.png"),
            content,
            ..Default::default()
        };
        // Keep masks enforceable for future rereads of the newly masked image,
        // without retaining a hash or metadata of the private source.
        let identity = source_identity(&public, &shared, c)?;
        shared.confirmed_redactions = page
            .confirmed_redactions
            .iter()
            .enumerate()
            .map(|(index, mask)| {
                Ok(ConfirmedMask {
                    id: fingerprint(&(&identity, index, &mask.region, mask.reason))?,
                    source: identity.clone(),
                    region: mask.region.clone(),
                    reason: mask.reason,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        public.save(&shared)?;
        staged_bytes = staged_bytes
            .checked_add(fs::metadata(&image_path)?.len())
            .and_then(|n| n.checked_add(fs::metadata(public.page_path(number)).ok()?.len()))
            .ok_or_else(|| fail("Sharing staging size cannot be measured"))?;
        anyhow::ensure!(
            staged_bytes <= MAX_SHARE_BYTES,
            fail("Sharing staging exceeds the 4 GB budget")
        );
        entries.push((
            format!("project/page-{number:05}.json"),
            public.page_path(number),
        ));
        entries.push((format!("project/page-{number:05}.png"), image_path));
    }
    let fragments = regex::RegexSetBuilder::new(all_fragments.iter().map(|s| regex::escape(s)))
        .size_limit(32_000_000)
        .dfa_size_limit(8_000_000)
        .build()
        .map_err(|_| fail("Known-private text exceeds the bounded leak-matcher budget"))?;
    for number in &selected {
        c.check()?;
        check_public_fields(&public.load(*number)?.content, &fragments, c)?;
    }
    let word_path = staging.path().join("shared.docx");
    crate::word::export(&public, &word_path, c)?;
    entries.push(("shared.docx".into(), word_path));
    let masks = fresh.pages.iter().map(|p| p.masks).sum();
    let withheld_blocks = fresh.pages.iter().map(|p| p.withheld_blocks).sum();
    let report = PublicReport {
        schema: 1,
        kind: "sanitized sharing copy",
        pages: selected.len(),
        masks,
        withheld_blocks,
        checked_private_fragments: all_fragments.len(),
        omitted: "Original input files, original image metadata, original hashes/paths, reader evidence/raw replies, revision history, manual layers, writer profiles, guidance, public lookup context, private settings and credentials are not included.",
        limits: LIMITS,
    };
    let report_path = staging.path().join("sharing-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    entries.push(("sharing-report.json".into(), report_path));
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = ZipWriter::new(output.as_file_mut());
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut written = 0u64;
        for (name, path) in entries {
            c.check()?;
            let mut input = fs::File::open(path)?;
            written = written
                .checked_add(input.metadata()?.len())
                .ok_or_else(|| fail("Sharing size overflow"))?;
            anyhow::ensure!(
                written <= MAX_SHARE_BYTES,
                fail("Sharing output exceeds the 4 GB budget")
            );
            zip.start_file(name, options)?;
            let mut chunk = [0u8; 65536];
            loop {
                c.check()?;
                let count = input.read(&mut chunk)?;
                if count == 0 {
                    break;
                }
                zip.write_all(&chunk[..count])?;
            }
        }
        zip.finish()?;
    }
    output.as_file().sync_all()?;
    anyhow::ensure!(
        preview_share(project, &selected, c)?.token == fresh.token,
        fail("Private project changed while building sharing copy")
    );
    safe_target(project, &target)?;
    let bytes = output.as_file().metadata()?.len();
    let sha256 = crate::storage::hash_file(output.path(), c)?;
    c.check()?;
    output.persist_noclobber(&target).map_err(|e| e.error)?;
    Ok(ShareReceipt {
        path: target,
        bytes,
        sha256,
        pages: selected.len(),
        masks,
        withheld_blocks,
        checked_private_fragments: all_fragments.len(),
        note: LIMITS.into(),
    })
}
