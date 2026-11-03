use notegen::{
    equations, review, roundtrip,
    storage::{Control, Manifest, Project},
    types::*,
    word,
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
fn round(source: &str) -> equations::Rendered {
    let rendered = equations::convert(source).unwrap();
    let doc = roxmltree::Document::parse(&rendered.omml).unwrap();
    assert_eq!(doc.root_element().tag_name().namespace(), Some(M));
    assert_eq!(
        equations::from_omml(doc.root_element()).unwrap(),
        rendered.canonical
    );
    assert_eq!(
        equations::canonical(&rendered.canonical).unwrap(),
        rendered.canonical
    );
    rendered
}
#[test]
fn fractions_are_editable_omml_with_separate_numerator_and_denominator() {
    let rendered = round(r"\frac{x+1}{y-2}");
    let doc = roxmltree::Document::parse(&rendered.omml).unwrap();
    for name in ["f", "num", "den"] {
        assert!(
            doc.descendants()
                .any(|n| n.tag_name().namespace() == Some(M) && n.tag_name().name() == name)
        );
    }
    assert_eq!(rendered.canonical, r"\frac{x+1}{y-2}");
}
#[test]
fn scripts_have_structured_bases_subscripts_and_superscripts() {
    for source in [
        r"x_2",
        r"x^2",
        r"x_i^2",
        r"x^2_i",
        r"{a+b}^{n+1}",
        r"\frac{1}{2}^{2}",
    ] {
        round(source);
    }
    let rendered = round(r"x_i^2");
    assert!(rendered.omml.contains("<m:sSubSup>"));
    assert_eq!(rendered.canonical, r"x_{i}^{2}");
}
#[test]
fn square_and_indexed_roots_keep_degree_and_radicand_editable() {
    let square = round(r"\sqrt{x^2+y^2}");
    assert!(square.omml.contains("<m:rad>"));
    assert!(square.omml.contains("m:degHide m:val=\"1\""));
    let cube = round(r"\sqrt[3]{x}");
    assert_eq!(cube.canonical, r"\sqrt[3]{x}");
    assert!(cube.omml.contains("m:degHide m:val=\"0\""));
    round(r"\sqrt[\frac{1}{2}]{x}");
    round(r"\sqrt[3]{\sqrt{x}}");
}
#[test]
fn greek_operators_unicode_and_numbers_keep_exact_symbols() {
    let rendered = round(r"\alpha+\beta\leq\Gamma\times1.25\pm\infty");
    for symbol in ['α', 'β', '≤', 'Γ', '×', '±', '∞'] {
        assert!(rendered.canonical.contains(symbol));
    }
    round("Ω+é=42");
    let epsilon = round(r"\epsilon+\varepsilon+\phi+\varphi");
    assert_eq!(epsilon.canonical, "ϵ+ε+ϕ+φ");
}
#[test]
fn text_runs_xml_characters_and_grouping_are_escaped_faithfully() {
    let rendered = round(r"\text{A \& B} < x\%+\text{a{b}c}");
    assert!(rendered.omml.contains("&amp;"));
    assert!(rendered.omml.contains("&lt;"));
    assert!(rendered.canonical.contains(r"\text{A \& B}"));
    assert!(rendered.canonical.contains(r"\text{abc}"));
    round(r"\backslash+\text{C:\textbackslash{}notes}");
}
#[test]
fn matrix_rows_cells_and_bracket_variants_are_actual_math_structures() {
    for env in ["matrix", "pmatrix", "bmatrix"] {
        let source = format!("\\begin{{{env}}}a&b\\\\c&d\\end{{{env}}}");
        let rendered = round(&source);
        let doc = roxmltree::Document::parse(&rendered.omml).unwrap();
        assert_eq!(
            doc.descendants()
                .filter(|n| n.tag_name().namespace() == Some(M) && n.tag_name().name() == "mr")
                .count(),
            2
        );
        assert!(doc.descendants().any(|n| n.tag_name().name() == "m"));
        if env != "matrix" {
            assert!(doc.descendants().any(|n| n.tag_name().name() == "d"));
        }
    }
}
#[test]
fn supported_math_delimiters_spacing_and_shorthand_normalize_without_changing_values() {
    for source in [
        r"$x^2$",
        r"$$\frac12$$",
        r"\(x+1\)",
        r"\[x+1\]",
        r"x\, +\;y\!+\quad z",
    ] {
        round(source);
    }
    assert_eq!(equations::canonical("x = 2").unwrap(), "x=2");
}
#[test]
fn unsupported_commands_and_malformed_expressions_fail_instead_of_guessing() {
    for source in [
        r"\unknown{x}",
        r"\hat{x}",
        r"\left(x\right)",
        r"\begin{align}a&=b\end{align}",
        r"\frac{1}",
        r"x^^2",
        r"_x",
        r"x_1_2",
        r"{x",
        r"x}",
        r"x&y",
        r"a\\b",
        r"\sqrt[3{x}",
    ] {
        assert!(equations::convert(source).is_err(), "accepted {source:?}");
    }
}
#[test]
fn resource_limits_reject_large_deep_or_irregular_expressions() {
    assert!(equations::convert(&"x".repeat(16001)).is_err());
    assert!(equations::convert(&"x".repeat(4097)).is_err());
    let deep = format!("{}x{}", "{".repeat(40), "}".repeat(40));
    assert!(equations::convert(&deep).is_err());
    assert!(equations::convert(r"\begin{matrix}a&b\\c\end{matrix}").is_err());
    let wide = format!(
        "\\begin{{matrix}}{}\\end{{matrix}}",
        vec!["x"; 21].join("&")
    );
    assert!(equations::convert(&wide).is_err());
}
#[test]
fn unsupported_or_ambiguous_word_math_is_rejected() {
    for inner in [
        "<m:nary/>",
        "<m:f><m:num/><m:den/><m:e/></m:f>",
        "<m:f><m:fPr><m:type m:val=\"noBar\"/></m:fPr><m:num/><m:den/></m:f>",
        "<m:sSup><m:e/><m:sub/><m:sup/></m:sSup>",
        "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg><m:r><m:t>3</m:t></m:r></m:deg><m:e/></m:rad>",
    ] {
        let xml = format!("<m:oMath xmlns:m=\"{M}\">{inner}</m:oMath>");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert!(
            equations::from_omml(doc.root_element()).is_err(),
            "accepted {inner}"
        );
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    project: Project,
    document: PathBuf,
}
fn fixture(source: &str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = Project::open(
        temp.path().join("project"),
        Manifest {
            schema: 2,
            input_sha256: "source".into(),
            fingerprint: "settings".into(),
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
            content: PageContent {
                blocks: vec![Block {
                    kind: "equation".into(),
                    text: source.into(),
                    bbox: Some(BoxRect::full()),
                    uncertain: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    let document = temp.path().join("equations.docx");
    word::export(&project, &document, &Control::default()).unwrap();
    Fixture {
        _temp: temp,
        project,
        document,
    }
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
fn change_math(input: &Path, output: &Path, change: impl Fn(&str) -> String) {
    let mut zip = zip::ZipArchive::new(fs::File::open(input).unwrap()).unwrap();
    let mut writer = zip::ZipWriter::new(fs::File::create(output).unwrap());
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let name = file.name().to_string();
        let mut bytes = vec![];
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            bytes = change(&String::from_utf8(bytes).unwrap()).into_bytes();
        }
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap();
}
#[test]
fn word_export_keeps_source_tags_and_uses_editable_math_without_false_roundtrip_edits() {
    let f = fixture(r"x^2+\frac{1}{y_i}");
    let xml = part(&f.document, "word/document.xml");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    assert!(
        doc.descendants()
            .any(|n| n.tag_name().namespace() == Some(M) && n.tag_name().name() == "oMath")
    );
    assert!(xml.contains("ng:p1:b0:text"));
    assert!(xml.contains("verify this equation"));
    let preview = roundtrip::preview(&f.project, &f.document, &Control::default()).unwrap();
    assert_eq!(preview.fields[0].status, "unchanged");
    assert_eq!(preview.fields[0].baseline, r"x^2+\frac{1}{y_i}");
}
#[test]
fn edited_office_math_roundtrips_to_a_selected_undoable_latex_correction() {
    let f = fixture(r"x^2+\frac{1}{y}");
    let edited = f.document.parent().unwrap().join("edited.docx");
    change_math(&f.document, &edited, |xml| {
        xml.replacen(
            "<m:t xml:space=\"preserve\">2</m:t>",
            "<m:t xml:space=\"preserve\">3</m:t>",
            1,
        )
    });
    let preview = roundtrip::preview(&f.project, &edited, &Control::default()).unwrap();
    assert_eq!(preview.fields[0].status, "changed");
    assert_eq!(
        preview.fields[0].imported.as_deref(),
        Some(r"x^{3}+\frac{1}{y}")
    );
    roundtrip::apply(
        &f.project,
        &edited,
        &preview,
        &["ng:p1:b0:text".into()],
        "Word equation correction",
        &Control::default(),
    )
    .unwrap();
    assert_eq!(
        f.project.load(1).unwrap().content.blocks[0].text,
        r"x^{3}+\frac{1}{y}"
    );
    review::undo(&f.project).unwrap();
    assert_eq!(
        f.project.load(1).unwrap().content.blocks[0].text,
        r"x^2+\frac{1}{y}"
    );
}
#[test]
fn unsupported_source_latex_is_retained_with_an_explicit_review_caption() {
    let source = r"\hat{x}+\unsupported{y}";
    let f = fixture(source);
    let xml = part(&f.document, "word/document.xml");
    assert!(xml.contains("REVIEW EQUATION"));
    assert!(xml.contains(source));
    let preview = roundtrip::preview(&f.project, &f.document, &Control::default()).unwrap();
    assert_eq!(preview.fields[0].status, "unchanged");
    assert_eq!(f.project.load(1).unwrap().content.blocks[0].text, source);
}
#[test]
fn unsupported_word_math_changes_stay_unresolved_and_never_replace_recognition() {
    let f = fixture(r"x^2");
    let edited = f.document.parent().unwrap().join("unsupported.docx");
    change_math(&f.document, &edited, |xml| {
        xml.replace("<m:sSup>", "<m:nary>")
            .replace("</m:sSup>", "</m:nary>")
    });
    let preview = roundtrip::preview(&f.project, &edited, &Control::default()).unwrap();
    assert_eq!(preview.fields[0].status, "structural_conflict");
    assert!(
        roundtrip::apply(
            &f.project,
            &edited,
            &preview,
            &["ng:p1:b0:text".into()],
            "Unsupported math",
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(f.project.load(1).unwrap().content.blocks[0].text, r"x^2");
}
