use notegen::{
    review::{self, Rule},
    storage::{Manifest, Project, atomic_json, fingerprint},
    types::*,
};
use serde_json::json;

struct Fixture {
    _temp: tempfile::TempDir,
    project: Project,
}
fn fixture(pages: usize) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = Project::open(
        temp.path().into(),
        Manifest {
            schema: 2,
            input_sha256: "test-source".into(),
            fingerprint: "test-settings".into(),
            pages: (1..=pages).collect(),
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    for n in 1..=pages {
        project
            .save(&Page {
                page: n,
                image: format!("page-{n:05}.png"),
                content: PageContent {
                    blocks: vec![Block::text(format!("Note {n}: 10.50"))],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
    }
    Fixture {
        _temp: temp,
        project,
    }
}
fn edit(p: &Project, page: usize, text: &str, label: &str) -> u64 {
    let before = p.load(page).unwrap();
    let mut after = before.clone();
    after.content.blocks[0].text = text.into();
    review::commit(p, &before, &after, label).unwrap()
}
fn rule() -> Rule {
    Rule {
        find: "Note".into(),
        replace: "Confirmed".into(),
        pages: vec![1, 2],
        exclude_blocks: vec![],
    }
}

#[test]
fn revisions_undo_and_redo_preserve_original_recognition() {
    let f = fixture(1);
    let p = &f.project;
    let original = std::fs::read(p.page_path(1)).unwrap();
    edit(p, 1, "first correction", "Text correction");
    edit(p, 1, "second correction", "Another correction");
    assert_eq!(review::timeline(p).unwrap().cursor, 2);
    review::undo(p).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "first correction");
    review::undo(p).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "Note 1: 10.50");
    assert!(review::undo(p).is_err());
    review::redo(p).unwrap();
    review::redo(p).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "second correction");
    assert!(review::redo(p).is_err());
    assert_eq!(std::fs::read(p.page_path(1)).unwrap(), original);
}
#[test]
fn a_revision_preserves_approval_geometry_cells_and_uncertainty() {
    let f = fixture(1);
    let p = &f.project;
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    after.reviewed = true;
    after.content.blocks[0].bbox = Some(BoxRect {
        left: 0.1,
        top: 0.2,
        right: 0.7,
        bottom: 0.8,
    });
    after.content.blocks[0].uncertain = true;
    after.content.blocks.push(Block {
        kind: "table".into(),
        rows: vec![vec!["Amount".into()], vec!["42".into()]],
        ..Default::default()
    });
    review::commit(p, &before, &after, "Reviewed region and cell").unwrap();
    let saved = p.load(1).unwrap();
    assert!(saved.reviewed);
    assert_eq!(saved.content.blocks[1].rows[1][0], "42");
    review::undo(p).unwrap();
    let restored = p.load(1).unwrap();
    assert!(!restored.reviewed);
    assert_eq!(restored.content.blocks.len(), 1);
    assert!(restored.content.blocks[0].bbox.is_none());
    review::redo(p).unwrap();
    assert_eq!(
        fingerprint(&p.load(1).unwrap()).unwrap(),
        fingerprint(&saved).unwrap()
    );
}
#[test]
fn stale_edits_are_rejected_without_adding_history() {
    let f = fixture(1);
    let p = &f.project;
    let stale = p.load(1).unwrap();
    edit(p, 1, "newest", "Recent edit");
    let mut replacement = stale.clone();
    replacement.content.blocks[0].text = "stale overwrite".into();
    assert!(review::commit(p, &stale, &replacement, "Stale").is_err());
    assert_eq!(p.load(1).unwrap().content.plain(), "newest");
    assert_eq!(review::timeline(p).unwrap().revisions.len(), 1);
}
#[test]
fn a_new_edit_after_undo_branches_without_reusing_revision_ids() {
    let f = fixture(1);
    let p = &f.project;
    let first = edit(p, 1, "first", "First");
    let discarded = edit(p, 1, "discarded", "Discarded");
    review::undo(p).unwrap();
    let replacement = edit(p, 1, "branch", "Branch");
    assert!(replacement > discarded);
    assert!(review::redo(p).is_err());
    assert!(review::restore(p, discarded).is_err());
    review::restore(p, first).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "first");
}
#[test]
fn named_restore_is_atomic_across_pages_and_has_a_read_only_snapshot() {
    let f = fixture(2);
    let p = &f.project;
    let first = edit(p, 1, "A", "Page one");
    edit(p, 2, "B", "Page two");
    let last = edit(p, 1, "C", "Page one again");
    assert_eq!(review::snapshot(p, first, 1).unwrap().content.plain(), "A");
    assert_eq!(p.load(1).unwrap().content.plain(), "C");
    let restored = review::restore(p, first).unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(p.load(1).unwrap().content.plain(), "A");
    assert_eq!(p.load(2).unwrap().content.plain(), "Note 2: 10.50");
    review::restore(p, last).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "C");
    assert_eq!(p.load(2).unwrap().content.plain(), "B");
    review::restore(p, 0).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "Note 1: 10.50");
}
#[test]
fn invalid_names_source_identities_and_boxes_never_change_a_page() {
    let f = fixture(1);
    let p = &f.project;
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    assert!(review::commit(p, &before, &after, "").is_err());
    assert!(review::commit(p, &before, &after, &"x".repeat(201)).is_err());
    after.page = 2;
    assert!(review::commit(p, &before, &after, "Bad page").is_err());
    after = before.clone();
    after.image = "../source.png".into();
    assert!(review::commit(p, &before, &after, "Bad image").is_err());
    after = before.clone();
    after.content.blocks[0].bbox = Some(BoxRect {
        left: 0.8,
        right: 0.2,
        top: 0.1,
        bottom: 0.9,
    });
    assert!(review::commit(p, &before, &after, "Bad box").is_err());
    assert_eq!(
        fingerprint(&p.load(1).unwrap()).unwrap(),
        fingerprint(&before).unwrap()
    );
    assert!(review::timeline(p).unwrap().revisions.is_empty());
}
#[test]
fn review_file_lock_rejects_concurrent_writers() {
    let f = fixture(1);
    let path = f.project.root.join("review.lock");
    let guard = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    guard.try_lock().unwrap();
    assert!(review::timeline(&f.project).is_err());
    drop(guard);
    assert!(review::timeline(&f.project).is_ok());
}
#[test]
fn untracked_external_edits_do_not_silently_break_the_revision_chain() {
    let f = fixture(1);
    let p = &f.project;
    edit(p, 1, "first", "First");
    let mut external = p.load(1).unwrap();
    external.content.blocks[0].text = "external".into();
    p.save(&external).unwrap();
    let mut next = external.clone();
    next.content.blocks[0].text = "next".into();
    assert!(review::commit(p, &external, &next, "Should fail").is_err());
    assert!(review::undo(p).is_err());
    assert_eq!(p.load(1).unwrap().content.plain(), "external");
}
#[test]
fn differences_include_context_uncertainty_approval_and_region_changes() {
    let f = fixture(1);
    let before = f.project.load(1).unwrap();
    let mut after = before.clone();
    after.reviewed = true;
    after.content.blocks[0].text = "context result".into();
    after.content.blocks[0].uncertain = true;
    after.content.blocks[0].bbox = Some(BoxRect::full());
    after.content.warnings.push("New ambiguity".into());
    let differences = review::compare(&before, &after).unwrap();
    for field in ["reviewed", "text", "uncertain", "box", "warnings"] {
        assert!(
            differences.iter().any(|d| d.field == field),
            "missing {field}"
        );
    }
    assert!(review::compare(&before, &before).unwrap().is_empty());
    after.page = 2;
    assert!(review::compare(&before, &after).is_err());
}
#[test]
fn literal_correction_preview_does_not_write_and_applies_as_one_undoable_revision() {
    let f = fixture(2);
    let p = &f.project;
    let preview = review::preview_rule(p, &rule()).unwrap();
    assert_eq!(preview.matches.len(), 2);
    assert_eq!(preview.matches[0].occurrences, 1);
    assert_eq!(p.load(1).unwrap().content.plain(), "Note 1: 10.50");
    review::apply_rule(p, &preview, "Confirmed terminology").unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "Confirmed 1: 10.50");
    assert_eq!(p.load(2).unwrap().content.plain(), "Confirmed 2: 10.50");
    assert_eq!(review::timeline(p).unwrap().revisions.len(), 1);
    review::undo(p).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "Note 1: 10.50");
    assert_eq!(p.load(2).unwrap().content.plain(), "Note 2: 10.50");
}
#[test]
fn correction_exclusions_and_evidence_are_respected() {
    let f = fixture(2);
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.candidates.push(Candidate {
        region: None,
        model_id: "reader".into(),
        family: "family".into(),
        view: "full".into(),
        content: page.content.clone(),
        reliability: 1.0,
    });
    p.save(&page).unwrap();
    let mut r = rule();
    r.exclude_blocks = vec![(2, 0)];
    let preview = review::preview_rule(p, &r).unwrap();
    assert_eq!(preview.matches.len(), 1);
    review::apply_rule(p, &preview, "Selected correction").unwrap();
    let saved = p.load(1).unwrap();
    assert_eq!(saved.candidates[0].content.plain(), "Note 1: 10.50");
    assert_eq!(p.load(2).unwrap().content.plain(), "Note 2: 10.50");
}
#[test]
fn altered_and_stale_rule_previews_are_rejected() {
    let f = fixture(2);
    let p = &f.project;
    let mut preview = review::preview_rule(p, &rule()).unwrap();
    preview.matches[0].after = "tampered".into();
    assert!(review::apply_rule(p, &preview, "Bad preview").is_err());
    let preview = review::preview_rule(p, &rule()).unwrap();
    edit(p, 1, "changed", "Another edit");
    assert!(review::apply_rule(p, &preview, "Stale preview").is_err());
    assert_eq!(p.load(2).unwrap().content.plain(), "Note 2: 10.50");
}
#[test]
fn empty_duplicate_or_foreign_rule_pages_are_rejected() {
    let f = fixture(2);
    let p = &f.project;
    let mut r = rule();
    r.find.clear();
    assert!(review::preview_rule(p, &r).is_err());
    r = rule();
    r.pages = vec![];
    assert!(review::preview_rule(p, &r).is_err());
    r.pages = vec![1, 1];
    assert!(review::preview_rule(p, &r).is_err());
    r.pages = vec![99];
    assert!(review::preview_rule(p, &r).is_err());
}
#[test]
fn table_cell_rules_and_interpretations_are_previewed_individually() {
    let f = fixture(1);
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks = vec![
        Block {
            kind: "table".into(),
            rows: vec![vec!["Note".into(), "Note Note".into()]],
            ..Default::default()
        },
        Block {
            kind: "figure".into(),
            bbox: Some(BoxRect::full()),
            description: "Note visible".into(),
            interpretation: "Note context".into(),
            ..Default::default()
        },
    ];
    p.save(&page).unwrap();
    let mut r = rule();
    r.pages = vec![1];
    let preview = review::preview_rule(p, &r).unwrap();
    assert_eq!(preview.matches.len(), 4);
    assert!(
        preview
            .matches
            .iter()
            .any(|m| m.field == "cell:0:1" && m.occurrences == 2)
    );
    review::apply_rule(p, &preview, "Cells and labels").unwrap();
    let saved = p.load(1).unwrap();
    assert_eq!(saved.content.blocks[0].rows[0][1], "Confirmed Confirmed");
    assert_eq!(saved.content.blocks[1].description, "Confirmed visible");
}
fn table(rows: &[&[&str]]) -> Block {
    Block {
        kind: "table".into(),
        header_rows: 1,
        rows: rows
            .iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect(),
        ..Default::default()
    }
}
#[test]
fn table_checks_report_total_errors_without_rewriting_values() {
    let b = table(&[
        &["Item", "Amount"],
        &["A", "10.25"],
        &["B", "2.50"],
        &["Total", "13.00"],
    ]);
    let issues = review::table_issues(1, 0, &b);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "total");
    assert_eq!(issues[0].row, Some(3));
    assert_eq!(b.rows[3][1], "13.00");
    let mut b = b;
    b.rows[3][1] = "12.75".into();
    assert!(review::table_issues(1, 0, &b).is_empty());
}
#[test]
fn table_checks_skip_ambiguous_grouped_numbers_and_merged_totals() {
    let b = table(&[
        &["Item", "Amount"],
        &["A", "1,000"],
        &["B", "2,000"],
        &["Total", "1"],
    ]);
    assert!(review::table_issues(1, 0, &b).is_empty());
    let mut b = table(&[
        &["Item", "Amount"],
        &["A", "10"],
        &["B", "20"],
        &["Total", "1"],
    ]);
    b.merges.push(Merge {
        row: 0,
        col: 0,
        rowspan: 1,
        colspan: 2,
    });
    assert!(review::table_issues(1, 0, &b).is_empty());
}
#[test]
fn table_checks_handle_negative_amounts_scientific_notation_and_subtotals() {
    let b = table(&[
        &["Item", "Amount"],
        &["A", "$10"],
        &["B", "(2)"],
        &["Subtotal", "8"],
        &["C", "1e1"],
        &["D", "-2"],
        &["Subtotal", "8"],
        &["Grand total", "16"],
    ]);
    assert!(review::table_issues(1, 0, &b).is_empty());
    let mut b = b;
    b.rows[6][1] = "9".into();
    let issues = review::table_issues(1, 0, &b);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].row, Some(6));
}
#[test]
fn table_checks_flag_types_and_ragged_rows() {
    let b = table(&[
        &["Item", "Amount"],
        &["A", "10"],
        &["B", "20"],
        &["C", "2O"],
    ]);
    let issues = review::table_issues(1, 0, &b);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "column-type");
    assert_eq!(issues[0].reading, "2O");
    let mut b = b;
    b.rows[1].pop();
    assert_eq!(review::table_issues(1, 0, &b)[0].kind, "row-length");
}
#[test]
fn impact_queue_prioritizes_numeric_conflicts_and_omits_approved_pages() {
    let f = fixture(2);
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks[0].confidence = Some(0.99);
    page.content.blocks[0].alternatives = vec!["reader-v2: Note 1: 10.60".into()];
    page.content.blocks.push(Block {
        text: "ordinary uncertain word".into(),
        kind: "text".into(),
        uncertain: true,
        ..Default::default()
    });
    p.save(&page).unwrap();
    let mut approved = p.load(2).unwrap();
    approved.reviewed = true;
    p.save(&approved).unwrap();
    let issues = review::issues(p).unwrap();
    assert_eq!(issues[0].kind, "numeric-conflict");
    assert_eq!(issues[0].priority, 100);
    assert!(issues.iter().all(|i| i.page == 1));
    assert!(issues.iter().any(|i| i.block == 1));
}
#[test]
fn numeric_reader_labels_do_not_create_false_conflicts() {
    let f = fixture(1);
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    page.content.blocks[0].confidence = Some(0.99);
    page.content.blocks[0].alternatives = vec!["reader-v2: Note 1: 10.50".into()];
    p.save(&page).unwrap();
    assert!(review::issues(p).unwrap().is_empty());
}

