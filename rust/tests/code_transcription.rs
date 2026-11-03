use ab_glyph::{Font, FontArc, ScaleFont};
use notegen::{
    code_transcription::{self, CodeOptions, Indentation},
    review,
    storage::{Control, Manifest, Project},
    types::*,
};
use std::{fs, path::Path};

#[test]
fn native_printed_code_has_calibrated_indentation_literal_evidence_and_source_bound_undo() {
    let font_path = [
        "C:/Windows/Fonts/consola.ttf",
        "C:/Windows/Fonts/cour.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .expect("native code OCR requires a system monospace font");
    let font = FontArc::try_from_vec(fs::read(font_path).unwrap()).unwrap();
    let pitch = font.as_scaled(48.).h_advance(font.glyph_id(' '));
    let root = tempfile::tempdir().unwrap();
    let settings = Settings {
        preset: Preset::Fast,
        models_dir: root.path().join("isolated-empty-models"),
        auto_rotate: false,
        deskew: false,
        ..Default::default()
    };
    let project = Project::open(
        root.path().join("code-project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1],
            models: vec![],
            settings: settings.clone(),
        },
    )
    .unwrap();
    let mut raster = image::RgbImage::from_pixel(1100, 250, image::Rgb([255, 255, 255]));
    for (x, y, text) in [
        (40, 35, "value = 1250;"),
        (40 + (4. * pitch).round() as i32, 115, "return value + 1;"),
    ] {
        imageproc::drawing::draw_text_mut(
            &mut raster,
            image::Rgb([0, 0, 0]),
            x,
            y,
            48.,
            &font,
            text,
        );
    }
    raster.save(project.image(1)).unwrap();
    project
        .save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks: vec![Block {
                    bbox: Some(BoxRect::full()),
                    ..Block::text("old recognition")
                }],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    let original = fs::read(project.image(1)).unwrap();
    let before = project.load(1).unwrap();
    let options = CodeOptions {
        indentation: Indentation::Monospace {
            left_margin_pixels: 40,
            space_width_pixels: pitch,
        },
        ..Default::default()
    };
    let c = Control::default();
    let preview =
        code_transcription::preview(&project, &before, 0, &options, &settings, &c).unwrap();
    assert_eq!(project.load(1).unwrap().content.plain(), "old recognition");
    let code = &preview.after.content.blocks[0];
    assert_eq!(code.kind, "code");
    assert!(code.uncertain);
    assert!(code.text.contains("1250"), "{}", code.text);
    assert!(code.text.contains('='), "{}", code.text);
    assert!(code.text.contains('+'), "{}", code.text);
    assert!(code.text.contains(';'), "{}", code.text);
    let indented = code
        .text
        .lines()
        .find(|line| line.contains("return"))
        .unwrap();
    assert!(indented.starts_with("    return"), "{indented:?}");
    assert_eq!(preview.after.candidates.len(), 2);
    assert!(preview.after.candidates[0].view.starts_with("code_raw:"));
    assert!(
        preview.after.candidates[0].content.blocks[0]
            .text
            .contains("1250")
    );
    assert!(code_transcription::apply_preview(&project, &before, &preview, "wrong", &c).is_err());
    code_transcription::apply_preview(&project, &before, &preview, &preview.token, &c).unwrap();
    assert_eq!(project.load(1).unwrap().content.blocks[0].text, code.text);
    review::undo(&project).unwrap();
    assert_eq!(project.load(1).unwrap().content.plain(), "old recognition");
    review::redo(&project).unwrap();
    assert_eq!(project.load(1).unwrap().candidates.len(), 2);
    assert_eq!(fs::read(project.image(1)).unwrap(), original);
    assert!(!settings.models_dir.exists());
    assert!(
        code_transcription::apply_preview(&project, &before, &preview, &preview.token, &c).is_err()
    );
    raster.put_pixel(0, 0, image::Rgb([0, 0, 0]));
    raster.save(project.image(1)).unwrap();
    assert!(
        code_transcription::apply_preview(&project, &before, &preview, &preview.token, &c).is_err()
    );
}
