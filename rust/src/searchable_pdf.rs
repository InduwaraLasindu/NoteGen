//! Source image PDFs with a searchable layer grounded only to measured block boxes.
use crate::{
    storage::{Control, Project},
    types::*,
};
use ab_glyph::{Font, FontArc};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub font: Option<PathBuf>,
    /// Pixel density controls physical page size; source pixels are retained unchanged.
    pub dpi: u32,
    pub mark_uncertain: bool,
    /// Abort rather than create an image-only block when its font lacks characters.
    pub strict_coverage: bool,
    /// Bounds retained raster data across the document, not just the current page.
    pub max_total_pixels: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            font: None,
            dpi: 300,
            mark_uncertain: true,
            strict_coverage: false,
            max_total_pixels: 80_000_000,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            (72..=600).contains(&self.dpi)
                && (1_000_000..=160_000_000).contains(&self.max_total_pixels),
            Fault::new(
                "NG102",
                "PDF density must be 72–600 DPI and total raster budget 1–160 million pixels"
            )
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Warning {
    pub page: usize,
    pub block: Option<usize>,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub pages: usize,
    pub searchable_blocks: usize,
    pub image_only_blocks: usize,
    pub marked_regions: usize,
    pub font: PathBuf,
    pub missing_glyphs: Vec<String>,
    pub warnings: Vec<Warning>,
    pub omitted_warnings: usize,
    pub alignment: String,
    pub source_invalidated: bool,
    pub warning_pages: usize,
    pub font_embedding: FontEmbedding,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FontEmbedding {
    pub permission: String,
    pub subsetting_allowed: bool,
}
/// Honor the font's own OS/2 embedding metadata before loading outline data into a PDF.
pub fn font_embedding(data: &[u8]) -> Result<FontEmbedding> {
    anyhow::ensure!(
        data.len() <= 64_000_000,
        Fault::new("NG400", "PDF font exceeds 64 MB")
    );
    let face = ttf_parser::Face::parse(data, 0)
        .map_err(|_| Fault::new("NG400", "The selected TrueType font could not be parsed"))?;
    let os2 = face.tables().os2.ok_or_else(|| {
        Fault::new(
            "NG400",
            "Font embedding metadata is missing or invalid; choose another font",
        )
    })?;
    let permission = os2.permissions().ok_or_else(|| {
        Fault::new(
            "NG400",
            "Font embedding permission flags are ambiguous; choose another font",
        )
    })?;
    anyhow::ensure!(
        permission != ttf_parser::Permissions::Restricted,
        Fault::new(
            "NG400",
            "The font's metadata restricts embedding; choose another font"
        )
    );
    anyhow::ensure!(
        os2.is_outline_embedding_allowed(),
        Fault::new(
            "NG400",
            "The font's metadata permits bitmap-only embedding; searchable PDF requires permitted outlines"
        )
    );
    Ok(FontEmbedding {
        permission: format!("{permission:?}"),
        subsetting_allowed: os2.is_subsetting_allowed(),
    })
}
fn warning(report: &mut Report, page: usize, block: Option<usize>, message: &str) {
    if report.warnings.len() < 1000 {
        report.warnings.push(Warning {
            page,
            block,
            message: message.into(),
        });
    } else {
        report.omitted_warnings += 1;
    }
}
/// Choose a known local font without downloading or recursively indexing user files.
pub fn default_font() -> Option<PathBuf> {
    let windows = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Windows"));
    [
        windows.join("Fonts/arial.ttf"),
        windows.join("Fonts/Nirmala.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf"),
        PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}
fn bind(library: &Path) -> Result<Pdfium> {
    let path = if library.is_file() {
        library.to_path_buf()
    } else {
        library.join(Pdfium::pdfium_platform_library_name())
    };
    let result = if path.is_file() {
        Pdfium::bind_to_library(path)
    } else {
        Pdfium::bind_to_system_library()
    };
    Ok(match result {
        Ok(bindings) => Pdfium::new(bindings),
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Pdfium::default(),
        Err(e) => {
            return Err(Fault::new(
                "NG400",
                format!("PDFium could not create the searchable PDF: {e}"),
            )
            .into());
        }
    })
}
fn rect(region: &BoxRect, width: f32, height: f32) -> PdfRect {
    PdfRect::new_from_values(
        (1.0 - region.bottom) * height,
        region.left * width,
        (1.0 - region.top) * height,
        region.right * width,
    )
}
fn note(page: &mut PdfPage<'_>, bounds: PdfRect, message: &str) -> Result<()> {
    let mut annotation = page.annotations_mut().create_square_annotation()?;
    annotation.set_bounds(bounds)?;
    annotation.set_stroke_color(PdfColor::new(230, 120, 0, 255))?;
    annotation.set_fill_color(PdfColor::new(255, 255, 255, 0))?;
    annotation.set_contents(message)?;
    annotation.set_creator("NoteGen")?;
    Ok(())
}
/// Export the saved source images; ungrounded or unsupported text remains image-only and is reported.
/// The searchable geometry is one measured block rectangle, never invented word/line coordinates.
pub fn export(project: &Project, target: &Path, options: &Options, c: &Control) -> Result<Report> {
    crate::exports::output_parent(project, target)?;
    anyhow::ensure!(
        !target.exists(),
        Fault::new("NG401", "Output already exists")
    );
    crate::redactions::export_projected(project, target, c, |visible, output| {
        export_inner(
            visible,
            output,
            options,
            c,
            &project.manifest.settings.pdfium,
        )
    })
}
fn export_inner(
    project: &Project,
    target: &Path,
    options: &Options,
    c: &Control,
    pdfium_path: &Path,
) -> Result<Report> {
    options.validate()?;
    c.check()?;
    anyhow::ensure!(
        target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf")),
        Fault::new("NG400", "Choose a .pdf output")
    );
    anyhow::ensure!(
        !target.exists(),
        Fault::new("NG401", "Output already exists")
    );
    let font_path = options
        .font
        .clone()
        .or_else(default_font)
        .ok_or_else(|| {
            Fault::new(
                "NG400",
                "Choose a local TrueType font for searchable PDF export",
            )
        })?
        .canonicalize()?;
    let info = fs::metadata(&font_path)?;
    anyhow::ensure!(
        info.is_file() && info.len() <= 64_000_000,
        Fault::new(
            "NG400",
            "PDF font must be a TrueType file no larger than 64 MB"
        )
    );
    let bytes = fs::read(&font_path)?;
    let embedding = font_embedding(&bytes)?;
    let coverage = FontArc::try_from_vec(bytes.clone())
        .map_err(|_| Fault::new("NG400", "The selected PDF font could not be read"))?;
    let pdfium = bind(pdfium_path)?;
    let mut document = pdfium.create_new_pdf()?;
    let font = document
        .fonts_mut()
        .load_true_type_from_bytes(&bytes, true)?;
    if !embedding.subsetting_allowed {
        let loaded = document
            .fonts()
            .get(font)
            .ok_or_else(|| Fault::new("NG400", "Full font embedding could not be verified"))?;
        anyhow::ensure!(
            loaded.is_embedded()? && loaded.data()? == bytes,
            Fault::new(
                "NG400",
                "The font forbids subsetting and full font embedding could not be verified; choose another font"
            )
        );
    }
    let invalidated = project.root.join("input-invalidated.json").is_file();
    let mut report=Report{pages:0,searchable_blocks:0,image_only_blocks:0,marked_regions:0,font:font_path,missing_glyphs:vec![],warnings:vec![],omitted_warnings:0,alignment:"Measured source block rectangles. Word and line coordinates are not inferred; selection within a multiline block is approximate.".into(),source_invalidated:invalidated,warning_pages:0,font_embedding:embedding};
    if invalidated {
        let warning_font = document.fonts_mut().helvetica_bold();
        let body_font = document.fonts_mut().helvetica();
        let mut cover = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::a4())?;
        let mut heading = cover.objects_mut().create_text_object(
            PdfPoints::new(44.0),
            PdfPoints::new(770.0),
            crate::word::INVALIDATED_SOURCE_WARNING,
            warning_font,
            PdfPoints::new(16.0),
        )?;
        heading.set_fill_color(PdfColor::new(198, 40, 40, 255))?;
        for (line, message) in [
            "Manual recovery export of preserved recognition.",
            "The input changed or became unavailable during conversion.",
            "Verify every following source page against the original input.",
        ]
        .iter()
        .enumerate()
        {
            cover.objects_mut().create_text_object(
                PdfPoints::new(44.0),
                PdfPoints::new(730.0 - line as f32 * 22.0),
                message,
                body_font,
                PdfPoints::new(12.0),
            )?;
        }
        report.pages += 1;
        report.warning_pages = 1;
        warning(&mut report, 0, None, crate::word::RECOVERY_EXPORT_NOTICE);
    }
    let mut missing = BTreeSet::new();
    let mut pixels = 0u64;
    let mut characters = 0usize;
    let mut manual_annotations = 0usize;
    for n in &project.manifest.pages {
        c.check()?;
        let source = project.load(*n)?;
        let path = crate::word::safe_image(&project.root, &source.image)?;
        let (image_width, image_height) = image::image_dimensions(&path)?;
        pixels = pixels
            .checked_add(u64::from(image_width) * u64::from(image_height))
            .ok_or_else(|| Fault::new("NG400", "PDF raster size overflow"))?;
        anyhow::ensure!(
            pixels <= options.max_total_pixels,
            Fault::new(
                "NG400",
                "Searchable PDF exceeds the total raster budget; export fewer project pages"
            )
        );
        let image = crate::writers::open_image(&path, 40_000_000)?;
        let width = image.width() as f32 * 72.0 / options.dpi as f32;
        let height = image.height() as f32 * 72.0 / options.dpi as f32;
        let mut page = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::new_custom(
                PdfPoints::new(width),
                PdfPoints::new(height),
            ))?;
        page.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
        page.objects_mut().create_image_object(
            PdfPoints::ZERO,
            PdfPoints::ZERO,
            &image,
            Some(PdfPoints::new(width)),
            Some(PdfPoints::new(height)),
        )?;
        // Preserve human labels as source-bound annotations without changing source pixels or text.
        for layer in crate::layers::report(&source)? {
            c.check()?;
            manual_annotations += 1;
            anyhow::ensure!(
                manual_annotations <= 10_000,
                Fault::new(
                    "NG400",
                    "PDF exceeds 10,000 manual annotation labels; export fewer pages"
                )
            );
            note(
                &mut page,
                rect(
                    &layer.source_region.clone().unwrap_or_else(BoxRect::full),
                    width,
                    height,
                ),
                &layer.caption(),
            )?;
            report.marked_regions += 1;
            if !layer.confirmed() {
                warning(&mut report, *n, None, &layer.caption());
            }
        }
        let mut full_page_marked = false;
        let marks_before = report.marked_regions;
        for (index, b) in source.content.blocks.iter().enumerate() {
            c.check()?;
            let uncertain = !source.reviewed
                && (b.uncertain
                    || b.kind == "figure"
                    || b.confidence
                        .is_none_or(|v| v < project.manifest.settings.review_threshold)
                    || b.plain().to_lowercase().contains("[illegible]"));
            if options.mark_uncertain && uncertain {
                let bounds = if let Some(bbox) = &b.bbox {
                    Some(rect(bbox, width, height))
                } else if !full_page_marked {
                    full_page_marked = true;
                    Some(rect(&BoxRect::full(), width, height))
                } else {
                    None
                };
                if let Some(bounds) = bounds {
                    note(
                        &mut page,
                        bounds,
                        &format!(
                            "Review needed: source page {}, block {}. Compare recognition and reader evidence in NoteGen.",
                            n,
                            index + 1
                        ),
                    )?;
                    report.marked_regions += 1;
                }
            }
            if b.kind == "figure" {
                report.image_only_blocks += 1;
                warning(
                    &mut report,
                    *n,
                    Some(index),
                    "Figure observations and contextual interpretations are excluded from the source transcription text layer.",
                );
                continue;
            }
            let text = crate::roundtrip::clean(&b.plain())
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if text.is_empty() {
                continue;
            }
            let Some(region) = &b.bbox else {
                report.image_only_blocks += 1;
                warning(
                    &mut report,
                    *n,
                    Some(index),
                    "No measured source box is available; this block remains image-only.",
                );
                continue;
            };
            let absent: BTreeSet<_> = text
                .chars()
                .filter(|ch| !ch.is_whitespace() && coverage.glyph_id(*ch).0 == 0)
                .collect();
            if !absent.is_empty() {
                missing.extend(absent);
                anyhow::ensure!(
                    !options.strict_coverage,
                    Fault::new(
                        "NG400",
                        format!(
                            "The PDF font lacks characters on page {n}, block {}; choose a covering font",
                            index + 1
                        )
                    )
                );
                report.image_only_blocks += 1;
                warning(
                    &mut report,
                    *n,
                    Some(index),
                    "The selected font lacks characters in this block. The source is retained; choose a covering font to make this block searchable.",
                );
                continue;
            }
            characters = characters
                .checked_add(text.chars().count())
                .ok_or_else(|| Fault::new("NG400", "PDF text size overflow"))?;
            anyhow::ensure!(
                characters <= 4_000_000,
                Fault::new(
                    "NG400",
                    "PDF searchable layer exceeds four million characters; export fewer pages"
                )
            );
            let mut object = PdfPageTextObject::new(&document, &text, font, PdfPoints::new(10.0))?;
            object.set_render_mode(PdfPageTextRenderMode::Invisible)?;
            let initial = object.bounds()?;
            if initial.width().value <= 0.0 || initial.height().value <= 0.0 {
                report.image_only_blocks += 1;
                warning(
                    &mut report,
                    *n,
                    Some(index),
                    "The font could not provide text bounds; the source block remains image-only.",
                );
                continue;
            }
            let bounds = rect(region, width, height);
            object.scale(
                bounds.width().value / initial.width().value,
                bounds.height().value / initial.height().value,
            )?;
            let current = object.bounds()?;
            object.translate(
                PdfPoints::new(bounds.left().value - current.left().value),
                PdfPoints::new(bounds.bottom().value - current.bottom().value),
            )?;
            page.objects_mut().add_text_object(object)?;
            report.searchable_blocks += 1;
        }
        if options.mark_uncertain
            && source.needs_review(project.manifest.settings.review_threshold)
            && report.marked_regions == marks_before
        {
            note(
                &mut page,
                rect(&BoxRect::full(), width, height),
                "This source page has unresolved recognition or scan warnings. Review it in NoteGen.",
            )?;
            report.marked_regions += 1;
        }
        page.regenerate_content()?;
        report.pages += 1;
    }
    report.missing_glyphs = missing.into_iter().map(|c| c.to_string()).collect();
    c.check()?;
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    document.save_to_writer(temp.as_file_mut())?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    if !report.font_embedding.subsetting_allowed {
        let verification = pdfium.load_pdf_from_file(temp.path(), None)?;
        for index in report.warning_pages as i32..verification.pages().len() {
            c.check()?;
            let page = verification.pages().get(index)?;
            for object in page.objects().iter() {
                if let Some(text) = object.as_text_object() {
                    let font = text.font();
                    anyhow::ensure!(
                        font.is_embedded()? && font.data()? == bytes,
                        Fault::new(
                            "NG400",
                            "The font forbids subsetting and saved full font embedding could not be verified; choose another font"
                        )
                    );
                }
            }
        }
    }
    c.check()?;
    temp.persist_noclobber(target)
        .map_err(|_| Fault::new("NG401", "PDF output collision or unavailable output folder"))?;
    Ok(report)
}