#[test]
fn issue_previews_bound_unicode_and_alternatives_but_numeric_checks_use_full_evidence() {
    let f = fixture(1);
    let p = &f.project;
    let mut page = p.load(1).unwrap();
    let prefix = "සිංහල தமிழ் ".repeat(1000);
    page.content.blocks[0].text = format!("{prefix} total 100");
    page.content.blocks[0].confidence = Some(1.0);
    page.content.blocks[0].alternatives = (0..64)
        .map(|index| format!("reader-{index}: {prefix} total 200"))
        .collect();
    p.save(&page).unwrap();
    let before = std::fs::read(p.page_path(1)).unwrap();
    let issues = review::issues(p).unwrap();
    assert_eq!(issues.len(), 1);
    let issue = &issues[0];
    assert_eq!(issue.kind, "numeric-conflict");
    assert_eq!(issue.priority, 100);
    assert!(issue.preview_truncated);
    assert!(issue.reading.ends_with('…'));
    assert!(issue.reading.chars().count() <= review::ISSUE_PREVIEW_CHARS + 1);
    assert_eq!(issue.alternatives.len(), review::ISSUE_PREVIEW_ALTERNATIVES);
    assert!(
        issue
            .alternatives
            .iter()
            .all(|alternative| alternative.ends_with('…')
                && alternative.chars().count() <= review::ISSUE_PREVIEW_CHARS + 1)
    );
    assert!(
        issue
            .message
            .contains("all 64 reader alternatives remain in the project")
    );
    assert_eq!(std::fs::read(p.page_path(1)).unwrap(), before);
    let saved = p.load(1).unwrap();
    assert_eq!(saved.content.blocks[0].alternatives.len(), 64);
    assert!(saved.content.blocks[0].text.ends_with("total 100"));
}

