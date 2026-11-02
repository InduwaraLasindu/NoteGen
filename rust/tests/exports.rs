use notegen::{
    exports::{self, Format},
    storage::{Control, Manifest, Project},
    types::*,
};
use std::{fs, io::Read, path::Path, sync::atomic::Ordering};

fn fixture(root: &Path) -> Project {
    let project = Project::open(
        root.join("source space {notes}"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1],
            models: vec!["reader".into()],
            settings: Settings::default(),
        },
    )
    .unwrap();
    let table = Block {
        kind: "table".into(),
        rows: vec![
            vec![
                "=HYPERLINK(\"https://invalid\")".into(),
                "සිංහල தமிழ் العربية".into(),
            ],
            vec!["001.20".into(), "<&>\"' |\nnext".into()],
        ],
        bbox: Some(BoxRect {
            left: 0.1,
            top: 0.2,
            right: 0.9,
            bottom: 0.6,
        }),
        uncertain: true,
        confidence: Some(0.3),
        header_rows: 1,
        ..Default::default()
    };
    let content = PageContent {
        blocks: vec![
            Block {
                kind: "heading".into(),
                text: "Notes <script>alert(1)</script> & #1".into(),
                confidence: Some(1.0),
                ..Default::default()
            },
            table,
            Block {
                kind: "equation".into(),
                text: "\\input{secret} $x_1^2$ 50% & # ~".into(),
                confidence: Some(0.9),
                ..Default::default()
            },
        ],
        warnings: vec!["Reader <warning> & review".into()],
        ..Default::default()
    };
    let page = Page {
        edited: true,
        page: 1,
        image: "page-00001.png".into(),
        candidates: vec![Candidate {
            model_id: "reader".into(),
            family: "independent-family".into(),
            view: "whole_page".into(),
            content: content.clone(),
            reliability: 0.8,
            region: None,
        }],
        content,
        reviewed: false,
        agreement: 0.8,
        source_warnings: vec!["Blur detected".into()],
        ..Default::default()
    };
    project.save(&page).unwrap();
    image::RgbImage::new(10, 10).save(project.image(1)).unwrap();
    project
}
fn entry(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> String {
    let mut s = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn markdown_keeps_content_sources_and_review_without_executable_markup() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let target = dir.path().join("notes.md");
    exports::export(&p, &target, Format::Markdown, &Control::default()).unwrap();
    let text = fs::read_to_string(target).unwrap();
    assert!(text.contains("REVIEW NEEDED"));
    assert!(text.contains("\\<script\\>"));
    assert!(!text.contains("<script>"));
    assert!(text.contains("සිංහල தமிழ் العربية"));
    assert!(text.contains("001\\.20"));
    assert!(text.contains("\\|<br>next"));
    assert!(text.contains("file:///"));
    assert!(text.contains("source%20space%20%7Bnotes%7D"));
    assert!(text.contains("#xywh=percent,10.0000,20.0000,80.0000,40.0000"));
    assert!(text.contains("page-level location"));
}
#[test]
fn html_is_semantic_unicode_and_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let target = dir.path().join("notes.html");
    exports::export(&p, &target, Format::Html, &Control::default()).unwrap();
    let text = fs::read_to_string(target).unwrap();
    let html = scraper::Html::parse_document(&text);
    let selector = |s| scraper::Selector::parse(s).unwrap();
    assert_eq!(html.select(&selector("script")).count(), 0);
    assert_eq!(html.select(&selector("article")).count(), 3);
    assert_eq!(html.select(&selector("table tr")).count(), 2);
    assert_eq!(html.select(&selector("th")).count(), 2);
    assert_eq!(html.select(&selector("td")).count(), 2);
    assert!(text.contains("&lt;script&gt;"));
    assert!(text.contains("Reader &lt;warning&gt; &amp; review"));
    assert!(text.contains("dir=\"auto\""));
    assert!(text.contains("සිංහල தமிழ் العربية"));
    assert!(text.ends_with("</body></html>\n"));
}
#[test]
fn latex_escapes_commands_math_and_source_percent_signs() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let target = dir.path().join("notes.tex");
    exports::export(&p, &target, Format::Latex, &Control::default()).unwrap();
    let text = fs::read_to_string(target).unwrap();
    assert!(!text.contains("\\input{secret}"));
    assert!(text.contains("\\textbackslash{}input\\{secret\\}"));
    assert!(text.contains("\\$x\\_1\\textasciicircum{}2\\$ 50\\% \\& \\# \\textasciitilde{}"));
    assert!(text.contains("\\%20"));
    assert!(text.contains("\\#xywh=percent"));
    assert!(text.contains("\\begin{longtable}{ll}"));
    assert!(text.contains("සිංහල தமிழ் العربية"));
    assert!(text.ends_with("\\end{document}\n"));
}
#[test]
fn spreadsheet_is_real_ooxml_with_text_values_sources_and_uncertainty() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let target = dir.path().join("notes.xlsx");
    exports::export(&p, &target, Format::Spreadsheet, &Control::default()).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(target).unwrap()).unwrap();
    let names = zip.file_names().map(str::to_owned).collect::<Vec<_>>();
    for name in names
        .iter()
        .filter(|v| v.ends_with(".xml") || v.ends_with(".rels"))
    {
        let s = entry(&mut zip, name);
        roxmltree::Document::parse(&s).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let sheet = entry(&mut zip, "xl/worksheets/sheet2.xml");
    let doc = roxmltree::Document::parse(&sheet).unwrap();
    let first = doc
        .descendants()
        .find(|n| {
            n.has_tag_name((
                "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                "c",
            )) && n.attribute("r") == Some("A1")
        })
        .unwrap();
    assert_eq!(first.attribute("t"), Some("inlineStr"));
    assert_eq!(
        first
            .descendants()
            .find(|n| n.tag_name().name() == "t")
            .unwrap()
            .text(),
        Some("=HYPERLINK(\"https://invalid\")")
    );
    assert_eq!(first.attribute("s"), Some("2"));
    assert!(!doc.descendants().any(|n| n.tag_name().name() == "f"));
    assert!(sheet.contains("001.20"));
    assert!(sheet.contains("සිංහල தமிழ் العربية"));
    let review = entry(&mut zip, "xl/worksheets/sheet1.xml");
    assert!(review.contains("Page reviewed"));
    assert!(review.contains("Blur detected"));
    assert!(review.contains("false"));
    assert!(review.contains("not an individual cell"));
    let rels = entry(&mut zip, "xl/worksheets/_rels/sheet2.xml.rels");
    assert!(rels.contains("TargetMode=\"External\""));
    assert!(rels.contains("#xywh=percent,10.0000"));
    assert!(!rels.contains("https://invalid"));
    let trace: exports::Traceability =
        serde_json::from_str(&entry(&mut zip, "notegen/traceability.json")).unwrap();
    assert_eq!(trace.pages[0].blocks[1].cells.len(), 4);
}
#[test]
fn traceability_identifies_page_fallback_table_scope_and_exact_reader_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let trace = exports::traceability(&p, &Control::default()).unwrap();
    assert_eq!(trace.schema, 1);
    assert_eq!(trace.input_sha256, "a".repeat(64));
    assert_eq!(trace.pages[0].blocks[0].localization, "page_fallback");
    assert_eq!(trace.pages[0].blocks[1].source_region.left, 0.1);
    let cell = &trace.pages[0].blocks[1].cells[2];
    assert_eq!((cell.row, cell.column), (2, 1));
    assert_eq!(cell.localization, "table_region");
    assert_eq!(
        cell.evidence[0].json_pointer,
        "/candidates/0/content/blocks/1/rows/1/0"
    );
    assert!(
        cell.evidence[0]
            .checkpoint_uri
            .ends_with("page-00001.edits.json")
    );
    assert!(trace.evidence_note.contains("not proof of correctness"));
    let mut page = p.load(1).unwrap();
    page.content.blocks[0].text = "A human corrected the text".into();
    p.save(&page).unwrap();
    let trace = exports::traceability(&p, &Control::default()).unwrap();
    assert!(trace.pages[0].blocks[0].evidence.is_empty());
    assert!(!trace.pages[0].blocks[0].source_uri.is_empty());
    let target = dir.path().join("trace.json");
    exports::export(&p, &target, Format::Traceability, &Control::default()).unwrap();
    let reread: exports::Traceability = serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
    assert_eq!(reread.pages[0].blocks[0].text, "A human corrected the text");
}
#[test]
fn export_protects_existing_files_rejects_wrong_extensions_and_respects_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    for format in [
        Format::Markdown,
        Format::Html,
        Format::Latex,
        Format::Spreadsheet,
        Format::Traceability,
    ] {
        let target = dir.path().join(format!("notes.{}", format.extension()));
        fs::write(&target, b"keep me").unwrap();
        assert!(exports::export(&p, &target, format, &Control::default()).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep me");
        assert!(
            exports::export(&p, &dir.path().join("bad.exe"), format, &Control::default()).is_err()
        );
        let cancelled = Control::default();
        cancelled.cancelled.store(true, Ordering::Relaxed);
        let target = dir.path().join(format!("cancelled.{}", format.extension()));
        assert!(exports::export(&p, &target, format, &cancelled).is_err());
        assert!(!target.exists());
    }
}
#[test]
fn empty_projects_export_valid_documents_and_empty_spreadsheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = fixture(dir.path());
    p.manifest.pages.clear();
    for format in [
        Format::Markdown,
        Format::Html,
        Format::Latex,
        Format::Spreadsheet,
        Format::Traceability,
    ] {
        let target = dir.path().join(format!("empty.{}", format.extension()));
        exports::export(&p, &target, format, &Control::default()).unwrap();
        assert!(fs::metadata(&target).unwrap().len() > 0);
        if format == Format::Spreadsheet {
            let mut zip = zip::ZipArchive::new(fs::File::open(target).unwrap()).unwrap();
            assert!(entry(&mut zip, "xl/worksheets/sheet1.xml").contains("No recognized tables"));
            assert!(!entry(&mut zip, "xl/workbook.xml").contains("Table 1"));
        }
    }
}
#[test]
fn excel_addresses_include_a1_aa_and_final_column() {
    for (n, expected) in [
        (1, "A"),
        (26, "Z"),
        (27, "AA"),
        (40, "AN"),
        (702, "ZZ"),
        (703, "AAA"),
        (16384, "XFD"),
    ] {
        assert_eq!(exports::column_name(n).unwrap(), expected);
    }
    assert!(exports::column_name(0).is_err());
    assert!(exports::column_name(16385).is_err());
}
#[test]
fn spreadsheet_preserves_merged_cell_values_and_rejects_oversized_excel_text_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let mut page = p.load(1).unwrap();
    page.content.blocks[1].merges.push(Merge {
        row: 0,
        col: 0,
        rowspan: 1,
        colspan: 2,
    });
    p.save(&page).unwrap();
    let target = dir.path().join("merged.xlsx");
    exports::export(&p, &target, Format::Spreadsheet, &Control::default()).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(target).unwrap()).unwrap();
    assert!(entry(&mut zip, "xl/worksheets/sheet2.xml").contains("<mergeCell ref=\"A1:B1\"/>"));
    assert!(entry(&mut zip, "xl/worksheets/sheet1.xml").contains("සිංහල தமிழ் العربية"));
    page.content.blocks[1].rows[1][0] = "😀".repeat(16384);
    p.save(&page).unwrap();
    let target = dir.path().join("oversized.xlsx");
    assert!(exports::export(&p, &target, Format::Spreadsheet, &Control::default()).is_err());
    assert!(!target.exists());
}
