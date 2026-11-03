//! Observed source geometry. Missing word/cell precision remains missing.
use crate::{redactions::SourceIdentity, storage::Control, types::*, watch::no_links};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs::File, io::Read, path::Path};

pub const MAX_PAGE_ENTRIES: usize = 100_000;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    TesseractTsv,
    DetectedGrid,
    ReaderSupplied,
    HumanConfirmed,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Quad {
    pub points: [Point; 4],
}
impl Quad {
    pub fn rectangle(region: &BoxRect) -> Self {
        Self {
            points: [
                Point {
                    x: region.left,
                    y: region.top,
                },
                Point {
                    x: region.right,
                    y: region.top,
                },
                Point {
                    x: region.right,
                    y: region.bottom,
                },
                Point {
                    x: region.left,
                    y: region.bottom,
                },
            ],
        }
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.points.iter().all(|p| p.x.is_finite()
                && p.y.is_finite()
                && (0.0..=1.0).contains(&p.x)
                && (0.0..=1.0).contains(&p.y)),
            fail("Line corners must be finite page coordinates")
        );
        for i in 0..4 {
            let (a, b, d) = (
                self.points[i],
                self.points[(i + 1) % 4],
                self.points[(i + 2) % 4],
            );
            anyhow::ensure!(
                (b.x - a.x) * (d.y - b.y) - (b.y - a.y) * (d.x - b.x) > 0.000_000_000_001,
                fail("Line corners must form a nondegenerate convex clockwise quadrilateral")
            );
        }
        Ok(())
    }
    pub fn bounds(&self) -> BoxRect {
        BoxRect {
            left: self.points.iter().map(|p| p.x).fold(1., f32::min),
            top: self.points.iter().map(|p| p.y).fold(1., f32::min),
            right: self.points.iter().map(|p| p.x).fold(0., f32::max),
            bottom: self.points.iter().map(|p| p.y).fold(0., f32::max),
        }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        (0..4).all(|i| {
            let (a, b) = (self.points[i], self.points[(i + 1) % 4]);
            (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x) >= -f32::EPSILON
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CellAddress {
    pub row: usize,
    pub col: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WordBox {
    pub text: String,
    pub region: BoxRect,
    pub confidence: Option<f32>,
    pub line: Option<usize>,
    pub cell: Option<CellAddress>,
    pub origin: Origin,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CellBox {
    pub row: usize,
    pub col: usize,
    pub region: BoxRect,
    pub origin: Origin,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LineRegion {
    pub ordinal: usize,
    pub shape: Quad,
    pub origin: Origin,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TableGrid {
    pub row_edges: Vec<f32>,
    pub column_edges: Vec<f32>,
    pub origin: Origin,
}
impl TableGrid {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            (2..=101).contains(&self.row_edges.len())
                && (2..=41).contains(&self.column_edges.len()),
            fail("Table grid exceeds 100 rows / 40 columns")
        );
        for edges in [&self.row_edges, &self.column_edges] {
            anyhow::ensure!(
                edges
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    && edges.windows(2).all(|p| p[0] < p[1]),
                fail("Table boundaries must be finite, strictly increasing page coordinates")
            );
        }
        Ok(())
    }
    pub fn cell(&self, row: usize, col: usize) -> Option<BoxRect> {
        Some(BoxRect {
            left: *self.column_edges.get(col)?,
            top: *self.row_edges.get(row)?,
            right: *self.column_edges.get(col + 1)?,
            bottom: *self.row_edges.get(row + 1)?,
        })
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BlockGeometry {
    /// None only while parsing native output before its immutable PNG is saved.
    pub source: Option<SourceIdentity>,
    pub words: Vec<WordBox>,
    pub cells: Vec<CellBox>,
    pub lines: Vec<LineRegion>,
    pub grid: Option<TableGrid>,
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG201", message).into()
}
pub fn validate_source(source: &SourceIdentity) -> Result<()> {
    anyhow::ensure!(
        (1..=3000).contains(&source.page)
            && source.sha256.len() == 64
            && source
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && source.width > 0
            && source.height > 0
            && source.width <= 20000
            && source.height <= 20000
            && u64::from(source.width) * u64::from(source.height) <= 40_000_000,
        fail("Invalid geometry source identity")
    );
    Ok(())
}
pub fn validate(block: &Block) -> Result<()> {
    let Some(data) = &block.geometry else {
        return Ok(());
    };
    if let Some(source) = &data.source {
        validate_source(source)?;
    }
    anyhow::ensure!(
        data.words.len() <= 30_000 && data.cells.len() <= 4000 && data.lines.len() <= 30_000,
        fail("Block geometry exceeds its entry budget")
    );
    let mut lines = BTreeSet::new();
    for line in &data.lines {
        line.shape.validate()?;
        anyhow::ensure!(
            line.ordinal < 30_000 && lines.insert(line.ordinal),
            fail("Invalid/duplicate source line")
        );
    }
    let dimensions = if block.kind == "table" {
        Some((block.rows.len(), block.rows.first().map_or(0, Vec::len)))
    } else {
        None
    };
    let mut cells = BTreeSet::new();
    for cell in &data.cells {
        anyhow::ensure!(
            cell.region.valid()
                && dimensions.is_some_and(|(rows, cols)| cell.row < rows && cell.col < cols)
                && cells.insert((cell.row, cell.col)),
            fail("Invalid/duplicate table-cell source box")
        );
    }
    for word in &data.words {
        anyhow::ensure!(
            !word.text.is_empty()
                && word.text.len() <= 4096
                && word.region.valid()
                && word
                    .confidence
                    .is_none_or(|v| v.is_finite() && (0.0..=1.0).contains(&v))
                && word.line.is_none_or(|line| lines.contains(&line))
                && word.cell.as_ref().is_none_or(|cell| dimensions
                    .is_some_and(|(rows, cols)| cell.row < rows && cell.col < cols)),
            fail("Invalid source word geometry")
        );
    }
    if let Some(grid) = &data.grid {
        grid.validate()?;
        anyhow::ensure!(
            dimensions == Some((grid.row_edges.len() - 1, grid.column_edges.len() - 1)),
            fail("Grid shape differs from table dimensions")
        );
    }
    Ok(())
}
pub fn validate_page(page: &Page) -> Result<()> {
    let mut count = 0usize;
    for block in page
        .content
        .blocks
        .iter()
        .chain(page.candidates.iter().flat_map(|c| c.content.blocks.iter()))
    {
        if let Some(data) = &block.geometry {
            let source = data
                .source
                .as_ref()
                .ok_or_else(|| fail("Checkpoint geometry must be bound to its saved source"))?;
            validate_source(source)?;
            anyhow::ensure!(
                source.page == page.page,
                fail("Geometry points at a different source page")
            );
            count = count
                .checked_add(data.words.len() + data.cells.len() + data.lines.len())
                .ok_or_else(|| fail("Geometry entry count overflow"))?;
            anyhow::ensure!(
                count <= MAX_PAGE_ENTRIES,
                fail("Page geometry exceeds its total evidence budget")
            );
        }
    }
    Ok(())
}
pub fn source_identity(path: &Path, page: usize, c: &Control) -> Result<SourceIdentity> {
    c.check()?;
    no_links(path)?;
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file() && before.len() <= 200_000_000,
        fail("Source image exceeds geometry byte budget")
    );
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        c.check()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        anyhow::ensure!(
            total <= 200_000_000,
            fail("Source grew beyond geometry byte budget")
        );
        hash.update(&buffer[..count]);
    }
    let (width, height) = image::image_dimensions(path)?;
    let source = SourceIdentity {
        page,
        sha256: hex::encode(hash.finalize()),
        width,
        height,
    };
    validate_source(&source)?;
    let after = std::fs::metadata(path)?;
    no_links(path)?;
    anyhow::ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        fail("Source changed while binding geometry")
    );
    Ok(source)
}
pub fn bind_content(content: &mut PageContent, source: &SourceIdentity) -> Result<()> {
    validate_source(source)?;
    for block in &mut content.blocks {
        if let Some(data) = &mut block.geometry {
            data.source = Some(source.clone());
        }
        validate(block)?;
    }
    Ok(())
}
/// Verify fine-grained metadata against today's actual immutable PNG before
/// rendering/export/rereading it. Legacy text-only pages without such metadata
/// do not acquire a new source-file requirement.
pub fn verify_metadata_source(
    project: &crate::storage::Project,
    page: &Page,
    c: &Control,
) -> Result<Option<SourceIdentity>> {
    let has_geometry = page
        .content
        .blocks
        .iter()
        .chain(
            page.candidates
                .iter()
                .flat_map(|candidate| candidate.content.blocks.iter()),
        )
        .any(|block| block.geometry.is_some());
    if !has_geometry && page.script_detection.is_none() && page.region_languages.is_empty() {
        return Ok(None);
    }
    let actual = source_identity(&project.image(page.page), page.page, c)?;
    for block in page.content.blocks.iter().chain(
        page.candidates
            .iter()
            .flat_map(|candidate| candidate.content.blocks.iter()),
    ) {
        if let Some(data) = &block.geometry {
            anyhow::ensure!(
                data.source.as_ref() == Some(&actual),
                fail(
                    "Fine-grained geometry refers to changed source pixels; preserve the checkpoint and reprepare explicitly"
                )
            );
        }
    }
    anyhow::ensure!(
        page.script_detection
            .as_ref()
            .is_none_or(|detection| detection.source == actual)
            && page
                .region_languages
                .iter()
                .all(|selection| selection.source == actual),
        fail("Script/language metadata refers to changed source pixels")
    );
    Ok(Some(actual))
}
fn map_box(region: &BoxRect, view: &BoxRect) -> BoxRect {
    BoxRect {
        left: view.left + region.left * (view.right - view.left),
        top: view.top + region.top * (view.bottom - view.top),
        right: view.left + region.right * (view.right - view.left),
        bottom: view.top + region.bottom * (view.bottom - view.top),
    }
}
/// Map normalized crop observations using the ACTUAL pixel crop's page view.
pub fn map_content(
    content: &mut PageContent,
    view: &BoxRect,
    source: &SourceIdentity,
) -> Result<()> {
    anyhow::ensure!(view.valid(), fail("Invalid global crop view"));
    validate_source(source)?;
    for block in &mut content.blocks {
        if let Some(region) = &mut block.bbox {
            *region = map_box(region, view);
        }
        if let Some(data) = &mut block.geometry {
            for word in &mut data.words {
                word.region = map_box(&word.region, view);
            }
            for cell in &mut data.cells {
                cell.region = map_box(&cell.region, view);
            }
            for line in &mut data.lines {
                for point in &mut line.shape.points {
                    point.x = view.left + point.x * (view.right - view.left);
                    point.y = view.top + point.y * (view.bottom - view.top);
                }
            }
            if let Some(grid) = &mut data.grid {
                for edge in &mut grid.row_edges {
                    *edge = view.top + *edge * (view.bottom - view.top);
                }
                for edge in &mut grid.column_edges {
                    *edge = view.left + *edge * (view.right - view.left);
                }
            }
            data.source = Some(source.clone());
        }
        validate(block)?;
    }
    Ok(())
}

/// Remove stale source precision after ordinary structural table edits. The
/// original observations stay in immutable candidates and review history.
pub fn clear_structurally_stale(block: &mut Block) {
    if block
        .geometry
        .as_ref()
        .and_then(|g| g.grid.as_ref())
        .is_some_and(|grid| {
            block.kind != "table"
                || grid.row_edges.len() != block.rows.len() + 1
                || grid.column_edges.len() != block.rows.first().map_or(0, Vec::len) + 1
        })
    {
        block.geometry = None;
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeatKind {
    LowWordConfidence,
    WordDisagreement,
    CellDisagreement,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeatSpot {
    pub region: BoxRect,
    pub kind: HeatKind,
    pub candidate: Option<usize>,
    pub block: usize,
    pub word: Option<usize>,
    pub cell: Option<CellAddress>,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeatmapReport {
    pub spots: Vec<HeatSpot>,
    pub missing_precision_blocks: usize,
    pub bounded: bool,
    pub note: String,
}
struct Observation<'a> {
    region: &'a BoxRect,
    value: &'a str,
    family: &'a str,
    candidate: usize,
    block: usize,
    word: Option<usize>,
    cell: Option<CellAddress>,
}
fn overlap_ratio(a: &BoxRect, b: &BoxRect) -> f32 {
    let overlap = (a.right.min(b.right) - a.left.max(b.left)).max(0.)
        * (a.bottom.min(b.bottom) - a.top.max(b.top)).max(0.);
    let area_a = (a.right - a.left) * (a.bottom - a.top);
    let area_b = (b.right - b.left) * (b.bottom - b.top);
    overlap / (area_a + area_b - overlap).max(f32::MIN_POSITIVE)
}
/// Fine overlays use supplied boxes only. With no such boxes, retain the UI's
/// coarse block issues rather than deriving word coordinates from text length.
pub fn heatmap(
    project: &crate::storage::Project,
    page: &Page,
    threshold: f32,
    c: &Control,
) -> Result<HeatmapReport> {
    anyhow::ensure!(
        threshold.is_finite() && (0.0..=1.0).contains(&threshold),
        fail("Invalid geometry review threshold")
    );
    page.validate()?;
    verify_metadata_source(project, page, c)?;
    let visible = |region: &BoxRect| {
        !page.confirmed_redactions.iter().any(|mask| {
            mask.region.left < region.right
                && region.left < mask.region.right
                && mask.region.top < region.bottom
                && region.top < mask.region.bottom
        })
    };
    let mut report=HeatmapReport{spots:vec![],missing_precision_blocks:page.content.blocks.iter().filter(|block|block.geometry.as_ref().is_none_or(|data|data.words.is_empty()&&data.cells.is_empty())).count(),bounded:false,
        note:"Word/cell boxes are supplied observations, not inferred from character lengths. OCR confidence is not measured accuracy. Disagreements require different reader families with overlapping supplied boxes; repeated crops/languages from one family are not independent. Missing or budget-limited precision keeps the coarse block overlay.".into()};
    let mut seen = BTreeSet::new();
    let mut observations = vec![];
    for (block_index, block) in page.content.blocks.iter().enumerate() {
        if let Some(data) = &block.geometry {
            for (word_index, word) in data.words.iter().enumerate() {
                if visible(&word.region) && word.confidence.is_some_and(|value| value < threshold) {
                    let key = (
                        word.region.left.to_bits(),
                        word.region.top.to_bits(),
                        word.region.right.to_bits(),
                        word.region.bottom.to_bits(),
                        0u8,
                    );
                    if seen.insert(key) {
                        if report.spots.len() < 2000 {
                            report.spots.push(HeatSpot{region:word.region.clone(),kind:HeatKind::LowWordConfidence,candidate:None,block:block_index,word:Some(word_index),cell:word.cell.clone(),note:"Supplied word confidence is below the review threshold; compare this observation with the source.".into()});
                        } else {
                            report.bounded = true;
                        }
                    }
                }
            }
        }
    }
    for (candidate_index, candidate) in page.candidates.iter().enumerate() {
        c.check()?;
        for (block_index, block) in candidate.content.blocks.iter().enumerate() {
            let Some(data) = &block.geometry else {
                continue;
            };
            for (word_index, word) in data.words.iter().enumerate() {
                if !visible(&word.region) {
                    continue;
                }
                if word.confidence.is_some_and(|value| value < threshold) {
                    let key = (
                        word.region.left.to_bits(),
                        word.region.top.to_bits(),
                        word.region.right.to_bits(),
                        word.region.bottom.to_bits(),
                        0u8,
                    );
                    if seen.insert(key) {
                        if report.spots.len() < 2000 {
                            report.spots.push(HeatSpot{region:word.region.clone(),kind:HeatKind::LowWordConfidence,candidate:Some(candidate_index),block:block_index,word:Some(word_index),cell:word.cell.clone(),note:"Supplied word confidence is below the review threshold; compare this reader observation with the source.".into()});
                        } else {
                            report.bounded = true;
                        }
                    }
                }
                observations.push(Observation {
                    region: &word.region,
                    value: &word.text,
                    family: &candidate.family,
                    candidate: candidate_index,
                    block: block_index,
                    word: Some(word_index),
                    cell: word.cell.clone(),
                });
            }
            for cell in &data.cells {
                if visible(&cell.region)
                    && let Some(value) = block.rows.get(cell.row).and_then(|row| row.get(cell.col))
                {
                    observations.push(Observation {
                        region: &cell.region,
                        value,
                        family: &candidate.family,
                        candidate: candidate_index,
                        block: block_index,
                        word: None,
                        cell: Some(CellAddress {
                            row: cell.row,
                            col: cell.col,
                        }),
                    });
                }
            }
        }
    }
    // A spatial index avoids a page-wide quadratic search. Huge/ambiguous boxes
    // remain coarse; caps bound indexing, comparison and rendering independently.
    let mut bins = BTreeMap::<(u8, u8, bool), Vec<usize>>::new();
    let mut index_entries = 0usize;
    let mut compared = BTreeSet::new();
    let mut comparisons = 0usize;
    for (index, observation) in observations.iter().enumerate() {
        c.check()?;
        let region = observation.region;
        let left = (region.left * 32.) as u8;
        let top = (region.top * 32.) as u8;
        let right = ((region.right * 32.).ceil() as u8).min(31);
        let bottom = ((region.bottom * 32.).ceil() as u8).min(31);
        let entries = (usize::from(right - left) + 1) * (usize::from(bottom - top) + 1);
        if entries > 64 || index_entries + entries > 1_000_000 {
            report.bounded = true;
            continue;
        }
        for y in top..=bottom {
            for x in left..=right {
                let key = (x, y, observation.word.is_some());
                if let Some(previous) = bins.get(&key) {
                    for other_index in previous {
                        if comparisons >= 200_000 {
                            report.bounded = true;
                            break;
                        }
                        if !compared.insert((*other_index, index)) {
                            continue;
                        }
                        comparisons += 1;
                        let other = &observations[*other_index];
                        if other.family == observation.family
                            || other.family.is_empty()
                            || observation.family.is_empty()
                            || other.value == observation.value
                            || overlap_ratio(other.region, region) < 0.5
                        {
                            continue;
                        }
                        let kind = if observation.word.is_some() {
                            HeatKind::WordDisagreement
                        } else {
                            HeatKind::CellDisagreement
                        };
                        let tag = if kind == HeatKind::WordDisagreement {
                            1
                        } else {
                            2
                        };
                        let spot_key = (
                            region.left.to_bits(),
                            region.top.to_bits(),
                            region.right.to_bits(),
                            region.bottom.to_bits(),
                            tag,
                        );
                        if seen.insert(spot_key) {
                            if report.spots.len() < 2000 {
                                report.spots.push(HeatSpot{region:region.clone(),kind,candidate:Some(observation.candidate),block:observation.block,word:observation.word,cell:observation.cell.clone(),note:"Different independent reader families supplied different values at overlapping observed source boxes. This localizes disagreement, not the correct answer.".into()});
                            } else {
                                report.bounded = true;
                            }
                        }
                    }
                }
                bins.entry(key).or_default().push(index);
                index_entries += 1;
            }
        }
    }
    Ok(report)
}
