use notegen::{
    document_tools::{self as tools, RecurringAction},
    storage::{Control, Manifest, Project},
    types::*,
};
use std::{fs, sync::atomic::Ordering};
fn block(text: &str, top: f32, bottom: f32) -> Block {
    Block {
        text: text.into(),
        kind: "text".into(),
        bbox: Some(BoxRect {
            left: 0.1,
            top,
            right: 0.9,
            bottom,
        }),
        confidence: Some(0.95),
        ..Default::default()
    }
}
fn fixture(root: &std::path::Path) -> Project {
    let project = Project::open(
        root.join("project"),
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
    for n in 1..=3 {
        let text = match n {
            1 => "Latin العربية עברית සිංහල தமிழ் हिन्दी 中文",
            2 => "Alice has 12 kg. ID: AB01. NASA cites [1] and [9].",
            _ => "Alyce has 12 g. ID: AB0I. N.A.S.A. cites [1].",
        };
        let mut blocks = vec![
            block("Course Notes", 0.02, 0.08),
            block(text, 0.2, 0.4),
            block(&format!("Page {n} of 3"), 0.92, 0.98),
        ];
        if n >= 2 {
            blocks.push(block(
                if n == 2 {
                    "[1] First reference"
                } else {
                    "[1] Another reference"
                },
                0.72,
                0.82,
            ));
        }
        let page = Page {
            page: n,
            image: format!("page-{n:05}.png"),
            agreement: 0.95,
            content: PageContent {
                blocks,
                ..Default::default()
            },
            ..Default::default()
        };
        project.save(&page).unwrap();
    }
    project
}

#[test]
fn recognized_unicode_script_hints_cover_mixed_scripts_and_direction_with_manual_override() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let c = Control::default();
    let before = fs::read(p.page_path(1)).unwrap();
    let hints = tools::script_hints(&p, Some("sin+tam"), &c).unwrap();
    let first = &hints[0];
    for name in [
        "Latin",
        "Arabic",
        "Hebrew",
        "Sinhala",
        "Tamil",
        "Devanagari",
        "CJK",
    ] {
        assert!(
            first.scripts.get(name).is_some_and(|n| *n > 0),
            "missing {name}"
        );
    }
    assert_eq!(first.state, "mixed_scripts");
    assert_eq!(first.direction, "mixed");
    assert_eq!(first.language_override.as_deref(), Some("sin+tam"));
    assert!(
        first
            .note
            .contains("does not detect a script from an image")
    );
    assert_eq!(fs::read(p.page_path(1)).unwrap(), before);
    let corrected = tools::language_override(&p.manifest.settings, "ara+heb").unwrap();
    assert_eq!(corrected.languages, "ara+heb");
    assert_eq!(p.manifest.settings.languages, "eng");
    assert!(tools::language_override(&p.manifest.settings, "../sin").is_err());
}
#[test]
fn script_hints_explicitly_report_no_text_unknown_and_rtl_without_using_numeric_direction() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    for (n, text) in [(1, "12345 . ,"), (2, "Ελληνικά"), (3, "עברית العربية 123")]
    {
        let mut page = p.load(n).unwrap();
        page.content.blocks = vec![Block::text(text)];
        p.save(&page).unwrap();
    }
    let hints = tools::script_hints(&p, None, &Control::default()).unwrap();
    assert_eq!(hints[0].state, "no_text");
    assert_eq!(hints[0].direction, "unknown");
    assert_eq!(hints[1].state, "unknown");
    assert_eq!(hints[2].direction, "rtl");
}
#[test]
fn recurring_elements_require_real_margin_boxes_on_multiple_pages_and_only_selected_groups_are_removed()
 {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let c = Control::default();
    let proposals = tools::recurring_elements(&p, &c).unwrap();
    let header = proposals.iter().find(|p| p.kind == "header").unwrap();
    assert_eq!(header.occurrences.len(), 3);
    assert!(
        header
            .occurrences
            .iter()
            .all(|s| s.region.as_ref().unwrap().bottom <= 0.15)
    );
    assert!(proposals.iter().any(|p| p.kind == "footer_page_number"));
    assert_eq!(
        tools::apply_recurring(
            &p,
            std::slice::from_ref(&header.id),
            RecurringAction::Keep,
            &c
        )
        .unwrap(),
        None
    );
    assert!(!p.root.join("review-history.json").exists());
    let source_checkpoint = fs::read(p.page_path(1)).unwrap();
    let revision = tools::apply_recurring(
        &p,
        std::slice::from_ref(&header.id),
        RecurringAction::Remove,
        &c,
    )
    .unwrap();
    assert!(revision.is_some());
    for n in 1..=3 {
        let page = p.load(n).unwrap();
        assert!(!page.content.blocks.iter().any(|b| b.text == "Course Notes"));
        assert!(
            page.content
                .blocks
                .iter()
                .any(|b| b.text.starts_with("Page "))
        );
        assert!(!page.reviewed);
    }
    assert_eq!(fs::read(p.page_path(1)).unwrap(), source_checkpoint);
    notegen::review::undo(&p).unwrap();
    assert_eq!(p.load(1).unwrap().content.blocks[0].text, "Course Notes");
    notegen::review::redo(&p).unwrap();
    assert!(
        !p.load(1)
            .unwrap()
            .content
            .blocks
            .iter()
            .any(|b| b.text == "Course Notes")
    );
}
#[test]
fn missing_or_body_position_boxes_do_not_become_recurring_headers_and_stale_previews_fail() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let c = Control::default();
    let proposals = tools::recurring_elements(&p, &c).unwrap();
    let id = proposals
        .iter()
        .find(|p| p.kind == "header")
        .unwrap()
        .id
        .clone();
    for n in 1..=2 {
        let mut page = p.load(n).unwrap();
        page.content.blocks[0].bbox = if n == 1 {
            None
        } else {
            Some(BoxRect {
                left: 0.1,
                top: 0.3,
                right: 0.9,
                bottom: 0.4,
            })
        };
        p.save(&page).unwrap();
    }
    assert!(
        !tools::recurring_elements(&p, &c)
            .unwrap()
            .iter()
            .any(|p| p.kind == "header")
    );
    assert!(tools::apply_recurring(&p, &[id], RecurringAction::Remove, &c).is_err());
    assert!(!p.root.join("review-history.json").exists());
    assert!(tools::apply_recurring(&p, &[], RecurringAction::Remove, &c).is_err());
}
#[test]
fn citation_and_footnote_links_preserve_ambiguity_and_unresolved_markers_without_forcing_a_match() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let before = fs::read(p.page_path(2)).unwrap();
    let links = tools::reference_links(&p, &Control::default()).unwrap();
    let numeric = links.iter().find(|l| l.marker == "1").unwrap();
    assert_eq!(numeric.status, "ambiguous");
    assert_eq!(numeric.candidates.len(), 2);
    assert!(numeric.candidates.iter().all(|s| s.region.is_some()));
    assert!(
        links
            .iter()
            .any(|l| l.marker == "9" && l.status == "unresolved")
    );
    assert!(!links.iter().any(|l| l.source.block_index == 3));
    assert_eq!(fs::read(p.page_path(2)).unwrap(), before);
    let mut page = p.load(3).unwrap();
    page.content
        .blocks
        .push(block("A phrase with footnote²", 0.4, 0.5));
    page.content
        .blocks
        .push(block("² A superscript footnote", 0.75, 0.8));
    p.save(&page).unwrap();
    let links = tools::reference_links(&p, &Control::default()).unwrap();
    let link = links.iter().find(|l| l.marker == "2").unwrap();
    assert_eq!(link.status, "candidate");
    assert_eq!(link.candidates.len(), 1);
}
#[test]
fn cross_page_suggestions_link_variants_without_rewriting_names_units_identifiers_or_abbreviations()
{
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let before = fs::read(p.page_path(2)).unwrap();
    let issues = tools::consistency_suggestions(&p, &Control::default()).unwrap();
    assert!(issues.iter().any(|s| s.kind == "similar_capitalized_terms"
        && s.variants.contains_key("Alice")
        && s.variants.contains_key("Alyce")));
    assert!(issues.iter().any(|s| s.kind == "unit_variation"));
    assert!(
        issues
            .iter()
            .any(|s| s.kind == "labelled_identifier_variation")
    );
    assert!(
        issues
            .iter()
            .any(|s| s.kind == "capitalization_or_abbreviation" && s.key == "nasa")
    );
    for issue in issues {
        assert!(
            issue
                .variants
                .values()
                .flatten()
                .all(|s| !s.block_fingerprint.is_empty() && !s.source_image.is_empty())
        );
    }
    assert_eq!(fs::read(p.page_path(2)).unwrap(), before);
}
#[test]
fn technical_checks_flag_syntax_and_real_reader_typography_disagreement_without_inventing_exponents()
 {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let mut page = p.load(1).unwrap();
    page.content.blocks = vec![
        block("x2", 0.2, 0.3),
        block("(H2O", 0.4, 0.5),
        block("q^{}", 0.6, 0.7),
    ];
    let mut alternative = page.content.clone();
    alternative.blocks[0].text = "x²".into();
    page.candidates = vec![Candidate {
        region: None,
        model_id: "reader-one".into(),
        family: "test".into(),
        view: "page".into(),
        content: alternative,
        reliability: 0.8,
    }];
    p.save(&page).unwrap();
    let before = fs::read(p.page_path(1)).unwrap();
    let issues = tools::technical_checks(&p, &Control::default()).unwrap();
    assert!(
        issues
            .iter()
            .any(|i| i.code == "unbalanced_notation" && i.source.page == 1)
    );
    assert!(issues.iter().any(|i| i.code == "incomplete_script_marker"));
    let typography = issues
        .iter()
        .find(|i| i.code == "script_typography_disagreement")
        .unwrap();
    assert_eq!(typography.reader_evidence, vec!["reader-one"]);
    assert!(typography.message.contains("not proof of correctness"));
    assert_eq!(fs::read(p.page_path(1)).unwrap(), before);
    let mut page = p.load(1).unwrap();
    page.candidates[0].content.blocks[0].bbox = Some(BoxRect {
        left: 0.1,
        top: 0.8,
        right: 0.9,
        bottom: 0.9,
    });
    p.save(&page).unwrap();
    assert!(
        !tools::technical_checks(&p, &Control::default())
            .unwrap()
            .iter()
            .any(|i| i.code == "script_typography_disagreement")
    );
}
#[test]
fn escaped_latex_delimiters_do_not_produce_false_unbalanced_dollar_warnings_and_all_tools_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let mut page = p.load(1).unwrap();
    page.content.blocks = vec![Block::text("Cost \\$5 and \\{literal\\}. $x_1$ [balanced]")];
    p.save(&page).unwrap();
    assert!(
        !tools::technical_checks(&p, &Control::default())
            .unwrap()
            .iter()
            .any(|i| i.source.page == 1)
    );
    let c = Control::default();
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(tools::script_hints(&p, None, &c).is_err());
    assert!(tools::recurring_elements(&p, &c).is_err());
    assert!(tools::reference_links(&p, &c).is_err());
    assert!(tools::consistency_suggestions(&p, &c).is_err());
    assert!(tools::technical_checks(&p, &c).is_err());
}