#[test]
fn large_tables_keep_high_impact_totals_and_report_source_free_preview_truncation() {
    let mut b = Block {
        kind: "table".into(),
        header_rows: 1,
        ..Default::default()
    };
    b.rows.push(
        (0..40)
            .map(|column| {
                if column == 0 {
                    "Label".into()
                } else {
                    "Amount".into()
                }
            })
            .collect(),
    );
    for row in 1..100 {
        b.rows.push(
            (0..40)
                .map(|column| {
                    if column == 0 {
                        if row == 99 {
                            "Grand total".into()
                        } else {
                            format!("Item {row}")
                        }
                    } else if row == 99 {
                        "999".into()
                    } else if column <= 20 && row >= 61 {
                        "unclear amount".into()
                    } else {
                        "1".into()
                    }
                })
                .collect(),
        );
    }
    b.alternatives = vec!["සිංහල reader evidence ".repeat(1000); 64];
    b.validate().unwrap();
    let before = fingerprint(&b).unwrap();
    let issues = review::table_issues(1, 0, &b);
    assert_eq!(issues.len(), review::MAX_TABLE_ISSUES + 1);
    let summary = &issues[0];
    assert_eq!(summary.kind, "table-truncated");
    assert_eq!(summary.page, 0);
    assert!(summary.reading.is_empty());
    assert!(summary.region.is_none());
    assert!(summary.alternatives.is_empty());
    assert!(summary.message.contains("further suggestions were omitted"));
    let details = &issues[1..];
    assert_eq!(
        details.iter().filter(|issue| issue.kind == "total").count(),
        19
    );
    assert!(
        details
            .windows(2)
            .all(|pair| pair[0].priority >= pair[1].priority)
    );
    assert!(details.iter().all(|issue| issue.alternatives.len()
        <= review::ISSUE_PREVIEW_ALTERNATIVES
        && issue.reading.chars().count() <= review::ISSUE_PREVIEW_CHARS + 1));
    assert_eq!(fingerprint(&b).unwrap(), before);
}

