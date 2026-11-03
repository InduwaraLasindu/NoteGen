//! Reversible active order over immutable, hash-bound source slots.
use crate::{
    migrations,
    redactions::{self, SourceIdentity},
    storage::{Control, Manifest, Project, fingerprint, read_json},
    types::*,
};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io::Cursor, path::PathBuf};
const CATALOG: &str = "source-catalog.json";
fn fail(message: impl Into<String>) -> anyhow::Error {
    Fault::new("NG500", message).into()
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    Reorder { pages: Vec<usize> },
    Remove { pages: Vec<usize> },
    Insert { image: PathBuf, position: usize },
    Split { page: usize, axis: Axis, at: f32 },
    Merge { pages: Vec<usize>, axis: Axis },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePart {
    pub source_id: String,
    pub parent_region: BoxRect,
    pub region: BoxRect,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRecord {
    pub id: String,
    pub slot: usize,
    pub raster: SourceIdentity,
    pub assignment: String,
    pub original_input_sha256: Option<String>,
    pub original_page: Option<usize>,
    pub original_file: Option<PathBuf>,
    pub parts: Vec<SourcePart>,
    pub lock_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutState {
    pub revision: u64,
    pub order: Vec<usize>,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: u32,
    pub review_anchor: String,
    pub base_identity: String,
    pub sources: Vec<SourceRecord>,
    pub states: Vec<LayoutState>,
    pub cursor: usize,
    pub next_revision: u64,
}
#[derive(Clone, Serialize)]
pub struct LayoutPreview {
    pub operation: Operation,
    pub token: String,
    pub order: Vec<usize>,
    pub created: Vec<SourceRecord>,
    pub withheld_blocks: usize,
    pub note: String,
    #[serde(skip)]
    plan: migrations::Plan,
    #[serde(skip)]
    bindings: Vec<(usize, String, SourceIdentity)>,
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn base(m: &Manifest) -> Result<String> {
    fingerprint(&(m.schema, &m.input_sha256, &m.fingerprint, &m.models))
}
fn valid(c: &Catalog, p: &Project) -> Result<()> {
    anyhow::ensure!(
        c.schema == 1
            && hash_valid(&c.review_anchor)
            && c.base_identity == base(&p.manifest)?
            && !c.sources.is_empty()
            && c.sources.len() <= 3000
            && !c.states.is_empty()
            && c.states.len() <= 256
            && c.cursor < c.states.len(),
        fail("Invalid or stale source catalog")
    );
    let mut slots = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for source in &c.sources {
        crate::geometry::validate_source(&source.raster)?;
        anyhow::ensure!(
            hash_valid(&source.id)
                && source.slot == source.raster.page
                && slots.insert(source.slot)
                && ids.insert(source.id.clone())
                && source.parts.len() <= 16
                && source.assignment.len() <= 100
                && source
                    .original_input_sha256
                    .as_deref()
                    .is_none_or(hash_valid)
                && source.lock_id.as_deref().is_none_or(hash_valid),
            fail("Invalid source catalog record")
        );
    }
    for (position, source) in c.sources.iter().enumerate() {
        anyhow::ensure!(
            source.parts.iter().all(|part| c.sources[..position]
                .iter()
                .any(|parent| parent.id == part.source_id)
                && part.parent_region.valid()
                && part.region.valid()),
            fail("Invalid parent source mapping")
        );
    }
    let mut revisions = BTreeSet::new();
    for state in &c.states {
        anyhow::ensure!(
            state.revision < c.next_revision
                && revisions.insert(state.revision)
                && !state.order.is_empty()
                && state.order.len() <= 3000
                && state.order.iter().all(|n| slots.contains(n))
                && state.order.iter().collect::<BTreeSet<_>>().len() == state.order.len()
                && state.label.len() <= 200,
            fail("Invalid active page order")
        );
    }
    anyhow::ensure!(
        c.states[c.cursor].order == p.manifest.pages,
        fail("Project page order changed; reopen the project")
    );
    Ok(())
}
fn catalog_bytes(catalog: &Catalog) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(catalog)?;
    anyhow::ensure!(
        bytes.len() <= 16_000_000,
        fail(
            "Source catalog exceeds its 16 MB history budget; retain this project and create a private separate revision"
        )
    );
    Ok(bytes)
}
pub fn catalog(p: &Project) -> Result<Option<Catalog>> {
    migrations::ensure_readable(&p.root)?;
    let path = p.root.join(CATALOG);
    crate::watch::no_links(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let c: Catalog = read_json(&path, 16_000_000)?;
    valid(&c, p)?;
    Ok(Some(c))
}
pub fn known_pages(p: &Project) -> Result<BTreeSet<usize>> {
    Ok(catalog(p)?.map_or_else(
        || p.manifest.pages.iter().copied().collect(),
        |c| c.sources.iter().map(|s| s.slot).collect(),
    ))
}
pub fn review_anchor(p: &Project) -> Result<String> {
    Ok(catalog(p)?.map_or(fingerprint(&p.manifest)?, |c| c.review_anchor))
}
fn lock_id(p: &Project, c: &Control) -> Result<Option<String>> {
    let path = p.root.join("model-lock.json");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(crate::project_lock::read(&path, c)?.id))
}
fn bind_selection(
    plan: &mut migrations::Plan,
    p: &Project,
    expected: &Option<String>,
    c: &Control,
) -> Result<()> {
    let bytes = migrations::snapshot(&p.root.join("model-lock.json"), c)?;
    let actual = if let Some(bytes) = &bytes {
        anyhow::ensure!(
            bytes.len() <= 4_000_000,
            fail("Project lock exceeds its byte limit")
        );
        let lock: crate::project_lock::Lockfile = serde_json::from_slice(bytes)?;
        crate::project_lock::validate(&lock)?;
        anyhow::ensure!(
            lock.input_sha256 == p.manifest.input_sha256,
            fail("Selected model lock belongs to another source")
        );
        Some(lock.id)
    } else {
        None
    };
    anyhow::ensure!(
        &actual == expected,
        fail("Selected model lock changed during preview")
    );
    migrations::bind_guard(
        plan,
        "model-lock.json",
        bytes.as_deref().map(migrations::snapshot_identity),
    )
}
fn initial(p: &Project, c: &Control) -> Result<Catalog> {
    anyhow::ensure!(
        catalog(p)?.is_none(),
        fail("Source identities are already enabled")
    );
    let anchor = fingerprint(&p.manifest)?;
    let pin = lock_id(p, c)?;
    let mut sources = vec![];
    // Explicit assignment now: no claim to recover an old original UUID/page ordinal.
    for n in &p.manifest.pages {
        c.check()?;
        let page = p.load(*n)?;
        let raster = redactions::source_identity(p, &page, c)?;
        redactions::validate_source(&page.confirmed_redactions, &raster)?;
        sources.push(SourceRecord {
            id: fingerprint(&(1, "assigned source", &anchor, n, &raster))?,
            slot: *n,
            raster,
            assignment: "Explicit legacy source assignment".into(),
            original_input_sha256: hash_valid(&p.manifest.input_sha256)
                .then(|| p.manifest.input_sha256.clone()),
            original_page: None,
            original_file: None,
            parts: vec![],
            lock_id: pin.clone(),
        });
    }
    let result = Catalog {
        schema: 1,
        review_anchor: anchor,
        base_identity: base(&p.manifest)?,
        sources,
        states: vec![LayoutState {
            revision: 0,
            order: p.manifest.pages.clone(),
            label: "Original saved page order".into(),
        }],
        cursor: 0,
        next_revision: 1,
    };
    valid(&result, p)?;
    Ok(result)
}
pub fn migration_plan(p: &Project, c: &Control) -> Result<migrations::Plan> {
    p.ensure_current()?;
    let original_manifest = migrations::snapshot(&p.root.join("project.json"), c)?
        .ok_or_else(|| fail("Project manifest missing"))?;
    let canonical = Project {
        root: p.root.clone(),
        manifest: serde_json::from_slice(&original_manifest)?,
    };
    let catalog = initial(&canonical, c)?;
    let mut plan = migrations::plan(
        &p.root,
        "Assign stable source identities",
        vec![
            ("project.json".into(), Some(original_manifest.clone())),
            (CATALOG.into(), Some(catalog_bytes(&catalog)?)),
        ],
        c,
    )?;
    anyhow::ensure!(
        plan.files[1].before.is_none()
            && plan.files[0].before.as_ref()
                == Some(&migrations::snapshot_identity(&original_manifest)),
        fail("Source catalog appeared during preview")
    );
    let mut bindings = vec![];
    for source in &catalog.sources {
        let page = canonical.load(source.slot)?;
        bindings.push((source.slot, fingerprint(&page)?, source.raster.clone()));
    }
    migrations::bind_pages(&mut plan, bindings)?;
    bind_selection(&mut plan, &canonical, &catalog.sources[0].lock_id, c)?;
    Ok(plan)
}
/// New conversions know their rendered input ordinals. Legacy assignment stays
/// explicit and never fills these fields by guessing from a filename.
pub(crate) fn initialize_new(p: &Project, input: &std::path::Path, c: &Control) -> Result<()> {
    p.ensure_current()?;
    let original_manifest = migrations::snapshot(&p.root.join("project.json"), c)?
        .ok_or_else(|| fail("Project manifest missing"))?;
    let canonical = Project {
        root: p.root.clone(),
        manifest: serde_json::from_slice(&original_manifest)?,
    };
    let mut catalog = initial(&canonical, c)?;
    for record in &mut catalog.sources {
        record.assignment = "Recorded input rasterization".into();
        record.original_page = Some(record.slot);
        record.original_file = Some(input.canonicalize()?);
    }
    let mut plan = migrations::plan(
        &p.root,
        "Record new project source identities",
        vec![
            ("project.json".into(), Some(original_manifest.clone())),
            (CATALOG.into(), Some(catalog_bytes(&catalog)?)),
        ],
        c,
    )?;
    anyhow::ensure!(
        plan.files[1].before.is_none()
            && plan.files[0].before.as_ref()
                == Some(&migrations::snapshot_identity(&original_manifest)),
        fail("New project source catalog already appeared")
    );
    let mut bindings = vec![];
    for source in &catalog.sources {
        let page = p.load(source.slot)?;
        bindings.push((source.slot, fingerprint(&page)?, source.raster.clone()));
    }
    migrations::bind_pages(&mut plan, bindings)?;
    bind_selection(&mut plan, p, &catalog.sources[0].lock_id, c)?;
    migrations::apply(&plan, &plan.token, c)?;
    Ok(())
}
fn next_slot(c: &Catalog, p: &Project) -> Result<usize> {
    (1..=3000).find(|n|!c.sources.iter().any(|s|s.slot==*n)&&[p.image(*n),p.page_path(*n),p.root.join(format!("page-{n:05}.edits.json"))].iter().all(|path|fs::symlink_metadata(path).is_err_and(|e|e.kind()==std::io::ErrorKind::NotFound))).ok_or_else(||fail("All 3,000 immutable source slots have been used or reserved; keep this project and create a separate revision"))
}
fn png(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut output = Cursor::new(vec![]);
    image.write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}
fn imported_image(path: &std::path::Path, c: &Control) -> Result<(String, DynamicImage)> {
    use sha2::Digest;
    use std::io::Read;
    crate::watch::no_links(path)?;
    let file = fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 200_000_000,
        fail("Inserted image exceeds 200 MB")
    );
    let mut bytes = vec![];
    let mut input = file.take(200_000_001);
    let mut buffer = [0u8; 65536];
    loop {
        c.check()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        anyhow::ensure!(
            bytes.len() + n <= 200_000_000,
            fail("Inserted image grew beyond its limit")
        );
        bytes.extend_from_slice(&buffer[..n]);
    }
    let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    anyhow::ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Tiff)
        ),
        fail("Insert a PNG, JPEG or single-frame TIFF source image")
    );
    if reader.format() == Some(image::ImageFormat::Tiff) {
        let decoder = tiff::decoder::Decoder::new(Cursor::new(&bytes))?;
        anyhow::ensure!(
            !decoder.more_images(),
            fail("Prepare a multipage TIFF first and insert explicitly selected source pages")
        );
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20000);
    limits.max_image_height = Some(20000);
    limits.max_alloc = Some(320_000_000);
    reader.limits(limits);
    let image = reader.decode()?;
    anyhow::ensure!(
        u64::from(image.width()) * u64::from(image.height()) <= 40_000_000,
        fail("Inserted image exceeds 40 million pixels")
    );
    Ok((hex::encode(sha2::Sha256::digest(&bytes)), image))
}
fn image_identity(n: usize, bytes: &[u8], image: &DynamicImage) -> SourceIdentity {
    use sha2::Digest;
    SourceIdentity {
        page: n,
        sha256: hex::encode(sha2::Sha256::digest(bytes)),
        width: image.width(),
        height: image.height(),
    }
}
fn contained(a: &BoxRect, b: &BoxRect) -> bool {
    a.left >= b.left && a.top >= b.top && a.right <= b.right && a.bottom <= b.bottom
}
fn mapped(b: &BoxRect, from: &BoxRect, to: &BoxRect) -> BoxRect {
    BoxRect {
        left: to.left + (b.left - from.left) / (from.right - from.left) * (to.right - to.left),
        top: to.top + (b.top - from.top) / (from.bottom - from.top) * (to.bottom - to.top),
        right: to.left + (b.right - from.left) / (from.right - from.left) * (to.right - to.left),
        bottom: to.top + (b.bottom - from.top) / (from.bottom - from.top) * (to.bottom - to.top),
    }
}
fn map_content(
    content: &PageContent,
    from: &BoxRect,
    to: &BoxRect,
    source: &SourceIdentity,
    split: bool,
) -> Result<(PageContent, usize)> {
    let mut output = PageContent::default();
    let mut withheld = 0;
    for block in &content.blocks {
        if split && block.bbox.as_ref().is_none_or(|b| !contained(b, from)) {
            withheld += 1;
            continue;
        }
        let mut b = block.clone();
        b.bbox = b.bbox.as_ref().map(|r| mapped(r, from, to));
        if let Some(g) = &mut b.geometry {
            if split
                && (g.words.iter().any(|w| !contained(&w.region, from))
                    || g.cells.iter().any(|cell| !contained(&cell.region, from))
                    || g.lines
                        .iter()
                        .any(|line| !contained(&line.shape.bounds(), from)))
            {
                b.geometry = None;
            } else {
                g.source = Some(source.clone());
                for word in &mut g.words {
                    word.region = mapped(&word.region, from, to);
                }
                for cell in &mut g.cells {
                    cell.region = mapped(&cell.region, from, to);
                }
                for line in &mut g.lines {
                    for point in &mut line.shape.points {
                        point.x = to.left
                            + (point.x - from.left) / (from.right - from.left)
                                * (to.right - to.left);
                        point.y = to.top
                            + (point.y - from.top) / (from.bottom - from.top)
                                * (to.bottom - to.top);
                    }
                }
                if let Some(grid) = &mut g.grid {
                    for edge in &mut grid.column_edges {
                        *edge = to.left
                            + (*edge - from.left) / (from.right - from.left) * (to.right - to.left);
                    }
                    for edge in &mut grid.row_edges {
                        *edge = to.top
                            + (*edge - from.top) / (from.bottom - from.top) * (to.bottom - to.top);
                    }
                }
            }
        }
        b.uncertain = true;
        output.blocks.push(b);
    }
    output.warnings=vec!["Derived page: compare projected readings with the immutable parent source. Crossing or ungrounded split blocks remain in the archived parent and are withheld here; no text was inferred.".into()];
    output.validate()?;
    Ok((output, withheld))
}
fn masks(
    page: &Page,
    from: &BoxRect,
    to: &BoxRect,
    source: &SourceIdentity,
) -> Result<Vec<redactions::ConfirmedMask>> {
    let mut result = vec![];
    for mask in &page.confirmed_redactions {
        let intersection = BoxRect {
            left: mask.region.left.max(from.left),
            top: mask.region.top.max(from.top),
            right: mask.region.right.min(from.right),
            bottom: mask.region.bottom.min(from.bottom),
        };
        if intersection.valid() {
            let region = mapped(&intersection, from, to);
            result.push(redactions::ConfirmedMask {
                id: fingerprint(&(&mask.id, source, &region))?,
                source: source.clone(),
                region,
                reason: mask.reason,
            });
        }
    }
    redactions::validate_metadata(&result)?;
    Ok(result)
}
fn projected(
    page: &Page,
    from: &BoxRect,
    to: &BoxRect,
    source: &SourceIdentity,
    split: bool,
) -> Result<(Page, usize)> {
    let (content, withheld) = map_content(&page.content, from, to, source, split)?;
    let mut candidates = vec![];
    for candidate in &page.candidates {
        let scope = candidate.region.clone().unwrap_or_else(BoxRect::full);
        let view = BoxRect {
            left: scope.left.max(from.left),
            top: scope.top.max(from.top),
            right: scope.right.min(from.right),
            bottom: scope.bottom.min(from.bottom),
        };
        if !view.valid() {
            continue;
        }
        let (content, _) = map_content(&candidate.content, from, to, source, split)?;
        if !content.blocks.is_empty() {
            candidates.push(Candidate {
                region: Some(mapped(&view, from, to)),
                model_id: candidate.model_id.clone(),
                family: candidate.family.clone(),
                view: format!("derived-page-{}-{}", page.page, candidate.view),
                content,
                reliability: candidate.reliability,
            });
        }
    }
    let mut result = Page {
        page: source.page,
        image: format!("page-{:05}.png", source.page),
        edited: true,
        content,
        candidates,
        confirmed_redactions: masks(page, from, to, source)?,
        ..Default::default()
    };
    if !result.confirmed_redactions.is_empty() {
        result.content = redactions::enforce_view(
            &result.content,
            source,
            &result.confirmed_redactions,
            None,
            &Control::default(),
        )?
        .0;
        for candidate in &mut result.candidates {
            candidate.content = redactions::enforce_view(
                &candidate.content,
                source,
                &result.confirmed_redactions,
                None,
                &Control::default(),
            )?
            .0;
        }
    }
    result.validate()?;
    Ok((result, withheld))
}
fn source_record(catalog: &Catalog, n: usize) -> Result<&SourceRecord> {
    catalog
        .sources
        .iter()
        .find(|s| s.slot == n)
        .ok_or_else(|| fail("Unknown source slot"))
}
fn load_bound(
    p: &Project,
    catalog: &Catalog,
    n: usize,
    c: &Control,
) -> Result<(Page, DynamicImage)> {
    let page = p.load(n)?;
    let (identity, image) = redactions::source_original(p, &page, c)?;
    redactions::validate_source(&page.confirmed_redactions, &identity)?;
    anyhow::ensure!(
        source_record(catalog, n)?.raster == identity,
        fail("Immutable source changed; preserve this project and explicitly prepare a new source")
    );
    Ok((page, image))
}
fn derived_record(
    catalog: &Catalog,
    source: SourceIdentity,
    parts: Vec<SourcePart>,
    pin: Option<String>,
) -> Result<SourceRecord> {
    Ok(SourceRecord {
        id: fingerprint(&(1, "derived source", &catalog.review_anchor, &source, &parts))?,
        slot: source.page,
        raster: source,
        assignment: "Explicit derived page with retained parent mappings".into(),
        original_input_sha256: None,
        original_page: None,
        original_file: None,
        parts,
        lock_id: pin,
    })
}
pub fn preview(p: &Project, operation: Operation, c: &Control) -> Result<LayoutPreview> {
    p.ensure_current()?;
    let original_manifest = migrations::snapshot(&p.root.join("project.json"), c)?
        .ok_or_else(|| fail("Project manifest missing"))?;
    let canonical_manifest: Manifest = serde_json::from_slice(&original_manifest)?;
    let original_catalog = migrations::snapshot(&p.root.join(CATALOG), c)?
        .ok_or_else(|| fail("Preview and apply stable-source migration first"))?;
    let mut catalog: Catalog = serde_json::from_slice(&original_catalog)?;
    valid(&catalog, p)?;
    let mut order = p.manifest.pages.clone();
    let mut changes = vec![];
    let mut created = vec![];
    let mut withheld = 0;
    let mut bindings = vec![];
    let pin = lock_id(p, c)?;
    match &operation {
        Operation::Reorder { pages } => {
            anyhow::ensure!(
                pages.len() == order.len()
                    && pages.iter().copied().collect::<BTreeSet<_>>()
                        == order.iter().copied().collect(),
                fail("Reorder must contain every active source exactly once")
            );
            order = pages.clone();
        }
        Operation::Remove { pages } => {
            let selected = pages.iter().copied().collect::<BTreeSet<_>>();
            anyhow::ensure!(
                !selected.is_empty()
                    && selected.len() == pages.len()
                    && selected.iter().all(|n| order.contains(n))
                    && selected.len() < order.len(),
                fail("Remove distinct active pages while retaining at least one active page")
            );
            order.retain(|n| !selected.contains(n));
        }
        Operation::Insert { image, position } => {
            anyhow::ensure!(
                *position <= order.len(),
                fail("Choose an insertion position in the active order")
            );
            let (original_sha, raster) = imported_image(image, c)?;
            let bytes = png(&raster)?;
            let n = next_slot(&catalog, p)?;
            let source = image_identity(n, &bytes, &raster);
            crate::geometry::validate_source(&source)?;
            let page=Page{page:n,image:format!("page-{n:05}.png"),edited:true,content:PageContent{blocks:vec![Block{kind:"figure".into(),bbox:Some(BoxRect::full()),description:"Inserted source: transcription pending. The original image is retained; use explicit region reading or Review.".into(),uncertain:true,..Default::default()}],..Default::default()},..Default::default()};
            let record = SourceRecord {
                id: fingerprint(&(
                    1,
                    "inserted source",
                    &catalog.review_anchor,
                    &source,
                    &original_sha,
                ))?,
                slot: n,
                raster: source,
                assignment: "Explicit inserted source; no recognition inherited".into(),
                original_input_sha256: Some(original_sha),
                original_page: Some(1),
                original_file: Some(image.canonicalize()?),
                parts: vec![],
                lock_id: pin.clone(),
            };
            changes.push((format!("page-{n:05}.png"), Some(bytes)));
            changes.push((
                format!("page-{n:05}.json"),
                Some(serde_json::to_vec_pretty(&page)?),
            ));
            changes.push((
                format!("page-{n:05}.edits.json"),
                Some(serde_json::to_vec_pretty(&page)?),
            ));
            catalog.sources.push(record.clone());
            created.push(record);
            order.insert(*position, n);
        }
        Operation::Split { page, axis, at } => {
            anyhow::ensure!(
                order.contains(page) && at.is_finite() && (0.05..=0.95).contains(at),
                fail("Choose an active source and a split fraction from 0.05 to 0.95")
            );
            let (parent, image) = load_bound(p, &catalog, *page, c)?;
            bindings.push((
                *page,
                fingerprint(&parent)?,
                source_record(&catalog, *page)?.raster.clone(),
            ));
            let vertical = matches!(axis, Axis::Vertical);
            let extent = if vertical {
                image.width()
            } else {
                image.height()
            };
            anyhow::ensure!(extent >= 2, fail("Source is too small to split"));
            let cut = ((*at * extent as f32).round() as u32).clamp(1, extent - 1);
            let fraction = cut as f32 / extent as f32;
            let regions = if vertical {
                [
                    BoxRect {
                        right: fraction,
                        ..BoxRect::full()
                    },
                    BoxRect {
                        left: fraction,
                        ..BoxRect::full()
                    },
                ]
            } else {
                [
                    BoxRect {
                        bottom: fraction,
                        ..BoxRect::full()
                    },
                    BoxRect {
                        top: fraction,
                        ..BoxRect::full()
                    },
                ]
            };
            let mut children = vec![];
            for (index, region) in regions.into_iter().enumerate() {
                let n = next_slot(&catalog, p)?;
                let (x, y, r, b) = if vertical {
                    if index == 0 {
                        (0, 0, cut, image.height())
                    } else {
                        (cut, 0, image.width(), image.height())
                    }
                } else if index == 0 {
                    (0, 0, image.width(), cut)
                } else {
                    (0, cut, image.width(), image.height())
                };
                let raster = image.crop_imm(x, y, r - x, b - y);
                let bytes = png(&raster)?;
                let source = image_identity(n, &bytes, &raster);
                let (derived, count) =
                    projected(&parent, &region, &BoxRect::full(), &source, true)?;
                withheld += count;
                let record = derived_record(
                    &catalog,
                    source,
                    vec![SourcePart {
                        source_id: source_record(&catalog, *page)?.id.clone(),
                        parent_region: region,
                        region: BoxRect::full(),
                    }],
                    pin.clone(),
                )?;
                changes.push((format!("page-{n:05}.png"), Some(bytes)));
                changes.push((
                    format!("page-{n:05}.json"),
                    Some(serde_json::to_vec_pretty(&derived)?),
                ));
                changes.push((
                    format!("page-{n:05}.edits.json"),
                    Some(serde_json::to_vec_pretty(&derived)?),
                ));
                children.push(n);
                catalog.sources.push(record.clone());
                created.push(record);
            }
            let position = order.iter().position(|n| n == page).unwrap();
            order.splice(position..=position, children);
        }
        Operation::Merge { pages, axis } => {
            let selected = pages.iter().copied().collect::<BTreeSet<_>>();
            anyhow::ensure!(
                (2..=8).contains(&pages.len())
                    && selected.len() == pages.len()
                    && pages.iter().all(|n| order.contains(n))
                    && order
                        .iter()
                        .filter(|n| selected.contains(n))
                        .copied()
                        .collect::<Vec<_>>()
                        == *pages,
                fail("Merge 2–8 distinct active pages in their current order")
            );
            let vertical = matches!(axis, Axis::Vertical);
            let mut parents = vec![];
            let (mut width, mut height) = (0u32, 0u32);
            for n in pages {
                let (page, image) = load_bound(p, &catalog, *n, c)?;
                bindings.push((
                    *n,
                    fingerprint(&page)?,
                    source_record(&catalog, *n)?.raster.clone(),
                ));
                if vertical {
                    width = width.max(image.width());
                    height = height
                        .checked_add(image.height())
                        .ok_or_else(|| fail("Merged image height overflow"))?;
                } else {
                    width = width
                        .checked_add(image.width())
                        .ok_or_else(|| fail("Merged image width overflow"))?;
                    height = height.max(image.height());
                }
                parents.push((page, image));
            }
            anyhow::ensure!(
                width <= 20000
                    && height <= 20000
                    && u64::from(width) * u64::from(height) <= 40_000_000,
                fail("Merged source exceeds 20,000 pixels per edge or 40 million pixels")
            );
            let mut canvas =
                RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
            let mut offset = 0;
            let mut views = vec![];
            for (_, image) in &parents {
                c.check()?;
                let (x, y) = if vertical { (0, offset) } else { (offset, 0) };
                image::imageops::overlay(
                    &mut canvas,
                    &image.to_rgba8(),
                    i64::from(x),
                    i64::from(y),
                );
                views.push(BoxRect {
                    left: x as f32 / width as f32,
                    top: y as f32 / height as f32,
                    right: (x + image.width()) as f32 / width as f32,
                    bottom: (y + image.height()) as f32 / height as f32,
                });
                offset += if vertical {
                    image.height()
                } else {
                    image.width()
                };
            }
            let image = DynamicImage::ImageRgba8(canvas);
            let bytes = png(&image)?;
            let n = next_slot(&catalog, p)?;
            let source = image_identity(n, &bytes, &image);
            let mut result = Page {
                page: n,
                image: format!("page-{n:05}.png"),
                edited: true,
                ..Default::default()
            };
            let mut parts = vec![];
            for ((parent, _), view) in parents.iter().zip(&views) {
                let (part, _) = projected(parent, &BoxRect::full(), view, &source, false)?;
                result.content.blocks.extend(part.content.blocks);
                result.candidates.extend(part.candidates);
                result
                    .confirmed_redactions
                    .extend(part.confirmed_redactions);
                parts.push(SourcePart {
                    source_id: source_record(&catalog, parent.page)?.id.clone(),
                    parent_region: BoxRect::full(),
                    region: view.clone(),
                });
            }
            result.content.warnings=vec!["Merged page: readings were geometrically projected from retained source pages; review before approval.".into()];
            if !result.confirmed_redactions.is_empty() {
                result.content = redactions::enforce_view(
                    &result.content,
                    &source,
                    &result.confirmed_redactions,
                    None,
                    c,
                )?
                .0;
                for candidate in &mut result.candidates {
                    candidate.content = redactions::enforce_view(
                        &candidate.content,
                        &source,
                        &result.confirmed_redactions,
                        None,
                        c,
                    )?
                    .0;
                    if let Some(view) = &candidate.region {
                        candidate.content.blocks.retain(|block| {
                            block.bbox.as_ref().is_some_and(|b| contained(b, view))
                        });
                    }
                }
            }
            result.validate()?;
            let record = derived_record(&catalog, source, parts, pin.clone())?;
            changes.push((format!("page-{n:05}.png"), Some(bytes)));
            changes.push((
                format!("page-{n:05}.json"),
                Some(serde_json::to_vec_pretty(&result)?),
            ));
            changes.push((
                format!("page-{n:05}.edits.json"),
                Some(serde_json::to_vec_pretty(&result)?),
            ));
            catalog.sources.push(record.clone());
            created.push(record);
            let first = order.iter().position(|n| selected.contains(n)).unwrap();
            order.retain(|n| !selected.contains(n));
            order.insert(first, n);
        }
    }
    anyhow::ensure!(
        order != p.manifest.pages,
        fail("Page organization would make no change")
    );
    catalog.states.truncate(catalog.cursor + 1);
    anyhow::ensure!(
        catalog.states.len() < 256,
        fail("Layout history limit reached; keep a private separate revision")
    );
    let revision = catalog.next_revision;
    catalog.next_revision = catalog.next_revision.checked_add(1).ok_or_else(|| {
        fail("Layout revision counter exhausted; keep a private separate revision")
    })?;
    catalog.states.push(LayoutState {
        revision,
        order: order.clone(),
        label: "Explicit page organization".into(),
    });
    catalog.cursor = catalog.states.len() - 1;
    let mut manifest = canonical_manifest;
    manifest.pages = order.clone();
    valid(
        &catalog,
        &Project {
            root: p.root.clone(),
            manifest: manifest.clone(),
        },
    )?;
    changes.push((
        "project.json".into(),
        Some(serde_json::to_vec_pretty(&manifest)?),
    ));
    changes.push((CATALOG.into(), Some(catalog_bytes(&catalog)?)));
    let mut plan = migrations::plan(&p.root, "Organize project pages", changes, c)?;
    anyhow::ensure!(
        plan.files
            .iter()
            .find(|f| f.file == "project.json")
            .and_then(|f| f.before.as_ref())
            == Some(&migrations::snapshot_identity(&original_manifest)),
        fail("Manifest changed during layout preview")
    );
    anyhow::ensure!(
        plan.files
            .iter()
            .find(|f| f.file == CATALOG)
            .and_then(|f| f.before.as_ref())
            == Some(&migrations::snapshot_identity(&original_catalog)),
        fail("Source catalog changed during layout preview")
    );
    for source in &created {
        for name in [
            format!("page-{:05}.png", source.slot),
            format!("page-{:05}.json", source.slot),
            format!("page-{:05}.edits.json", source.slot),
        ] {
            anyhow::ensure!(
                plan.files
                    .iter()
                    .find(|f| f.file == name)
                    .is_some_and(|f| f.before.is_none()),
                fail("New source slot has untracked files; nothing was overwritten")
            );
        }
    }
    migrations::bind_pages(&mut plan, bindings.clone())?;
    bind_selection(&mut plan, p, &pin, c)?;
    let note="Active order changes only. Removed/parent scans, checkpoints, candidates, ordinary review history and exact model locks remain private and unchanged. Split crossings and ungrounded blocks are explicitly withheld from derived pages and retained in their parent; no missing text is inferred.".into();
    let token = fingerprint(&(
        &plan.token,
        &operation,
        &order,
        &created,
        withheld,
        &bindings,
    ))?;
    Ok(LayoutPreview {
        operation,
        token,
        order,
        created,
        withheld_blocks: withheld,
        note,
        plan,
        bindings,
    })
}
pub fn apply(p: &Project, preview: &LayoutPreview, token: &str, c: &Control) -> Result<Project> {
    anyhow::ensure!(
        token == preview.token
            && token
                == fingerprint(&(
                    &preview.plan.token,
                    &preview.operation,
                    &preview.order,
                    &preview.created,
                    preview.withheld_blocks,
                    &preview.bindings
                ))?,
        fail("Confirm the exact fresh page organization preview")
    );
    for (n, digest, source) in &preview.bindings {
        let page = p.load(*n)?;
        anyhow::ensure!(
            fingerprint(&page)? == *digest && redactions::source_identity(p, &page, c)? == *source,
            fail("Parent page changed after layout preview")
        );
    }
    migrations::apply(&preview.plan, &preview.plan.token, c)?;
    Project::read(p.root.clone())
}
fn move_cursor(p: &Project, expected_revision: u64, forward: bool, c: &Control) -> Result<Project> {
    p.ensure_current()?;
    let original_manifest = migrations::snapshot(&p.root.join("project.json"), c)?
        .ok_or_else(|| fail("Project manifest missing"))?;
    let original_catalog = migrations::snapshot(&p.root.join(CATALOG), c)?
        .ok_or_else(|| fail("Stable source identities are not enabled"))?;
    let mut catalog: Catalog = serde_json::from_slice(&original_catalog)?;
    valid(&catalog, p)?;
    anyhow::ensure!(
        catalog.states[catalog.cursor].revision == expected_revision,
        fail("Layout changed; inspect the current order")
    );
    let next = if forward {
        catalog
            .cursor
            .checked_add(1)
            .filter(|n| *n < catalog.states.len())
    } else {
        catalog.cursor.checked_sub(1)
    }
    .ok_or_else(|| fail("No layout revision in that direction"))?;
    let mut bindings = vec![];
    for n in &catalog.states[next].order {
        let (page, _) = load_bound(p, &catalog, *n, c)?;
        bindings.push((
            *n,
            fingerprint(&page)?,
            source_record(&catalog, *n)?.raster.clone(),
        ));
    }
    catalog.cursor = next;
    let mut manifest: Manifest = serde_json::from_slice(&original_manifest)?;
    manifest.pages = catalog.states[next].order.clone();
    valid(
        &catalog,
        &Project {
            root: p.root.clone(),
            manifest: manifest.clone(),
        },
    )?;
    let mut plan = migrations::plan(
        &p.root,
        if forward {
            "Redo page organization"
        } else {
            "Undo page organization"
        },
        vec![
            (
                "project.json".into(),
                Some(serde_json::to_vec_pretty(&manifest)?),
            ),
            (CATALOG.into(), Some(catalog_bytes(&catalog)?)),
        ],
        c,
    )?;
    anyhow::ensure!(
        plan.files[0].before.as_ref() == Some(&migrations::snapshot_identity(&original_manifest)),
        fail("Manifest changed during layout navigation")
    );
    anyhow::ensure!(
        plan.files[1].before.as_ref() == Some(&migrations::snapshot_identity(&original_catalog)),
        fail("Source catalog changed during layout navigation")
    );
    migrations::bind_pages(&mut plan, bindings)?;
    migrations::apply(&plan, &plan.token, c)?;
    Project::read(p.root.clone())
}
pub fn undo(p: &Project, expected_revision: u64, c: &Control) -> Result<Project> {
    move_cursor(p, expected_revision, false, c)
}
pub fn redo(p: &Project, expected_revision: u64, c: &Control) -> Result<Project> {
    move_cursor(p, expected_revision, true, c)
}
