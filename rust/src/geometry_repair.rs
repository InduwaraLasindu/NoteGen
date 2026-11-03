//! Human-confirmed line/table source repairs. Every edit uses ordinary undo history.
use crate::{
    geometry::{self, BlockGeometry, CellAddress, CellBox, LineRegion, Origin, Quad, TableGrid},
    redactions::SourceIdentity,
    storage::{Control, Project, fingerprint},
    types::*,
};
use image::{DynamicImage, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG201", message).into()
}
fn checked_source(project: &Project, expected: &Page, c: &Control) -> Result<SourceIdentity> {
    anyhow::ensure!(
        fingerprint(&project.load(expected.page)?)? == fingerprint(expected)?,
        fail("Page changed after the geometry selection")
    );
    let source = crate::redactions::source_identity(project, expected, c)?;
    crate::redactions::validate_source(&expected.confirmed_redactions, &source)?;
    Ok(source)
}
fn existing_geometry(block: &Block, source: &SourceIdentity) -> Result<BlockGeometry> {
    let mut data = block.geometry.clone().unwrap_or_default();
    anyhow::ensure!(
        data.source.as_ref().is_none_or(|old| old == source),
        fail("Observed geometry refers to changed source pixels")
    );
    data.source = Some(source.clone());
    Ok(data)
}
pub fn repair_lines(
    project: &Project,
    expected: &Page,
    block_index: usize,
    shapes: &[Quad],
    c: &Control,
) -> Result<u64> {
    anyhow::ensure!(
        !shapes.is_empty() && shapes.len() <= 1000,
        fail("Confirm 1–1,000 ordered source line regions")
    );
    for shape in shapes {
        shape.validate()?;
    }
    let source = checked_source(project, expected, c)?;
    let mut next = expected.clone();
    let block = next
        .content
        .blocks
        .get_mut(block_index)
        .ok_or_else(|| fail("Source block no longer exists"))?;
    let mut data = existing_geometry(block, &source)?;
    data.lines = shapes
        .iter()
        .enumerate()
        .map(|(ordinal, shape)| LineRegion {
            ordinal,
            shape: shape.clone(),
            origin: Origin::HumanConfirmed,
        })
        .collect();
    for word in &mut data.words {
        let x = (word.region.left + word.region.right) / 2.;
        let y = (word.region.top + word.region.bottom) / 2.;
        let matches = data
            .lines
            .iter()
            .filter(|line| line.shape.contains(x, y))
            .map(|line| line.ordinal)
            .collect::<Vec<_>>();
        word.line = if matches.len() == 1 {
            Some(matches[0])
        } else {
            None
        };
    }
    block.geometry = Some(data);
    block.uncertain = true;
    next.reviewed = false;
    next.validate()?;
    c.check()?;
    crate::review::commit(
        project,
        expected,
        &next,
        "Repair handwritten line boundaries",
    )
}
pub struct LineView {
    pub source: SourceIdentity,
    pub region: BoxRect,
    pub image: DynamicImage,
}
/// Re-read input that uses the repaired line, rather than its old rectangular
/// paragraph. Opaque covers are applied first; pixels outside the quad are white.
pub fn line_view(
    project: &Project,
    expected: &Page,
    block_index: usize,
    ordinal: usize,
    c: &Control,
) -> Result<LineView> {
    let (source, image) = crate::redactions::recognition_view(project, expected, c)?;
    anyhow::ensure!(
        fingerprint(&project.load(expected.page)?)? == fingerprint(expected)?,
        fail("Page changed before line re-reading")
    );
    let data = expected
        .content
        .blocks
        .get(block_index)
        .and_then(|block| block.geometry.as_ref())
        .ok_or_else(|| fail("This block has no confirmed line boundaries"))?;
    anyhow::ensure!(
        data.source.as_ref() == Some(&source),
        fail("Line geometry refers to changed source pixels")
    );
    let shape = &data
        .lines
        .iter()
        .find(|line| line.ordinal == ordinal)
        .ok_or_else(|| fail("Selected source line no longer exists"))?
        .shape;
    shape.validate()?;
    let (l, t, r, b) = shape.bounds().pixels(image.width(), image.height());
    let crop = image.crop_imm(l, t, r - l, b - t).to_rgba8();
    let mut output = RgbImage::from_pixel(r - l, b - t, Rgb([255, 255, 255]));
    for y in 0..output.height() {
        c.check()?;
        for x in 0..output.width() {
            let nx = (l as f64 + f64::from(x) + 0.5) / f64::from(image.width());
            let ny = (t as f64 + f64::from(y) + 0.5) / f64::from(image.height());
            if shape.contains(nx as f32, ny as f32) {
                let pixel = crop[(x, y)];
                let alpha = u32::from(pixel[3]);
                output[(x, y)] = Rgb([0, 1, 2].map(|i| {
                    ((u32::from(pixel[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
                }));
            }
        }
    }
    let region = BoxRect {
        left: l as f32 / image.width() as f32,
        top: t as f32 / image.height() as f32,
        right: r as f32 / image.width() as f32,
        bottom: b as f32 / image.height() as f32,
    };
    anyhow::ensure!(
        geometry::source_identity(&project.image(expected.page), expected.page, c)? == source,
        fail("Source changed while building the repaired line view")
    );
    Ok(LineView {
        source,
        region,
        image: DynamicImage::ImageRgb8(output),
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellMove {
    pub from_row: usize,
    pub from_col: usize,
    pub to_row: usize,
    pub to_col: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridRepair {
    pub grid: TableGrid,
    pub mapping: Vec<CellMove>,
    pub merges: Vec<Merge>,
    pub header_rows: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GridPreview {
    pub source: SourceIdentity,
    pub before_fingerprint: String,
    pub after: Page,
    pub token: String,
    pub note: String,
}
fn grid_token(source: &SourceIdentity, before: &str, after: &Page) -> Result<String> {
    fingerprint(&(1, source, before, after))
}
pub fn preview_grid(
    project: &Project,
    expected: &Page,
    block_index: usize,
    repair: &GridRepair,
    c: &Control,
) -> Result<GridPreview> {
    repair.grid.validate()?;
    let source = checked_source(project, expected, c)?;
    let original = expected
        .content
        .blocks
        .get(block_index)
        .ok_or_else(|| fail("Selected table no longer exists"))?;
    anyhow::ensure!(
        original.kind == "table"
            && repair.mapping.len() == original.rows.iter().map(Vec::len).sum::<usize>(),
        fail("Map every original table cell exactly once, including empty cells")
    );
    let rows = repair.grid.row_edges.len() - 1;
    let cols = repair.grid.column_edges.len() - 1;
    let mut values = vec![vec![String::new(); cols]; rows];
    let mut seen = BTreeSet::new();
    let mut mapping = BTreeMap::new();
    for cell in &repair.mapping {
        let text = original
            .rows
            .get(cell.from_row)
            .and_then(|row| row.get(cell.from_col))
            .ok_or_else(|| fail("Cell mapping points outside the old grid"))?;
        anyhow::ensure!(
            cell.to_row < rows && cell.to_col < cols && seen.insert((cell.from_row, cell.from_col)),
            fail("Cell mapping is duplicate or outside the repaired grid")
        );
        let target = &mut values[cell.to_row][cell.to_col];
        // Several explicitly mapped cells combine in the supplied order, retaining
        // every nonempty original value. New split cells start empty, never guessed.
        if !text.is_empty() {
            if !target.is_empty() {
                target.push('\n');
            }
            target.push_str(text);
        }
        mapping.insert(
            (cell.from_row, cell.from_col),
            CellAddress {
                row: cell.to_row,
                col: cell.to_col,
            },
        );
    }
    let mut next = expected.clone();
    let block = &mut next.content.blocks[block_index];
    let mut data = existing_geometry(block, &source)?;
    let mut grid = repair.grid.clone();
    grid.origin = Origin::HumanConfirmed;
    data.cells.clear();
    for row in 0..rows {
        for col in 0..cols {
            data.cells.push(CellBox {
                row,
                col,
                region: grid.cell(row, col).expect("validated grid"),
                origin: Origin::HumanConfirmed,
            });
        }
    }
    for word in &mut data.words {
        if let Some(cell) = &word.cell {
            word.cell = mapping.get(&(cell.row, cell.col)).cloned();
        }
    }
    block.rows = values;
    block.merges = repair.merges.clone();
    block.header_rows = repair.header_rows;
    for merge in &block.merges {
        anyhow::ensure!(
            merge.rowspan > 0
                && merge.colspan > 0
                && merge
                    .row
                    .checked_add(merge.rowspan)
                    .is_some_and(|end| end <= rows)
                && merge
                    .col
                    .checked_add(merge.colspan)
                    .is_some_and(|end| end <= cols),
            fail("Repaired merge is outside the new table")
        );
        for row in merge.row..merge.row + merge.rowspan {
            for col in merge.col..merge.col + merge.colspan {
                anyhow::ensure!(
                    (row == merge.row && col == merge.col) || block.rows[row][col].is_empty(),
                    fail(
                        "A merge would hide a nonempty cell; explicitly map its value into the anchor first"
                    )
                );
            }
        }
    }
    block.bbox = Some(BoxRect {
        left: grid.column_edges[0],
        top: grid.row_edges[0],
        right: *grid.column_edges.last().unwrap(),
        bottom: *grid.row_edges.last().unwrap(),
    });
    data.grid = Some(grid);
    block.geometry = Some(data);
    block.uncertain = true;
    next.reviewed = false;
    next.content.summary = next.content.plain().chars().take(2000).collect();
    next.validate()?;
    let before_fingerprint = fingerprint(expected)?;
    let token = grid_token(&source, &before_fingerprint, &next)?;
    Ok(GridPreview{source,before_fingerprint,after:next,token,note:"Preview only. Source row/column edges are human-confirmed. Every old cell is mapped exactly once; combined values retain supplied order with newlines. New split cells are empty. OCR evidence and immutable source pixels are unchanged.".into()})
}
pub fn apply_grid(
    project: &Project,
    expected: &Page,
    preview: &GridPreview,
    token: &str,
    c: &Control,
) -> Result<u64> {
    let source = checked_source(project, expected, c)?;
    anyhow::ensure!(
        source == preview.source
            && fingerprint(expected)? == preview.before_fingerprint
            && token == preview.token
            && token == grid_token(&source, &preview.before_fingerprint, &preview.after)?
            && preview.after.page == expected.page
            && preview.after.image == expected.image
            && preview.after.confirmed_redactions == expected.confirmed_redactions
            && fingerprint(&preview.after.candidates)? == fingerprint(&expected.candidates)?,
        fail("Confirm the exact fresh table-grid preview")
    );
    preview.after.validate()?;
    c.check()?;
    crate::review::commit(
        project,
        expected,
        &preview.after,
        "Repair source table grid",
    )
}