#[test]
fn review_queue_retains_late_high_impact_issues_with_explicit_cap_and_exposes_remaining_pages() {
    let f = fixture(21);
    let p = &f.project;
    for n in 1..=21 {
        let mut page = p.load(n).unwrap();
        page.content.blocks = (0..500)
            .map(|_| Block::text("ordinary uncertain words"))
            .collect();
        if n == 21 {
            page.content.blocks[499] = Block {
                text: "Confirmed amount 100".into(),
                confidence: Some(1.0),
                alternatives: vec!["reader: Confirmed amount 200".into()],
                ..Block::text("")
            };
        }
        p.save(&page).unwrap();
    }
    let before = std::fs::read(p.page_path(21)).unwrap();
    let issues = review::issues(p).unwrap();
    assert_eq!(issues.len(), review::MAX_REVIEW_ISSUES + 1);
    let summary = &issues[0];
    assert_eq!(summary.kind, "queue-truncated");
    assert_eq!(summary.page, 0);
    assert!(
        summary.reading.is_empty() && summary.alternatives.is_empty() && summary.region.is_none()
    );
    assert!(summary.message.contains("10500"));
    assert!(summary.message.contains("500 were omitted"));
    assert_eq!(
        (issues[1].page, issues[1].block, issues[1].priority),
        (21, 499, 100)
    );
    assert!(issues.iter().skip(1).all(|issue| issue.page != 0));
    assert_eq!(std::fs::read(p.page_path(21)).unwrap(), before);
    let mut page = p.load(1).unwrap();
    page.reviewed = true;
    p.save(&page).unwrap();
    let refreshed = review::issues(p).unwrap();
    assert_eq!(refreshed.len(), review::MAX_REVIEW_ISSUES);
    assert!(
        !refreshed
            .iter()
            .any(|issue| issue.kind == "queue-truncated")
    );
    assert!(
        refreshed
            .iter()
            .any(|issue| issue.page == 21 && issue.block == 499)
    );
}

