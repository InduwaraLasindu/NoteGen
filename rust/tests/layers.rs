use notegen::{
    exports::{self, Format},
    layers::{self, LayerKind},
    review, roundtrip,
    storage::{Control, Manifest, Project, fingerprint},
    types::*,
    word,
};
use std::{fs, io::Read, path::Path};

fn fixture(root: &Path) -> Project {
    let project = Project::open(
        root.join("project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    project
        .save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            agreement: 1.0,
            reviewed: true,
            content: PageContent {
                blocks: vec![
                    Block {
                        text: "Original <text> & Tamil தமிழ்".into(),
                        bbox: Some(BoxRect::full()),
                        confidence: Some(1.0),
                        ..Block::text("")
                    },
                    Block::text("Margin note stays in every text export"),
                    Block {
                        kind: "table".into(),
                        rows: vec![vec!["A1 retained".into(), "=2+2 stays text".into()]],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    image::RgbImage::from_pixel(20, 30, image::Rgb([255, 255, 255]))
        .save(project.image(1))
        .unwrap();
    project
}
fn assign(project: &Project, block: usize, kind: LayerKind) -> String {
    let before = project.load(1).unwrap();
    layers::set(
        project,
        &before,
        block,
        kind,
        "Manual <tag> தமிழ்",
        "Human confirmation only",
        None,
    )
    .unwrap();
    project
        .load(1)
        .unwrap()
        .manual_layers
        .last()
        .unwrap()
        .id
        .clone()
}
fn zip_text(path: &Path, name: &str) -> String {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    zip.by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn human_labels_bind_full_blocks_and_are_reversible_without_changing_text() {
    let temp = tempfile::tempdir().unwrap();
    let p = fixture(temp.path());
    let initial = p.load(1).unwrap();
    let id = assign(&p, 1, LayerKind::MarginalNote);
    let assigned = p.load(1).unwrap();
    assert_eq!(
        fingerprint(&assigned.content).unwrap(),
        fingerprint(&initial.content).unwrap()
    );
    let report = layers::report(&assigned).unwrap();
    assert!(report[0].confirmed());
    assert_eq!(
        report[0].assignment.block_fingerprint,
        fingerprint(&initial.content.blocks[1]).unwrap()
    );
    assert!(report[0].caption().contains("marginal note"));
    review::undo(&p).unwrap();
    assert!(p.load(1).unwrap().manual_layers.is_empty());
    review::redo(&p).unwrap();
    assert_eq!(p.load(1).unwrap().manual_layers[0].id, id);
    let before = p.load(1).unwrap();
    layers::remove(&p, &before, &id).unwrap();
    assert!(p.load(1).unwrap().manual_layers.is_empty());
    assert_eq!(
        fingerprint(&p.load(1).unwrap().content).unwrap(),
        fingerprint(&initial.content).unwrap()
    );
    review::undo(&p).unwrap();
    assert_eq!(p.load(1).unwrap().manual_layers[0].id, id);
}

#[test]
fn reordering_text_and_evidence_edits_make_labels_stale_until_explicit_rebinding() {
    let temp = tempfile::tempdir().unwrap();
    let p = fixture(temp.path());
    let id = assign(&p, 0, LayerKind::Highlight);
    let mut changed = p.load(1).unwrap();
    changed.content.blocks.swap(0, 1);
    changed.reviewed = true;
    let report = layers::report(&changed).unwrap();
    assert_eq!(report[0].status, "stale");
    assert!(report[0].source_region.is_none());
    assert!(changed.needs_review(0.1));
    // Merely finding the old text elsewhere never moves the label automatically.
    assert_eq!(report[0].assignment.block, 0);
    let before = p.load(1).unwrap();
    review::commit(&p, &before, &changed, "Reorder source blocks").unwrap();
    layers::reconfirm(&p, &p.load(1).unwrap(), &id, 1).unwrap();
    let rebound = p.load(1).unwrap();
    assert!(layers::report(&rebound).unwrap()[0].confirmed());
    assert_eq!(rebound.manual_layers[0].block, 1);
    for change in 0..3 {
        let mut edited = rebound.clone();
        match change {
            0 => edited.content.blocks[1]
                .alternatives
                .push("New reader alternative".into()),
            1 => edited.content.blocks[1].text.push_str(" corrected"),
            _ => edited.content.blocks[1].confidence = Some(0.5),
        }
        assert!(!layers::report(&edited).unwrap()[0].confirmed());
    }
    let mut deleted = rebound;
    deleted.content.blocks.clear();
    assert_eq!(layers::report(&deleted).unwrap()[0].status, "stale");
}

#[test]
fn stale_page_snapshots_and_duplicate_or_tampered_metadata_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let p = fixture(temp.path());
    let outdated = p.load(1).unwrap();
    let id = assign(&p, 0, LayerKind::OriginalText);
    assert!(layers::set(&p, &outdated, 1, LayerKind::Stamp, "", "", None).is_err());
    let current = p.load(1).unwrap();
    assert!(layers::set(&p, &current, 0, LayerKind::OriginalText, "", "", None).is_err());
    assert!(
        layers::set(
            &p,
            &current,
            1,
            LayerKind::Other,
            &"x".repeat(129),
            "",
            None
        )
        .is_err()
    );
    assert!(layers::set(&p, &current, 1, LayerKind::Other, "", "bad\0note", None).is_err());
    assert!(layers::reconfirm(&p, &current, &id, 499).is_err());
    for corruption in 0..5 {
        let mut invalid = current.clone();
        match corruption {
            0 => invalid.manual_layers[0].block_fingerprint = "wrong".into(),
            1 => invalid.manual_layers[0].block = 500,
            2 => invalid.manual_layers.push(invalid.manual_layers[0].clone()),
            3 => invalid.manual_layers[0].id = "A".repeat(64),
            _ => invalid.manual_layers[0].note = "x".repeat(1025),
        }
        assert!(invalid.validate().is_err());
        assert!(p.save(&invalid).is_err());
    }
    let mut json = serde_json::to_value(&current.manual_layers[0]).unwrap();
    json["automatic_classifier"] = true.into();
    assert!(serde_json::from_value::<layers::Assignment>(json).is_err());
    let mut legacy = serde_json::to_value(current).unwrap();
    legacy.as_object_mut().unwrap().remove("manual_layers");
    let legacy: Page = serde_json::from_value(legacy).unwrap();
    assert!(legacy.manual_layers.is_empty());
}

#[test]
fn all_exports_retain_tagged_and_untagged_content_with_visible_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let p = fixture(temp.path());
    assign(&p, 1, LayerKind::TeacherCorrection);
    assign(&p, 2, LayerKind::Stamp);
    for (format, name) in [
        (Format::Markdown, "out.md"),
        (Format::Html, "out.html"),
        (Format::Latex, "out.tex"),
    ] {
        let output = temp.path().join(name);
        exports::export(&p, &output, format, &Control::default()).unwrap();
        let text = fs::read_to_string(output).unwrap();
        for value in [
            "Margin note stays in every text export",
            "A1 retained",
            "Tamil தமிழ்",
            "teacher correction",
            "stamp",
            "Human confirmation only",
        ] {
            assert!(text.contains(value), "Missing {value} in {name}");
        }
        assert!(text.contains("MANUAL LAYER"));
    }
    let trace = exports::traceability(&p, &Control::default()).unwrap();
    assert_eq!(trace.pages[0].blocks.len(), 3);
    assert_eq!(trace.pages[0].manual_layers.len(), 2);
    assert_eq!(
        trace.pages[0].blocks[1].manual_layers[0].assignment.kind,
        LayerKind::TeacherCorrection
    );
    let xlsx = temp.path().join("out.xlsx");
    exports::export(&p, &xlsx, Format::Spreadsheet, &Control::default()).unwrap();
    assert!(zip_text(&xlsx, "xl/worksheets/sheet2.xml").contains("=2+2 stays text"));
    let annotations = zip_text(&xlsx, "xl/worksheets/sheet3.xml");
    assert!(annotations.contains("teacher correction"));
    assert!(annotations.contains("Manual &lt;tag&gt; தமிழ்"));
    let docx = temp.path().join("out.docx");
    word::export(&p, &docx, &Control::default()).unwrap();
    let xml = zip_text(&docx, "word/document.xml");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let source_values: Vec<_> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "sdtContent")
        .map(|n| {
            n.descendants()
                .filter(|n| n.tag_name().name() == "t")
                .filter_map(|n| n.text())
                .collect::<String>()
        })
        .collect();
    assert_eq!(
        source_values,
        [
            "Original <text> & Tamil தமிழ்",
            "Margin note stays in every text export",
            "A1 retained",
            "=2+2 stays text"
        ]
    );
    assert!(xml.contains("teacher correction"));
    assert!(xml.contains("Manual &lt;tag&gt; தமிழ்"));
    let preview = roundtrip::preview(&p, &docx, &Control::default()).unwrap();
    assert!(preview.fields.iter().all(|f| f.status == "unchanged"));
    let before = p.load(1).unwrap();
    layers::remove(&p, &before, &before.manual_layers[0].id).unwrap();
    let preview = roundtrip::preview(&p, &docx, &Control::default()).unwrap();
    assert!(
        preview
            .fields
            .iter()
            .all(|f| f.status == "structural_conflict")
    );
}

#[test]
fn stale_exports_notice_does_not_attach_label_to_replacement_or_omit_any_text() {
    let temp = tempfile::tempdir().unwrap();
    let p = fixture(temp.path());
    assign(&p, 1, LayerKind::MarginalNote);
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[1].text = "Replacement content must survive".into();
    after.reviewed = true;
    review::commit(&p, &before, &after, "Explicit text correction").unwrap();
    let trace = exports::traceability(&p, &Control::default()).unwrap();
    assert!(trace.pages[0].requires_review);
    assert_eq!(trace.pages[0].manual_layers[0].status, "stale");
    assert!(trace.pages[0].blocks[1].manual_layers.is_empty());
    let output = temp.path().join("stale.html");
    exports::export(&p, &output, Format::Html, &Control::default()).unwrap();
    let text = fs::read_to_string(output).unwrap();
    assert!(text.contains("REVIEW NEEDED: the assigned block changed"));
    assert!(text.contains("Replacement content must survive"));
    assert!(text.contains("Original &lt;text&gt; &amp; Tamil தமிழ்"));
}
