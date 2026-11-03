use notegen::{
    exports::{self, Format},
    redactions::{self, MaskSpec, Reason},
    references::{self, Bibliography, Kind, Request},
    review, roundtrip,
    storage::{Control, Manifest, Project},
    types::*,
    word,
};
use std::{fs, io::Read, path::Path};
fn fixture(root: &Path) -> Project {
    let p = Project::open(
        root.join("private-project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1, 2, 3],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    for n in [1, 2, 3] {
        image::RgbImage::from_pixel(40, 40, image::Rgb([180, n as u8 * 30, 90]))
            .save(p.image(n))
            .unwrap();
        let body = match n {
            1 => "Claim λ [1] and source (Smith).",
            2 => "PRIVATE_NOTE <literal> & $5 \\input{evil}",
            _ => "Smith. 2024. Original bibliography <title> & page 1250.",
        };
        let mut block = Block {
            bbox: Some(BoxRect::full()),
            ..Block::text(body)
        };
        if n == 3 {
            block.kind = "reference".into();
        }
        if n == 2 {
            block.uncertain = true;
        }
        let mut blocks = vec![block];
        if n == 1 {
            blocks.push(Block::text("Ordinary editable paragraph."));
        }
        p.save(&Page {
            page: n,
            image: format!("page-{n:05}.png"),
            content: PageContent {
                blocks,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    }
    p
}
fn request(page: &Page, kind: Kind) -> Request {
    let (marker, target) = match kind {
        Kind::Footnote => ("[1]", 2),
        Kind::Citation => ("(Smith)", 3),
    };
    let start = page.content.blocks[0].text.find(marker).unwrap();
    Request {
        kind,
        anchor_block: 0,
        marker_start: start,
        marker_end: start + marker.len(),
        target_page: target,
        target_block: 0,
        bibliography: if kind == Kind::Citation {
            Some(Bibliography {
                title: "PRIVATE_BIB_TITLE <x> & safe".into(),
                author: "Smith & A".into(),
                year: "2024".into(),
            })
        } else {
            None
        },
    }
}
fn confirm(p: &Project, kind: Kind) -> String {
    let page = p.load(1).unwrap();
    let preview =
        references::preview(p, &page, &request(&page, kind), &Control::default()).unwrap();
    references::confirm(p, &page, &preview, &preview.token, &Control::default()).unwrap();
    preview.link.id
}
fn part(path: &Path, name: &str) -> String {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    zip.by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}
fn has_part(path: &Path, name: &str) -> bool {
    let zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    zip.file_names().any(|n| n == name)
}

#[test]
fn only_confirmed_links_become_real_word_notes_citations_bibliography_and_text_format_links() {
    let root = tempfile::tempdir().unwrap();
    let p = fixture(root.path());
    let original = fs::read(p.image(1)).unwrap();
    let plain = root.path().join("plain.docx");
    word::export(&p, &plain, &Control::default()).unwrap();
    assert!(!has_part(&plain, "word/footnotes.xml"));
    assert!(!has_part(&plain, "customXml/item2.xml"));
    let before = p.load(1).unwrap();
    let preview = references::preview(
        &p,
        &before,
        &request(&before, Kind::Footnote),
        &Control::default(),
    )
    .unwrap();
    assert!(references::confirm(&p, &before, &preview, "wrong", &Control::default()).is_err());
    assert!(p.load(1).unwrap().confirmed_references.is_empty());
    references::confirm(&p, &before, &preview, &preview.token, &Control::default()).unwrap();
    assert_eq!(
        p.load(1).unwrap().content.blocks[0].text,
        before.content.blocks[0].text
    );
    review::undo(&p).unwrap();
    assert!(p.load(1).unwrap().confirmed_references.is_empty());
    review::redo(&p).unwrap();
    assert_eq!(p.load(1).unwrap().confirmed_references.len(), 1);
    confirm(&p, Kind::Citation);
    let resolved = references::resolve(&p, &Control::default()).unwrap();
    assert_eq!(resolved.links.len(), 2);
    assert!(resolved.warnings.is_empty());
    assert_eq!(
        exports::traceability(&p, &Control::default())
            .unwrap()
            .confirmed_references
            .links
            .len(),
        2
    );
    let doc = root.path().join("semantic.docx");
    word::export(&p, &doc, &Control::default()).unwrap();
    let document = part(&doc, "word/document.xml");
    let notes = part(&doc, "word/footnotes.xml");
    let sources = part(&doc, "customXml/item2.xml");
    assert!(document.contains("w:footnoteReference w:id=\"1\" w:customMarkFollows=\"1\""));
    assert!(document.contains("[1]"));
    assert!(document.contains("CITATION NG"));
    assert!(document.contains("\\l 1033"));
    assert!(document.contains("BIBLIOGRAPHY"));
    assert!(!document.contains("PRIVATE_NOTE"));
    let parsed = roxmltree::Document::parse(&notes).unwrap();
    let body = parsed
        .descendants()
        .filter_map(|n| n.text())
        .collect::<String>();
    assert!(body.contains("PRIVATE_NOTE <literal> & $5 \\input{evil}"));
    assert!(body.contains("Uncertain source reading"));
    assert!(sources.contains("PRIVATE_BIB_TITLE &lt;x&gt; &amp; safe"));
    assert!(sources.contains("<b:Corporate>Smith &amp; A</b:Corporate>"));
    let rels = part(&doc, "word/_rels/document.xml.rels");
    assert!(rels.contains("Target=\"footnotes.xml\""));
    assert!(rels.contains("Target=\"../customXml/item2.xml\""));
    assert!(part(&doc, "[Content_Types].xml").contains("wordprocessingml.footnotes+xml"));
    let mut zip = zip::ZipArchive::new(fs::File::open(&doc).unwrap()).unwrap();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        if entry.name().ends_with(".xml") || entry.name().ends_with(".rels") {
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            roxmltree::Document::parse(&text).unwrap();
        }
    }
    let imported = roundtrip::preview(&p, &doc, &Control::default()).unwrap();
    assert!(!imported.fields.is_empty());
    assert!(imported.fields.iter().all(|f| f.status == "unchanged"));
    assert!(
        imported
            .fields
            .iter()
            .any(|f| f.imported.as_deref() == Some("Ordinary editable paragraph."))
    );
    for format in [Format::Markdown, Format::Html, Format::Latex] {
        let out = root.path().join(format!("semantic.{}", format.extension()));
        exports::export(&p, &out, format, &Control::default()).unwrap();
        let text = fs::read_to_string(out).unwrap();
        match format {
            Format::Markdown => {
                assert!(text.contains("[^ng-"));
                assert!(text.contains("](#bib-"));
                assert!(text.contains("## Confirmed bibliography"));
            }
            Format::Html => {
                assert!(text.contains("role=\"doc-noteref\""));
                assert!(text.contains("role=\"doc-backlink\""));
                assert!(text.contains("role=\"doc-biblioref\""));
                assert!(!text.contains("<literal>"));
            }
            Format::Latex => {
                assert!(text.contains("\\footnote{"));
                assert!(text.contains("\\begin{thebibliography}"));
                assert!(text.contains("\\bibitem{NG"));
                assert!(text.contains("\\nocite{NG"));
                assert!(!text.contains("\\input{evil}"));
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(fs::read(p.image(1)).unwrap(), original);
}

#[test]
fn stale_markers_targets_sources_and_overlapping_selection_never_retarget_or_remove_plain_text() {
    let root = tempfile::tempdir().unwrap();
    let p = fixture(root.path());
    let id = confirm(&p, Kind::Footnote);
    let bound = references::resolve(&p, &Control::default()).unwrap();
    let page = p.load(1).unwrap();
    assert!(
        references::preview(
            &p,
            &page,
            &request(&page, Kind::Footnote),
            &Control::default()
        )
        .is_err()
    );
    let citation = references::preview(
        &p,
        &page,
        &request(&page, Kind::Citation),
        &Control::default(),
    )
    .unwrap();
    let target = p.load(3).unwrap();
    let mut changed = target.clone();
    changed.content.blocks[0].text.push_str(" corrected");
    review::commit(&p, &target, &changed, "Correct bibliography").unwrap();
    assert!(
        references::confirm(&p, &page, &citation, &citation.token, &Control::default()).is_err()
    );
    assert_eq!(p.load(1).unwrap().confirmed_references.len(), 1);
    let target = p.load(2).unwrap();
    let mut changed = target.clone();
    changed.content.blocks[0].text = "New note body".into();
    review::commit(&p, &target, &changed, "Correct note body").unwrap();
    assert!(bound.verify_unchanged(&p, &Control::default()).is_err());
    let resolved = references::resolve(&p, &Control::default()).unwrap();
    assert!(resolved.links.is_empty());
    assert!(!resolved.warnings.is_empty());
    let doc = root.path().join("stale.docx");
    word::export(&p, &doc, &Control::default()).unwrap();
    assert!(!has_part(&doc, "word/footnotes.xml"));
    let text = part(&doc, "word/document.xml");
    assert!(text.contains("[1]"));
    assert!(text.contains("New note body"));
    assert!(text.contains("stale, covered or ambiguous"));
    let page = p.load(1).unwrap();
    references::remove(&p, &page, &id, &Control::default()).unwrap();
    assert!(p.load(1).unwrap().confirmed_references.is_empty());
    review::undo(&p).unwrap();
    assert_eq!(p.load(1).unwrap().confirmed_references.len(), 1);
    review::undo(&p).unwrap(); // Restore the original bound note body.
    assert_eq!(
        references::resolve(&p, &Control::default())
            .unwrap()
            .links
            .len(),
        1
    );
    image::RgbImage::from_pixel(40, 40, image::Rgb([0, 0, 0]))
        .save(p.image(2))
        .unwrap();
    assert!(
        references::resolve(&p, &Control::default())
            .unwrap()
            .links
            .is_empty()
    );
    let page = p.load(1).unwrap();
    let mut invalid = request(&page, Kind::Citation);
    invalid.marker_start = 7;
    invalid.marker_end = 8;
    assert!(references::preview(&p, &page, &invalid, &Control::default()).is_err());
}

#[test]
fn ambiguous_imported_relationships_remain_plain_and_invalid_bibliography_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let p = fixture(root.path());
    let before = p.load(1).unwrap();
    let footnote = references::preview(
        &p,
        &before,
        &request(&before, Kind::Footnote),
        &Control::default(),
    )
    .unwrap();
    let mut choice = request(&before, Kind::Citation);
    choice.marker_start = footnote.link.marker_start;
    choice.marker_end = footnote.link.marker_end;
    let citation = references::preview(&p, &before, &choice, &Control::default()).unwrap();
    let mut invalid = choice.clone();
    invalid.bibliography.as_mut().unwrap().title.clear();
    assert!(references::preview(&p, &before, &invalid, &Control::default()).is_err());
    invalid = choice;
    invalid.bibliography.as_mut().unwrap().author = "Smith\nInjected".into();
    assert!(references::preview(&p, &before, &invalid, &Control::default()).is_err());
    let mut imported = before;
    imported.confirmed_references = vec![footnote.link, citation.link];
    p.save(&imported).unwrap();
    let resolved = references::resolve(&p, &Control::default()).unwrap();
    assert!(resolved.links.is_empty());
    assert!(resolved.warnings.iter().any(|w| w.contains("Conflicting")));
    let doc = root.path().join("ambiguous.docx");
    word::export(&p, &doc, &Control::default()).unwrap();
    assert!(!has_part(&doc, "word/footnotes.xml"));
    assert!(!has_part(&doc, "customXml/item2.xml"));
    let text = part(&doc, "word/document.xml");
    assert!(text.contains("[1]"));
    assert!(text.contains("PRIVATE_NOTE"));
    assert!(text.contains("Original bibliography"));
}

#[test]
fn masked_reference_targets_cannot_leak_private_note_or_bibliography_metadata_into_exports() {
    let root = tempfile::tempdir().unwrap();
    let p = fixture(root.path());
    confirm(&p, Kind::Footnote);
    confirm(&p, Kind::Citation);
    for n in [2, 3] {
        let before = p.load(n).unwrap();
        let preview = redactions::preview_confirmation(
            &p,
            &before,
            &[MaskSpec {
                region: BoxRect::full(),
                reason: Reason::PersonalInformation,
            }],
            &Control::default(),
        )
        .unwrap();
        let mut after = before.clone();
        after.confirmed_redactions = redactions::confirm(
            &p,
            &before,
            &preview,
            &preview.token,
            &[0],
            &Control::default(),
        )
        .unwrap();
        review::commit(&p, &before, &after, "Confirm private reference cover").unwrap();
    }
    assert!(
        references::resolve(&p, &Control::default())
            .unwrap()
            .links
            .is_empty()
    );
    let private = fs::read(p.root.join("review-history.json")).unwrap();
    let anchor = fs::read(p.root.join("page-00001.edits.json")).unwrap();
    let doc = root.path().join("redacted.docx");
    word::export(&p, &doc, &Control::default()).unwrap();
    assert!(!has_part(&doc, "word/footnotes.xml"));
    assert!(!has_part(&doc, "customXml/item2.xml"));
    let mut zip = zip::ZipArchive::new(fs::File::open(doc).unwrap()).unwrap();
    for i in 0..zip.len() {
        let mut bytes = vec![];
        zip.by_index(i).unwrap().read_to_end(&mut bytes).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("PRIVATE_NOTE"));
        assert!(!text.contains("PRIVATE_BIB_TITLE"));
    }
    for format in [Format::Markdown, Format::Html, Format::Latex] {
        let out = root.path().join(format!("redacted.{}", format.extension()));
        exports::export(&p, &out, format, &Control::default()).unwrap();
        let text = fs::read_to_string(out).unwrap();
        assert!(!text.contains("PRIVATE_NOTE"));
        assert!(!text.contains("PRIVATE_BIB_TITLE"));
        assert!(!text.contains("doc-noteref"));
    }
    assert_eq!(
        fs::read(p.root.join("review-history.json")).unwrap(),
        private
    );
    assert_eq!(
        fs::read(p.root.join("page-00001.edits.json")).unwrap(),
        anchor
    );
}