#[test]
fn review_transactions_use_the_same_candidate_validation_as_project_storage() {
    let f = fixture(1);
    let p = &f.project;
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    after.candidates.push(Candidate {
        region: None,
        model_id: "reader".into(),
        family: "family".into(),
        view: "full".into(),
        content: PageContent::default(),
        reliability: 5.0,
    });
    assert!(after.validate().is_err());
    assert!(review::commit(p, &before, &after, "Excessive reader reliability").is_err());
    assert!(!p.root.join("review-history.json").exists());
    assert!(!p.root.join("review-pending.json").exists());
    assert_eq!(
        fingerprint(&p.load(1).unwrap()).unwrap(),
        fingerprint(&before).unwrap()
    );
}
#[test]
fn interrupted_transactions_recover_idempotently_and_keep_original_pages() {
    let f = fixture(1);
    let p = &f.project;
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    after.edited = true;
    after.content.blocks[0].text = "Recovered".into();
    let project = fingerprint(&p.manifest).unwrap();
    let old = json!({"schema":1,"project":project,"next_id":1,"cursor":0,"revisions":[]});
    let next = json!({"schema":1,"project":project,"next_id":2,"cursor":1,"revisions":[{"id":1,"label":"Interrupted correction","created":"2026-11-02T00:00:00Z","changes":[{"before":before,"after":after}]}]});
    atomic_json(&p.root.join("review-pending.json"),&json!({"schema":1,"project":project,"before_history":fingerprint(&old).unwrap(),"history":next,"changes":[{"before":before,"after":after}]})).unwrap();
    // Fingerprints are defined over typed serialization, whose key order is stable.
    let mut pending: serde_json::Value =
        serde_json::from_slice(&std::fs::read(p.root.join("review-pending.json")).unwrap())
            .unwrap();
    let typed_order = format!(
        "{{\"schema\":1,\"project\":\"{}\",\"next_id\":1,\"cursor\":0,\"revisions\":[]}}",
        project
    );
    use sha2::{Digest, Sha256};
    pending["before_history"] = json!(hex::encode(Sha256::digest(typed_order.as_bytes())));
    atomic_json(&p.root.join("review-pending.json"), &pending).unwrap();
    review::recover(p).unwrap();
    review::recover(p).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "Recovered");
    assert!(!p.root.join("review-pending.json").exists());
    assert_eq!(review::timeline(p).unwrap().cursor, 1);
    let original: Page = serde_json::from_slice(&std::fs::read(p.page_path(1)).unwrap()).unwrap();
    assert_eq!(original.content.plain(), "Note 1: 10.50");
}
#[test]
fn recovery_preserves_conflicts_and_rejects_another_projects_history() {
    let f = fixture(1);
    let p = &f.project;
    edit(p, 1, "Saved", "Saved");
    let history_path = p.root.join("review-history.json");
    let mut h: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&history_path).unwrap()).unwrap();
    h["project"] = json!("different-project");
    atomic_json(&history_path, &h).unwrap();
    assert!(review::timeline(p).is_err());
    assert_eq!(p.load(1).unwrap().content.plain(), "Saved");
}
