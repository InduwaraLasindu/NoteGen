use notegen::{
    searchable_pdf::{self, Options},
    storage::{Control, Manifest, Project},
    types::*,
};
use pdfium_render::prelude::*;
use std::{fs, path::PathBuf};
struct Fixture {
    _temp: tempfile::TempDir,
    project: Project,
    output: PathBuf,
    image: image::RgbImage,
}
fn library() -> PathBuf {
    std::env::var_os("NOTEGEN_PDFIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("runtime"))
}
fn pdfium() -> Pdfium {
    let lib = library();
    let lib = if lib.is_file() {
        lib
    } else {
        lib.join(Pdfium::pdfium_platform_library_name())
    };
    let result = if lib.is_file() {
        Pdfium::bind_to_library(lib)
    } else {
        Pdfium::bind_to_system_library()
    };
    match result {
        Ok(binding) => Pdfium::new(binding),
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Pdfium::default(),
        Err(e) => panic!("Native PDFium is required for searchable PDF tests: {e}"),
    }
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let settings = Settings {
        pdfium: library(),
        ..Default::default()
    };
    let project = Project::open(
        temp.path().join("project"),
        Manifest {
            schema: 2,
            input_sha256: "source".into(),
            fingerprint: "settings".into(),
            pages: vec![1, 2],
            models: vec![],
            settings,
        },
    )
    .unwrap();
    let mut image = image::RgbImage::from_pixel(240, 320, image::Rgb([255, 255, 255]));
    for y in 100..220 {
        for x in 80..160 {
            image.put_pixel(x, y, image::Rgb([20, 100, 180]));
        }
    }
    for n in [1, 2] {
        image.save(project.image(n)).unwrap();
        project
            .save(&Page {
                page: n,
                image: format!("page-{n:05}.png"),
                agreement: 1.0,
                content: PageContent {
                    blocks: vec![Block {
                        kind: "text".into(),
                        text: format!("Search {n}250 Ω é"),
                        bbox: Some(BoxRect {
                            left: 0.1,
                            top: 0.1,
                            right: 0.9,
                            bottom: 0.25,
                        }),
                        confidence: Some(0.99),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
    }
    let output = temp.path().join("searchable.pdf");
    Fixture {
        _temp: temp,
        project,
        output,
        image,
    }
}
fn options() -> Options {
    Options {
        font: searchable_pdf::default_font(),
        ..Default::default()
    }
}
#[test]
fn native_searchable_export_has_source_pages_unicode_embedded_font_and_grounded_text() {
    let f = fixture();
    let report =
        searchable_pdf::export(&f.project, &f.output, &options(), &Control::default()).unwrap();
    assert_eq!(report.pages, 2);
    assert_eq!(report.searchable_blocks, 2);
    assert_eq!(report.image_only_blocks, 0);
    assert!(report.missing_glyphs.is_empty());
    assert!(report.alignment.contains("block"));
    let engine = pdfium();
    let document = engine.load_pdf_from_file(&f.output, None).unwrap();
    assert_eq!(document.pages().len(), 2);
    let page = document.pages().get(0).unwrap();
    let text = page.text().unwrap().all();
    assert!(text.contains("Search 1250"), "{text:?}");
    assert!(text.contains('Ω'), "{text:?}");
    assert!(text.contains('é'), "{text:?}");
    let mut texts = 0;
    let mut images = 0;
    for object in page.objects().iter() {
        if let Some(text) = object.as_text_object() {
            texts += 1;
            assert_eq!(text.render_mode(), PdfPageTextRenderMode::Invisible);
            assert!(text.font().is_embedded().unwrap());
            let bounds = text.bounds().unwrap();
            let width = page.width().value;
            let height = page.height().value;
            assert!((bounds.left().value - width * 0.1).abs() < 0.2);
            assert!((bounds.bottom().value - height * 0.75).abs() < 0.2);
        }
        if let Some(image) = object.as_image_object() {
            images += 1;
            assert_eq!(image.get_raw_image().unwrap().to_rgb8(), f.image);
        }
    }
    assert_eq!((texts, images), (1, 1));
    let rendered = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(240)
                .set_target_height(320)
                .render_annotations(false),
        )
        .unwrap()
        .as_image()
        .to_rgb8();
    assert_eq!(rendered.get_pixel(120, 160), f.image.get_pixel(120, 160));
    assert_eq!(rendered.get_pixel(20, 20), f.image.get_pixel(20, 20));
}
#[test]
fn uncertainty_annotations_are_visible_metadata_and_preserve_the_source_image() {
    let f = fixture();
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks[0].uncertain = true;
    p.save(&page).unwrap();
    let report = searchable_pdf::export(p, &f.output, &options(), &Control::default()).unwrap();
    assert_eq!(report.marked_regions, 1);
    let engine = pdfium();
    let document = engine.load_pdf_from_file(&f.output, None).unwrap();
    let page = document.pages().get(0).unwrap();
    assert_eq!(page.annotations().len(), 1);
    let annotation = page.annotations().get(0).unwrap();
    assert!(annotation.contents().unwrap().contains("Review needed"));
    assert!(annotation.bounds().unwrap().width().value > 0.0);
    for object in page.objects().iter() {
        if let Some(image) = object.as_image_object() {
            assert_eq!(image.get_raw_image().unwrap().to_rgb8(), f.image);
        }
    }
}
#[test]
fn missing_source_boxes_remain_image_only_without_fabricated_coordinates() {
    let f = fixture();
    let mut page = f.project.load(1).unwrap();
    page.content.blocks[0].bbox = None;
    f.project.save(&page).unwrap();
    let report =
        searchable_pdf::export(&f.project, &f.output, &options(), &Control::default()).unwrap();
    assert_eq!(report.image_only_blocks, 1);
    assert_eq!(report.searchable_blocks, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.page == 1 && w.message.contains("No measured source box"))
    );
    let engine = pdfium();
    let document = engine.load_pdf_from_file(&f.output, None).unwrap();
    assert!(
        !document
            .pages()
            .get(0)
            .unwrap()
            .text()
            .unwrap()
            .all()
            .contains("Search 1250")
    );
}
#[test]
fn unsupported_font_characters_are_reported_and_can_require_strict_abort() {
    let f = fixture();
    let mut page = f.project.load(1).unwrap();
    page.content.blocks[0].text.push('\u{10ffff}');
    f.project.save(&page).unwrap();
    let mut strict = options();
    strict.strict_coverage = true;
    assert!(searchable_pdf::export(&f.project, &f.output, &strict, &Control::default()).is_err());
    assert!(!f.output.exists());
    let report =
        searchable_pdf::export(&f.project, &f.output, &options(), &Control::default()).unwrap();
    assert!(report.missing_glyphs.contains(&"\u{10ffff}".into()));
    assert_eq!(report.image_only_blocks, 1);
    assert_eq!(report.pages, 2);
}
#[test]
fn existing_output_cancellation_bad_options_and_missing_images_never_publish_partial_pdf() {
    let f = fixture();
    fs::write(&f.output, b"existing output").unwrap();
    assert!(
        searchable_pdf::export(&f.project, &f.output, &options(), &Control::default()).is_err()
    );
    assert_eq!(fs::read(&f.output).unwrap(), b"existing output");
    fs::remove_file(&f.output).unwrap();
    let control = Control::default();
    control
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(searchable_pdf::export(&f.project, &f.output, &options(), &control).is_err());
    assert!(!f.output.exists());
    let mut invalid = options();
    invalid.dpi = 0;
    assert!(searchable_pdf::export(&f.project, &f.output, &invalid, &Control::default()).is_err());
    assert!(!f.output.exists());
    fs::remove_file(f.project.image(2)).unwrap();
    assert!(
        searchable_pdf::export(&f.project, &f.output, &options(), &Control::default()).is_err()
    );
    assert!(!f.output.exists());
}
#[test]
fn raster_budget_applies_to_the_whole_document_and_annotations_are_optional() {
    let f = fixture();
    let mut page = f.project.load(1).unwrap();
    page.content.blocks[0].uncertain = true;
    f.project.save(&page).unwrap();
    let mut selected = options();
    selected.mark_uncertain = false;
    let report =
        searchable_pdf::export(&f.project, &f.output, &selected, &Control::default()).unwrap();
    assert_eq!(report.marked_regions, 0);
    let mut invalid = selected;
    invalid.max_total_pixels = 160_000_001;
    assert!(invalid.validate().is_err());
    let second = fixture();
    let larger = image::RgbImage::from_pixel(1200, 600, image::Rgb([255, 255, 255]));
    for n in [1, 2] {
        larger.save(second.project.image(n)).unwrap();
    }
    let mut limited = options();
    limited.max_total_pixels = 1_000_000;
    assert!(
        searchable_pdf::export(
            &second.project,
            &second.output,
            &limited,
            &Control::default()
        )
        .is_err()
    );
    assert!(!second.output.exists());
}
#[test]
fn figure_interpretations_do_not_enter_the_source_transcription_layer() {
    let f = fixture();
    let mut page = f.project.load(1).unwrap();
    page.content.blocks.push(Block {
        kind: "figure".into(),
        bbox: Some(BoxRect::full()),
        description: "Visible chart".into(),
        interpretation: "Speculative relationship".into(),
        ..Default::default()
    });
    f.project.save(&page).unwrap();
    let report =
        searchable_pdf::export(&f.project, &f.output, &options(), &Control::default()).unwrap();
    assert_eq!(report.image_only_blocks, 1);
    let engine = pdfium();
    let document = engine.load_pdf_from_file(&f.output, None).unwrap();
    let text = document.pages().get(0).unwrap().text().unwrap().all();
    assert!(!text.contains("Speculative relationship"));
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.message.contains("interpretations"))
    );
}
