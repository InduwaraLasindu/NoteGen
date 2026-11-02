//! Bounded, non-destructive scan tools. Every operation returns a new image.
use crate::{storage::Control, types::*};
use image::{DynamicImage, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

pub const MAX_PIXELS: u64 = 40_000_000;
fn dimensions(width: u32, height: u32) -> Result<()> {
    anyhow::ensure!(
        width > 0
            && height > 0
            && width <= 20000
            && height <= 20000
            && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        Fault::new(
            "NG100",
            "Scan dimensions exceed the 40 million pixel / 20,000 pixel edge limit"
        )
    );
    Ok(())
}
fn checked(image: &DynamicImage, c: &Control) -> Result<()> {
    c.check()?;
    dimensions(image.width(), image.height())
}
fn light(p: &Rgb<u8>) -> u8 {
    ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
fn validate_corners(corners: &[Point; 4]) -> Result<()> {
    anyhow::ensure!(
        corners.iter().all(|p| p.x.is_finite()
            && p.y.is_finite()
            && (0.0..=1.0).contains(&p.x)
            && (0.0..=1.0).contains(&p.y)),
        Fault::new(
            "NG100",
            "Perspective corners must be finite normalized coordinates"
        )
    );
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        let d = corners[(i + 2) % 4];
        let cross = (b.x - a.x) * (d.y - b.y) - (b.y - a.y) * (d.x - b.x);
        anyhow::ensure!(
            cross > 0.000_001,
            Fault::new(
                "NG100",
                "Select a nondegenerate convex quadrilateral in top-left, top-right, bottom-right, bottom-left order"
            )
        );
    }
    Ok(())
}
/// Maps an editable source quadrilateral onto a rectangle with bilinear sampling.
pub fn perspective(
    image: &DynamicImage,
    corners: [Point; 4],
    width: u32,
    height: u32,
    c: &Control,
) -> Result<DynamicImage> {
    checked(image, c)?;
    dimensions(width, height)?;
    anyhow::ensure!(
        width >= 2 && height >= 2,
        Fault::new("NG100", "Perspective output must be at least 2×2 pixels")
    );
    validate_corners(&corners)?;
    let [p0, p1, p2, p3] = corners;
    let dx1 = p1.x - p2.x;
    let dx2 = p3.x - p2.x;
    let dx3 = p0.x - p1.x + p2.x - p3.x;
    let dy1 = p1.y - p2.y;
    let dy2 = p3.y - p2.y;
    let dy3 = p0.y - p1.y + p2.y - p3.y;
    let (g, h) = if dx3.abs() + dy3.abs() < 1e-12 {
        (0., 0.)
    } else {
        let denominator = dx1 * dy2 - dx2 * dy1;
        anyhow::ensure!(
            denominator.abs() > 1e-12,
            Fault::new("NG100", "Perspective corners are numerically degenerate")
        );
        (
            (dx3 * dy2 - dx2 * dy3) / denominator,
            (dx1 * dy3 - dx3 * dy1) / denominator,
        )
    };
    let a = p1.x - p0.x + g * p1.x;
    let b = p3.x - p0.x + h * p3.x;
    let d = p1.y - p0.y + g * p1.y;
    let e = p3.y - p0.y + h * p3.y;
    let source = image.to_rgb8();
    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        c.check()?;
        let v = f64::from(y) / f64::from(height - 1);
        for x in 0..width {
            let u = f64::from(x) / f64::from(width - 1);
            let denominator = g * u + h * v + 1.;
            anyhow::ensure!(
                denominator.abs() > 1e-12,
                Fault::new("NG100", "Perspective transform crosses infinity")
            );
            let sx = ((a * u + b * v + p0.x) / denominator * f64::from(source.width() - 1))
                .clamp(0., f64::from(source.width() - 1));
            let sy = ((d * u + e * v + p0.y) / denominator * f64::from(source.height() - 1))
                .clamp(0., f64::from(source.height() - 1));
            let x0 = sx.floor() as u32;
            let y0 = sy.floor() as u32;
            let x1 = (x0 + 1).min(source.width() - 1);
            let y1 = (y0 + 1).min(source.height() - 1);
            let fx = sx - f64::from(x0);
            let fy = sy - f64::from(y0);
            let mut pixel = [0u8; 3];
            for (channel, value) in pixel.iter_mut().enumerate() {
                let top = f64::from(source[(x0, y0)][channel]) * (1. - fx)
                    + f64::from(source[(x1, y0)][channel]) * fx;
                let bottom = f64::from(source[(x0, y1)][channel]) * (1. - fx)
                    + f64::from(source[(x1, y1)][channel]) * fx;
                *value = (top * (1. - fy) + bottom * fy).round().clamp(0., 255.) as u8;
            }
            output[(x, y)] = Rgb(pixel);
        }
    }
    Ok(DynamicImage::ImageRgb8(output))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanupOptions {
    pub shadow_variant: bool,
    pub pale_ruling_variant: bool,
    pub horizontal_rules: bool,
    pub vertical_rules: bool,
    pub line_coverage: f32,
}
impl Default for CleanupOptions {
    fn default() -> Self {
        Self {
            shadow_variant: true,
            pale_ruling_variant: false,
            horizontal_rules: true,
            vertical_rules: true,
            line_coverage: 0.8,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeReport {
    pub id: String,
    pub description: String,
    pub changed_pixels: u64,
    pub dark_ink_before: u64,
    pub dark_ink_after: u64,
    pub mean_luminance_before: f64,
    pub mean_luminance_after: f64,
    pub warnings: Vec<String>,
}
pub struct Variant {
    pub image: DynamicImage,
    pub report: ChangeReport,
}
pub struct Comparison {
    pub width: u32,
    pub height: u32,
    pub variants: Vec<Variant>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComparisonManifest {
    pub schema: u32,
    pub width: u32,
    pub height: u32,
    pub files: Vec<String>,
    pub reports: Vec<ChangeReport>,
    pub note: String,
}
fn report(
    original: &RgbImage,
    changed: &RgbImage,
    id: &str,
    description: &str,
    c: &Control,
) -> Result<ChangeReport> {
    let mut result=ChangeReport{id:id.into(),description:description.into(),changed_pixels:0,dark_ink_before:0,dark_ink_after:0,mean_luminance_before:0.,mean_luminance_after:0.,warnings:vec!["Compare with original.png. Cleanup cannot recover missing or glare-obscured ink and can alter faint marks.".into()]};
    for y in 0..original.height() {
        c.check()?;
        for x in 0..original.width() {
            let before = original[(x, y)];
            let after = changed[(x, y)];
            if before != after {
                result.changed_pixels += 1;
            }
            let a = light(&before);
            let b = light(&after);
            result.dark_ink_before += u64::from(a < 96);
            result.dark_ink_after += u64::from(b < 96);
            result.mean_luminance_before += f64::from(a);
            result.mean_luminance_after += f64::from(b);
        }
    }
    let pixels = f64::from(original.width()) * f64::from(original.height());
    result.mean_luminance_before /= pixels;
    result.mean_luminance_after /= pixels;
    if result.dark_ink_after.saturating_mul(100) < result.dark_ink_before.saturating_mul(95) {
        result.warnings.push("More than 5% of dark pixels changed to lighter values; inspect possible ink loss before recognition.".into());
    }
    Ok(result)
}
fn flatten(source: &RgbImage, c: &Control) -> Result<RgbImage> {
    // A bounded tile grid estimates paper from its 90th luminance percentile.
    // Black ink remains black; no textual details are synthesized.
    let tile = 64u32;
    let columns = source.width().div_ceil(tile);
    let rows = source.height().div_ceil(tile);
    let mut background = vec![255u8; (columns * rows) as usize];
    for ty in 0..rows {
        c.check()?;
        for tx in 0..columns {
            let mut histogram = [0u32; 256];
            let mut count = 0u32;
            for y in ty * tile..((ty + 1) * tile).min(source.height()) {
                for x in tx * tile..((tx + 1) * tile).min(source.width()) {
                    histogram[light(&source[(x, y)]) as usize] += 1;
                    count += 1;
                }
            }
            let mut sum = 0;
            for (level, n) in histogram.iter().enumerate() {
                sum += n;
                if sum >= (count * 9).div_ceil(10) {
                    background[(ty * columns + tx) as usize] = level.max(32) as u8;
                    break;
                }
            }
        }
    }
    let mut output = source.clone();
    for y in 0..source.height() {
        c.check()?;
        let gy = (f64::from(y) / 64. - 0.5).clamp(0., f64::from(rows - 1));
        let y0 = gy.floor() as u32;
        let y1 = (y0 + 1).min(rows - 1);
        let fy = gy - f64::from(y0);
        for x in 0..source.width() {
            let gx = (f64::from(x) / 64. - 0.5).clamp(0., f64::from(columns - 1));
            let x0 = gx.floor() as u32;
            let x1 = (x0 + 1).min(columns - 1);
            let fx = gx - f64::from(x0);
            let level = |xx, yy| f64::from(background[(yy * columns + xx) as usize]);
            let bg = (level(x0, y0) * (1. - fx) + level(x1, y0) * fx) * (1. - fy)
                + (level(x0, y1) * (1. - fx) + level(x1, y1) * fx) * fy;
            for channel in 0..3 {
                output[(x, y)][channel] = (f64::from(source[(x, y)][channel]) * 245. / bg)
                    .round()
                    .clamp(0., 255.) as u8;
            }
        }
    }
    Ok(output)
}
fn suppress_rules(source: &RgbImage, options: &CleanupOptions, c: &Control) -> Result<RgbImage> {
    let pale = |p: &Rgb<u8>| (120..=230).contains(&light(p));
    let mut horizontal = vec![false; source.height() as usize];
    let mut vertical = vec![false; source.width() as usize];
    if options.horizontal_rules {
        for y in 0..source.height() {
            c.check()?;
            let count = (0..source.width())
                .filter(|x| pale(&source[(*x, y)]))
                .count();
            horizontal[y as usize] = count as f32 / source.width() as f32 >= options.line_coverage;
        }
    }
    if options.vertical_rules {
        for x in 0..source.width() {
            c.check()?;
            let count = (0..source.height())
                .filter(|y| pale(&source[(x, *y)]))
                .count();
            vertical[x as usize] = count as f32 / source.height() as f32 >= options.line_coverage;
        }
    }
    let mut output = source.clone();
    for y in 0..source.height() {
        c.check()?;
        for x in 0..source.width() {
            if !(horizontal[y as usize] || vertical[x as usize]) || !pale(&source[(x, y)]) {
                continue;
            }
            let protected = (y.saturating_sub(2)..=(y + 2).min(source.height() - 1)).any(|yy| {
                (x.saturating_sub(2)..=(x + 2).min(source.width() - 1))
                    .any(|xx| light(&source[(xx, yy)]) < 96)
            });
            if !protected {
                output[(x, y)] = Rgb([255; 3]);
            }
        }
    }
    Ok(output)
}
pub fn compare(image: &DynamicImage, options: &CleanupOptions, c: &Control) -> Result<Comparison> {
    checked(image, c)?;
    anyhow::ensure!(
        options.line_coverage.is_finite() && (0.65..=1.0).contains(&options.line_coverage),
        Fault::new("NG102", "Pale ruling coverage must be 0.65–1.0")
    );
    let source = image.to_rgb8();
    let original_report = report(&source, &source, "original", "Unmodified source pixels", c)?;
    let mut variants = vec![Variant {
        image: image.clone(),
        report: original_report,
    }];
    if options.shadow_variant {
        let output = flatten(&source, c)?;
        let metrics = report(
            &source,
            &output,
            "lighting",
            "Non-destructive paper-lighting normalization using a coarse background estimate",
            c,
        )?;
        variants.push(Variant {
            image: DynamicImage::ImageRgb8(output),
            report: metrics,
        });
    }
    if options.pale_ruling_variant {
        let output = suppress_rules(&source, options, c)?;
        let mut metrics = report(
            &source,
            &output,
            "pale-ruling",
            "Suppress only long pale rows/columns; protect dark ink and its immediate neighbors",
            c,
        )?;
        metrics.warnings.push("Pale diagram lines, highlights or annotations may resemble ruling. Dark grid lines are intentionally retained. This is a separate optional variant.".into());
        variants.push(Variant {
            image: DynamicImage::ImageRgb8(output),
            report: metrics,
        });
    }
    Ok(Comparison {
        width: image.width(),
        height: image.height(),
        variants,
    })
}
pub fn write_image(image: &DynamicImage, output: &Path, c: &Control) -> Result<()> {
    checked(image, c)?;
    anyhow::ensure!(
        output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
        Fault::new("NG400", "Choose a new .png image")
    );
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Output image already exists")
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    image.write_to(&mut temp, image::ImageFormat::Png)?;
    c.check()?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output)
        .map_err(|_| Fault::new("NG401", "Image collision or unavailable output folder"))?;
    Ok(())
}
fn publish_folder(stage: &Path, output: &Path) -> Result<()> {
    fs::create_dir(output)
        .map_err(|_| Fault::new("NG401", "Output folder already exists or is unavailable"))?;
    if let Err(error) = fs::rename(stage, output.join("files")) {
        let _ = fs::remove_dir(output);
        return Err(error.into());
    }
    Ok(())
}
pub fn export_comparison(
    comparison: &Comparison,
    output: &Path,
    c: &Control,
) -> Result<ComparisonManifest> {
    c.check()?;
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Comparison output folder already exists")
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    let mut manifest=ComparisonManifest{schema:1,width:comparison.width,height:comparison.height,files:vec![],reports:vec![],note:"All variants derive from original.png. Files are independent copies. Compare possible faint-ink changes before selecting a variant for recognition.".into()};
    let mut names = std::collections::HashSet::new();
    for variant in &comparison.variants {
        anyhow::ensure!(
            ["original", "lighting", "pale-ruling"].contains(&variant.report.id.as_str())
                && names.insert(variant.report.id.clone()),
            Fault::new("NG400", "Invalid or duplicate comparison variant ID")
        );
        anyhow::ensure!(
            variant.image.width() == comparison.width
                && variant.image.height() == comparison.height,
            Fault::new("NG400", "Comparison variant dimensions differ")
        );
        let name = format!("{}.png", variant.report.id);
        write_image(&variant.image, &stage.path().join(&name), c)?;
        manifest.files.push(format!("files/{name}"));
        manifest.reports.push(variant.report.clone());
    }
    anyhow::ensure!(
        names.contains("original"),
        Fault::new("NG400", "Comparison must retain an original copy")
    );
    crate::storage::atomic_json(&stage.path().join("comparison.json"), &manifest)?;
    c.check()?;
    publish_folder(stage.path(), output)?;
    Ok(manifest)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SplitRelationship {
    pub schema: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub gutter_start: u32,
    pub gutter_end: u32,
    pub left_source: BoxRect,
    pub right_source: BoxRect,
    pub note: String,
}
pub struct Split {
    pub original: DynamicImage,
    pub left: DynamicImage,
    pub right: DynamicImage,
    pub relationship: SplitRelationship,
}
pub fn split_double(
    image: &DynamicImage,
    gutter_left: f32,
    gutter_right: f32,
    c: &Control,
) -> Result<Split> {
    checked(image, c)?;
    anyhow::ensure!(
        gutter_left.is_finite()
            && gutter_right.is_finite()
            && gutter_left > 0.
            && gutter_left <= gutter_right
            && gutter_right < 1.,
        Fault::new(
            "NG100",
            "Choose ordered gutter boundaries strictly inside the image"
        )
    );
    let x1 = (gutter_left * image.width() as f32).floor() as u32;
    let x2 = if gutter_left == gutter_right {
        x1
    } else {
        (gutter_right * image.width() as f32).ceil() as u32
    };
    anyhow::ensure!(
        x1 > 0 && x2 < image.width(),
        Fault::new("NG100", "Gutter boundaries would produce an empty page")
    );
    let left = image.crop_imm(0, 0, x1, image.height());
    c.check()?;
    let right = image.crop_imm(x2, 0, image.width() - x2, image.height());
    Ok(Split{original:image.clone(),left,right,relationship:SplitRelationship{schema:1,original_width:image.width(),original_height:image.height(),gutter_start:x1,gutter_end:x2,left_source:BoxRect{left:0.,top:0.,right:x1 as f32/image.width() as f32,bottom:1.},right_source:BoxRect{left:x2 as f32/image.width() as f32,top:0.,right:1.,bottom:1.},note:"Left and right are cropped copies of the retained original. The user-selected gutter is omitted from the crops; no missing writing is reconstructed.".into()}})
}
pub fn export_split(split: &Split, output: &Path, c: &Control) -> Result<()> {
    c.check()?;
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Split output folder already exists")
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    for (name, image) in [
        ("original.png", &split.original),
        ("left.png", &split.left),
        ("right.png", &split.right),
    ] {
        write_image(image, &stage.path().join(name), c)?;
    }
    crate::storage::atomic_json(&stage.path().join("relationship.json"), &split.relationship)?;
    c.check()?;
    publish_folder(stage.path(), output)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RedactionReport {
    pub redacted_regions: usize,
    pub redacted_pixels: u64,
    pub width: u32,
    pub height: u32,
    pub note: String,
}
pub fn redact(
    image: &DynamicImage,
    regions: &[BoxRect],
    c: &Control,
) -> Result<(DynamicImage, RedactionReport)> {
    checked(image, c)?;
    anyhow::ensure!(
        !regions.is_empty() && regions.len() <= 1000 && regions.iter().all(BoxRect::valid),
        Fault::new("NG100", "Choose 1–1000 explicit valid redaction boxes")
    );
    let mut output = image.to_rgb8();
    let boxes = regions
        .iter()
        .map(|r| r.pixels(image.width(), image.height()))
        .collect::<Vec<_>>();
    let mut pixels = 0;
    // Row intervals avoid double-counting overlapping redactions without a full-size mask.
    for y in 0..image.height() {
        c.check()?;
        let mut intervals = boxes
            .iter()
            .filter(|(_, top, _, bottom)| y >= *top && y < *bottom)
            .map(|(left, _, right, _)| (*left, *right))
            .collect::<Vec<_>>();
        intervals.sort_unstable();
        let mut end = 0;
        for (left, right) in intervals {
            let start = left.max(end);
            for x in start..right {
                output[(x, y)] = Rgb([0; 3]);
                pixels += 1;
            }
            end = end.max(right);
        }
    }
    Ok((DynamicImage::ImageRgb8(output),RedactionReport{redacted_regions:regions.len(),redacted_pixels:pixels,width:image.width(),height:image.height(),note:"Opaque black pixels replace only the user-selected regions. The PNG copy contains no original image metadata or embedded source. Review the complete copy for other identifying content before manually sharing. Recognized text/project metadata require separate redaction.".into()}))
}
pub fn export_redacted(
    image: &DynamicImage,
    regions: &[BoxRect],
    output: &Path,
    c: &Control,
) -> Result<RedactionReport> {
    let (copy, report) = redact(image, regions, c)?;
    write_image(&copy, output, c)?;
    Ok(report)
}
