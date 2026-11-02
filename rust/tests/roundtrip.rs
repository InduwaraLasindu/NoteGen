use notegen::{
    review, roundtrip,
    storage::{Control, Manifest, Project},
    types::*,
    word,
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};
struct Fixture {
    _temp: tempfile::TempDir,
    project: Project,
    document: PathBuf,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = Project::open(
        temp.path().join("project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "settings-source".into(),
            pages: vec![1, 2],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    for n in [1, 2] {
        project
            .save(&Page {
                page: n,
                image: format!("page-{n:05}.png"),
                content: PageContent {
                    blocks: vec![
                        Block::text(format!("Original paragraph {n}")),
                        Block {
                            kind: "table".into(),
                            rows: vec![
                                vec!["Item".into(), "Amount".into()],
                                vec!["A".into(), format!("{n}0.50")],
                            ],
                            header_rows: 1,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
    }
    let document = temp.path().join("original.docx");
    word::export(&project, &document, &Control::default()).unwrap();
    Fixture {
        _temp: temp,
        project,
        document,
    }
}
fn text_part(input: &Path, name: &str) -> String {
    let mut zip = ZipArchive::new(fs::File::open(input).unwrap()).unwrap();
    let mut s = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}
fn rewrite(input: &Path, output: &Path, mut change: impl FnMut(&str, &[u8]) -> Vec<u8>) {
    let mut zip = ZipArchive::new(fs::File::open(input).unwrap()).unwrap();
    let mut out = ZipWriter::new(fs::File::create(output).unwrap());
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let name = file.name().to_string();
        let mut bytes = vec![];
        file.read_to_end(&mut bytes).unwrap();
        out.start_file(&name, SimpleFileOptions::default()).unwrap();
        out.write_all(&change(&name, &bytes)).unwrap();
    }
    out.finish().unwrap();
}
fn replace_control(xml: &str, key: &str, text: &str) -> String {
    let parsed = roxmltree::Document::parse(xml).unwrap();
    let tag = parsed
        .descendants()
        .find(|n| {
            n.tag_name().name() == "tag"
                && n.attribute((
                    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
                    "val",
                )) == Some(key)
        })
        .unwrap();
    let sdt = tag.parent().unwrap().parent().unwrap();
    let content = sdt
        .children()
        .find(|n| n.tag_name().name() == "sdtContent")
        .unwrap();
    let mut result = xml.to_string();
    result.replace_range(
        content.range(),
        &format!(
            "<w:sdtContent>{}</w:sdtContent>",
            word::paragraph(text, "Normal")
        ),
    );
    result
}
fn edited(f: &Fixture, name: &str, changes: &[(&str, &str)]) -> PathBuf {
    let output = f.document.parent().unwrap().join(name);
    rewrite(&f.document, &output, |part, bytes| {
        if part != "word/document.xml" {
            return bytes.to_vec();
        }
        let mut xml = String::from_utf8(bytes.to_vec()).unwrap();
        for (key, text) in changes {
            xml = replace_control(&xml, key, text);
        }
        xml.into_bytes()
    });
    output
}

#[test]
fn unchanged_word_is_source_bound_and_has_no_importable_changes() {
    let f = fixture();
    let preview = roundtrip::preview(&f.project, &f.document, &Control::default()).unwrap();
    assert!(preview.warnings.is_empty());
    assert_eq!(preview.fields.len(), 10);
    assert!(preview.fields.iter().all(|f| f.status == "unchanged"));
    let source = text_part(&f.document, "customXml/item1.xml");
    assert!(source.contains("urn:notegen:roundtrip:1"));
    let rels = text_part(&f.document, "word/_rels/document.xml.rels");
    assert!(rels.contains("../customXml/item1.xml"));
}
#[test]
fn selected_text_and_cell_corrections_across_pages_form_one_undoable_revision() {
    let f = fixture();
    let p = &f.project;
    let original = fs::read(p.page_path(1)).unwrap();
    let input = edited(
        &f,
        "corrected.docx",
        &[
            ("ng:p1:b0:text", "Corrected paragraph"),
            ("ng:p2:b1:cell:1:1", "25.75"),
        ],
    );
    let preview = roundtrip::preview(p, &input, &Control::default()).unwrap();
    let selected: Vec<_> = preview
        .fields
        .iter()
        .filter(|f| f.status == "changed")
        .map(|f| f.tag.clone())
        .collect();
    assert_eq!(selected.len(), 2);
    roundtrip::apply(
        p,
        &input,
        &preview,
        &selected,
        "Word corrections",
        &Control::default(),
    )
    .unwrap();
    assert_eq!(
        p.load(1).unwrap().content.blocks[0].text,
        "Corrected paragraph"
    );
    assert_eq!(p.load(2).unwrap().content.blocks[1].rows[1][1], "25.75");
    assert_eq!(review::timeline(p).unwrap().revisions.len(), 1);
    assert_eq!(fs::read(p.page_path(1)).unwrap(), original);
    review::undo(p).unwrap();
    assert_eq!(
        p.load(1).unwrap().content.blocks[0].text,
        "Original paragraph 1"
    );
    assert_eq!(p.load(2).unwrap().content.blocks[1].rows[1][1], "20.50");
}
#[test]
fn importing_only_selected_fields_keeps_other_word_changes_unapplied() {
    let f = fixture();
    let input = edited(
        &f,
        "selected.docx",
        &[
            ("ng:p1:b0:text", "Selected"),
            ("ng:p2:b0:text", "Not selected"),
        ],
    );
    let preview = roundtrip::preview(&f.project, &input, &Control::default()).unwrap();
    roundtrip::apply(
        &f.project,
        &input,
        &preview,
        &["ng:p1:b0:text".into()],
        "Selected Word correction",
        &Control::default(),
    )
    .unwrap();
    assert_eq!(
        f.project.load(1).unwrap().content.blocks[0].text,
        "Selected"
    );
    assert_eq!(
        f.project.load(2).unwrap().content.blocks[0].text,
        "Original paragraph 2"
    );
}
#[test]
fn simultaneous_word_and_project_changes_are_explicit_conflicts() {
    let f = fixture();
    let p = &f.project;
    let input = edited(&f, "conflict.docx", &[("ng:p1:b0:text", "Word version")]);
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[0].text = "Project version".into();
    review::commit(p, &before, &after, "Local correction").unwrap();
    let preview = roundtrip::preview(p, &input, &Control::default()).unwrap();
    let field = preview
        .fields
        .iter()
        .find(|f| f.tag == "ng:p1:b0:text")
        .unwrap();
    assert_eq!(field.status, "conflict");
    assert_eq!(field.baseline, "Original paragraph 1");
    assert_eq!(field.current, "Project version");
    assert_eq!(field.imported.as_deref(), Some("Word version"));
    assert!(
        roundtrip::apply(
            p,
            &input,
            &preview,
            &[field.tag.clone()],
            "Conflict",
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(p.load(1).unwrap().content.blocks[0].text, "Project version");
}
#[test]
fn unrelated_current_corrections_survive_word_import() {
    let f = fixture();
    let p = &f.project;
    let input = edited(&f, "unrelated.docx", &[("ng:p1:b0:text", "Word version")]);
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[1].rows[1][1] = "99".into();
    review::commit(p, &before, &after, "Different cell").unwrap();
    let preview = roundtrip::preview(p, &input, &Control::default()).unwrap();
    assert_eq!(
        preview
            .fields
            .iter()
            .find(|f| f.tag == "ng:p1:b0:text")
            .unwrap()
            .status,
        "changed"
    );
    roundtrip::apply(
        p,
        &input,
        &preview,
        &["ng:p1:b0:text".into()],
        "Word text",
        &Control::default(),
    )
    .unwrap();
    assert_eq!(p.load(1).unwrap().content.blocks[1].rows[1][1], "99");
}
#[test]
fn stale_or_tampered_previews_and_duplicate_selections_are_rejected() {
    let f = fixture();
    let input = edited(&f, "stale.docx", &[("ng:p1:b0:text", "New text")]);
    let preview = roundtrip::preview(&f.project, &input, &Control::default()).unwrap();
    let mut tampered = preview.clone();
    tampered.fields[0].imported = Some("Injected".into());
    assert!(
        roundtrip::apply(
            &f.project,
            &input,
            &tampered,
            &["ng:p1:b0:text".into()],
            "Bad",
            &Control::default()
        )
        .is_err()
    );
    assert!(
        roundtrip::apply(
            &f.project,
            &input,
            &preview,
            &["ng:p1:b0:text".into(), "ng:p1:b0:text".into()],
            "Duplicate",
            &Control::default()
        )
        .is_err()
    );
    let before = f.project.load(2).unwrap();
    let mut after = before.clone();
    after.reviewed = true;
    review::commit(&f.project, &before, &after, "Other page approval").unwrap();
    assert!(
        roundtrip::apply(
            &f.project,
            &input,
            &preview,
            &["ng:p1:b0:text".into()],
            "Stale",
            &Control::default()
        )
        .is_err()
    );
}
#[test]
fn changing_the_word_file_after_preview_is_detected() {
    let f = fixture();
    let input = edited(&f, "changed.docx", &[("ng:p1:b0:text", "First Word edit")]);
    let preview = roundtrip::preview(&f.project, &input, &Control::default()).unwrap();
    let other = edited(&f, "other.docx", &[("ng:p1:b0:text", "Second Word edit")]);
    fs::copy(other, &input).unwrap();
    assert!(
        roundtrip::apply(
            &f.project,
            &input,
            &preview,
            &["ng:p1:b0:text".into()],
            "Stale file",
            &Control::default()
        )
        .is_err()
    );
}
#[test]
fn wrong_project_source_or_settings_cannot_accept_word_corrections() {
    let f = fixture();
    let mut other = f.project.clone();
    other.manifest.input_sha256 = "b".repeat(64);
    assert!(roundtrip::preview(&other, &f.document, &Control::default()).is_err());
    other = f.project.clone();
    other.manifest.fingerprint = "changed-settings".into();
    assert!(roundtrip::preview(&other, &f.document, &Control::default()).is_err());
}
#[test]
fn removed_controls_and_source_geometry_changes_require_manual_resolution() {
    let f = fixture();
    let input = f.document.parent().unwrap().join("missing-control.docx");
    rewrite(&f.document, &input, |name, bytes| {
        if name != "word/document.xml" {
            return bytes.to_vec();
        }
        let xml = String::from_utf8(bytes.to_vec()).unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let n = doc
            .descendants()
            .find(|n| n.tag_name().name() == "sdt")
            .unwrap();
        let mut out = xml.clone();
        out.replace_range(n.range(), "");
        out.into_bytes()
    });
    let preview = roundtrip::preview(&f.project, &input, &Control::default()).unwrap();
    assert!(!preview.warnings.is_empty());
    assert!(
        preview
            .fields
            .iter()
            .any(|f| f.imported.is_none() && f.status == "structural_conflict")
    );
    let before = f.project.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[0].bbox = Some(BoxRect::full());
    review::commit(&f.project, &before, &after, "Region repair").unwrap();
    let preview = roundtrip::preview(&f.project, &f.document, &Control::default()).unwrap();
    assert!(
        preview
            .fields
            .iter()
            .filter(|f| f.page == 1)
            .all(|f| f.status == "structural_conflict")
    );
}
#[test]
fn unknown_and_duplicate_control_tags_are_rejected() {
    let f = fixture();
    for (name, from, to) in [
        ("unknown.docx", "ng:p1:b0:text", "ng:unknown:control"),
        ("duplicate.docx", "ng:p2:b0:text", "ng:p1:b0:text"),
    ] {
        let output = f.document.parent().unwrap().join(name);
        rewrite(&f.document, &output, |part, bytes| {
            if part == "word/document.xml" {
                String::from_utf8(bytes.to_vec())
                    .unwrap()
                    .replace(from, to)
                    .into_bytes()
            } else {
                bytes.to_vec()
            }
        });
        assert!(roundtrip::preview(&f.project, &output, &Control::default()).is_err());
    }
}
#[test]
fn tracked_changes_fields_and_external_objects_are_not_imported_as_plain_text() {
    let f = fixture();
    let output = f.document.parent().unwrap().join("tracked.docx");
    rewrite(&f.document, &output, |part, bytes| {
        if part != "word/document.xml" {
            return bytes.to_vec();
        }
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace(
                "<w:t xml:space=\"preserve\">Original paragraph 1</w:t>",
                "<w:ins><w:t>Unaccepted insertion</w:t></w:ins>",
            )
            .into_bytes()
    });
    let preview = roundtrip::preview(&f.project, &output, &Control::default()).unwrap();
    assert_eq!(
        preview
            .fields
            .iter()
            .find(|f| f.tag == "ng:p1:b0:text")
            .unwrap()
            .status,
        "structural_conflict"
    );
}
#[test]
fn code_indentation_tabs_newlines_and_xml_filtering_roundtrip_without_false_edits() {
    let f = fixture();
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks[0].kind = "code".into();
    page.content.blocks[0].text = "fn main() {\n\t  let x = 1;\n}\n\0".into();
    p.save(&page).unwrap();
    let output = f.document.parent().unwrap().join("code.docx");
    word::export(p, &output, &Control::default()).unwrap();
    let preview = roundtrip::preview(p, &output, &Control::default()).unwrap();
    assert!(preview.fields.iter().all(|f| f.status == "unchanged"));
    let xml = text_part(&output, "word/document.xml");
    assert!(xml.contains("w:val=\"Code\""));
    assert!(xml.contains("\t  let x = 1;"));
    assert!(!xml.contains('\0'));
    let styles = text_part(&output, "word/styles.xml");
    assert!(styles.contains("Consolas"));
}
#[test]
fn semantic_styles_and_configured_fonts_spacing_are_written_to_word() {
    let f = fixture();
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks = [
        ("heading", "Title"),
        ("list", "• List item"),
        ("caption", "Caption"),
        ("equation", "x = 2"),
        ("reference", "[1] Reference"),
        ("code", "  return 42;"),
    ]
    .into_iter()
    .map(|(kind, text)| Block {
        kind: kind.into(),
        text: text.into(),
        ..Default::default()
    })
    .collect();
    p.save(&page).unwrap();
    let mut profile = word::StyleProfile::defaults(&p.manifest.settings);
    profile.styles.get_mut("Heading1").unwrap().font = "Georgia".into();
    profile.styles.get_mut("Heading1").unwrap().space_after = 300;
    word::save_style_profile(p, &profile).unwrap();
    let output = f.document.parent().unwrap().join("styles.docx");
    word::export(p, &output, &Control::default()).unwrap();
    let xml = text_part(&output, "word/document.xml");
    for style in [
        "Heading1",
        "ListParagraph",
        "Caption",
        "Equation",
        "Reference",
        "Code",
    ] {
        assert!(xml.contains(&format!("w:val=\"{style}\"")));
    }
    let styles = text_part(&output, "word/styles.xml");
    assert!(styles.contains("Georgia"));
    assert!(styles.contains("w:after=\"300\""));
}
#[test]
fn invalid_word_style_profiles_are_rejected_without_overwriting_good_styles() {
    let f = fixture();
    let p = &f.project;
    let profile = word::StyleProfile::defaults(&p.manifest.settings);
    word::save_style_profile(p, &profile).unwrap();
    let original = fs::read(p.root.join("word-styles.json")).unwrap();
    let mut bad = profile.clone();
    bad.styles.get_mut("Normal").unwrap().size = f32::NAN;
    assert!(word::save_style_profile(p, &bad).is_err());
    bad = profile.clone();
    bad.styles
        .insert("UnknownStyle".into(), profile.styles["Normal"].clone());
    assert!(word::save_style_profile(p, &bad).is_err());
    assert_eq!(fs::read(p.root.join("word-styles.json")).unwrap(), original);
}
#[test]
fn right_to_left_paragraphs_and_tables_preserve_number_text() {
    let f = fixture();
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks[0].text = "المجموع 123.45 سنة 2026".into();
    page.content.blocks[1].rows[0][0] = "שם".into();
    p.save(&page).unwrap();
    let output = f.document.parent().unwrap().join("rtl.docx");
    word::export(p, &output, &Control::default()).unwrap();
    let xml = text_part(&output, "word/document.xml");
    assert!(xml.contains("<w:bidi/>"));
    assert!(xml.contains("<w:bidiVisual/>"));
    assert!(xml.contains("123.45"));
    let preview = roundtrip::preview(p, &output, &Control::default()).unwrap();
    assert!(preview.fields.iter().all(|f| f.status == "unchanged"));
}
#[test]
fn source_figure_observation_and_interpretation_can_be_corrected_separately() {
    let f = fixture();
    let p = &f.project;
    image::RgbImage::from_pixel(10, 10, image::Rgb([255, 255, 255]))
        .save(p.image(1))
        .unwrap();
    let mut page = p.load(1).unwrap();
    page.content.blocks.push(Block {
        kind: "figure".into(),
        bbox: Some(BoxRect::full()),
        description: "Visible circle".into(),
        interpretation: "Possible atom".into(),
        ..Default::default()
    });
    p.save(&page).unwrap();
    let exported = f.document.parent().unwrap().join("figure.docx");
    word::export(p, &exported, &Control::default()).unwrap();
    let edited = f.document.parent().unwrap().join("figure-edited.docx");
    rewrite(&exported, &edited, |part, bytes| {
        if part == "word/document.xml" {
            replace_control(
                &String::from_utf8(bytes.to_vec()).unwrap(),
                "ng:p1:b2:interpretation",
                "Confirmed cell diagram",
            )
            .into_bytes()
        } else {
            bytes.to_vec()
        }
    });
    let preview = roundtrip::preview(p, &edited, &Control::default()).unwrap();
    roundtrip::apply(
        p,
        &edited,
        &preview,
        &["ng:p1:b2:interpretation".into()],
        "Figure explanation",
        &Control::default(),
    )
    .unwrap();
    let page = p.load(1).unwrap();
    assert_eq!(page.content.blocks[2].description, "Visible circle");
    assert_eq!(
        page.content.blocks[2].interpretation,
        "Confirmed cell diagram"
    );
}
#[test]
fn merged_cell_source_fields_remain_individually_traceable() {
    let f = fixture();
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks[1].merges = vec![Merge {
        row: 0,
        col: 0,
        rowspan: 1,
        colspan: 2,
    }];
    p.save(&page).unwrap();
    let output = f.document.parent().unwrap().join("merged.docx");
    word::export(p, &output, &Control::default()).unwrap();
    let preview = roundtrip::preview(p, &output, &Control::default()).unwrap();
    assert!(preview.fields.iter().all(|f| f.status == "unchanged"));
    assert!(preview.fields.iter().any(|f| f.tag == "ng:p1:b1:cell:0:1"));
    assert!(text_part(&output, "word/document.xml").contains("gridSpan"));
}
#[test]
fn cancelled_import_does_not_create_a_revision() {
    let f = fixture();
    let input = edited(
        &f,
        "cancelled.docx",
        &[("ng:p1:b0:text", "Should not apply")],
    );
    let control = Control::default();
    control
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(roundtrip::preview(&f.project, &input, &control).is_err());
    assert!(review::timeline(&f.project).unwrap().revisions.is_empty());
}
#[test]
fn manual_recovery_word_export_conspicuously_labels_unverified_source_without_blocking_edits() {
    let f = fixture();
    fs::write(
        f.project.root.join("input-invalidated.json"),
        br#"{"reason":"Input changed during conversion"}"#,
    )
    .unwrap();
    let output = f.document.parent().unwrap().join("recovery.docx");
    let original = fs::read(f.project.page_path(1)).unwrap();
    word::export(&f.project, &output, &Control::default()).unwrap();
    let xml = text_part(&output, "word/document.xml");
    assert!(xml.contains(word::INVALIDATED_SOURCE_WARNING));
    assert!(xml.contains("UNVERIFIED SOURCE"));
    assert!(xml.contains("manual recovery export"));
    assert!(xml.contains("w:color w:val=\"C62828\""));
    let preview = roundtrip::preview(&f.project, &output, &Control::default()).unwrap();
    assert!(preview.fields.iter().all(|f| f.status == "unchanged"));
    assert_eq!(fs::read(f.project.page_path(1)).unwrap(), original);
}
