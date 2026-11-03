use notegen::{
    migrations,
    project_layout::{self as layout, Axis, Operation},
    redactions, review,
    storage::{Control, Manifest, Project},
    types::*,
};
use std::fs;
fn region(left: f32, right: f32) -> BoxRect {
    BoxRect {
        left,
        top: 0.1,
        right,
        bottom: 0.4,
    }
}
fn fixture() -> (tempfile::TempDir, Project) {
    let temp = tempfile::tempdir().unwrap();
    let settings = Settings {
        preset: Preset::Fast,
        models_dir: temp.path().join("models"),
        ..Default::default()
    };
    let p = Project::open(
        temp.path().join("project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1, 2],
            models: vec![],
            settings,
        },
    )
    .unwrap();
    for n in [1, 2] {
        let im = image::RgbImage::from_fn(101, 80, |x, y| image::Rgb([x as u8, y as u8, n as u8]));
        im.save(p.image(n)).unwrap();
        let content = PageContent {
            blocks: vec![
                Block {
                    bbox: Some(region(0.1, 0.4)),
                    ..Block::text(format!("Left {n}250"))
                },
                Block {
                    bbox: Some(region(0.6, 0.9)),
                    ..Block::text(format!("Right {n}550"))
                },
            ],
            ..Default::default()
        };
        p.save(&Page {
            page: n,
            image: format!("page-{n:05}.png"),
            candidates: vec![Candidate {
                model_id: "original-reader".into(),
                family: "original-family".into(),
                view: "full".into(),
                content: content.clone(),
                reliability: 0.8,
                region: None,
            }],
            content,
            ..Default::default()
        })
        .unwrap();
    }
    (temp, p)
}
fn migrate(p: &Project) -> Project {
    let plan = migrations::preview_project_sources(p, &Control::default()).unwrap();
    migrations::apply(&plan, &plan.token, &Control::default()).unwrap();
    Project::read(p.root.clone()).unwrap()
}
fn apply(p: &Project, op: Operation) -> Project {
    let preview = layout::preview(p, op, &Control::default()).unwrap();
    layout::apply(p, &preview, &preview.token, &Control::default()).unwrap()
}
#[test]
fn reorder_remove_and_layout_undo_preserve_source_ids_and_ordinary_review_history() {
    let (_temp, p) = fixture();
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[0].text = "Confirmed 1250".into();
    review::commit(&p, &before, &after, "Correct source text").unwrap();
    let history = fs::read(p.root.join("review-history.json")).unwrap();
    let original = fs::read(p.image(1)).unwrap();
    let p = migrate(&p);
    let ids = layout::catalog(&p).unwrap().unwrap().sources;
    assert!(ids.iter().all(|source| source.original_page.is_none()));
    let reordered = apply(&p, Operation::Reorder { pages: vec![2, 1] });
    assert_eq!(reordered.manifest.pages, vec![2, 1]);
    assert!(p.load(1).is_err());
    assert_eq!(
        fs::read(reordered.root.join("review-history.json")).unwrap(),
        history
    );
    assert_eq!(
        layout::catalog(&reordered).unwrap().unwrap().sources[0].id,
        ids[0].id
    );
    let removed = apply(&reordered, Operation::Remove { pages: vec![1] });
    assert_eq!(removed.manifest.pages, vec![2]);
    assert!(removed.image(1).is_file());
    assert_eq!(
        removed.load(1).unwrap().content.blocks[0].text,
        "Confirmed 1250"
    );
    review::undo(&removed).unwrap();
    assert_eq!(removed.load(1).unwrap().content.blocks[0].text, "Left 1250");
    review::redo(&removed).unwrap();
    let revision = layout::catalog(&removed)
        .unwrap()
        .unwrap()
        .states
        .last()
        .unwrap()
        .revision;
    let restored = layout::undo(&removed, revision, &Control::default()).unwrap();
    assert_eq!(restored.manifest.pages, vec![2, 1]);
    assert_eq!(
        restored.load(1).unwrap().content.blocks[0].text,
        "Confirmed 1250"
    );
    assert_eq!(fs::read(restored.image(1)).unwrap(), original);
}
#[test]
fn split_merge_retain_exact_pixels_parent_evidence_and_source_mappings() {
    let (_temp, p) = fixture();
    let p = migrate(&p);
    let original = image::open(p.image(1)).unwrap().to_rgba8();
    let checkpoint = fs::read(p.page_path(1)).unwrap();
    let preview = layout::preview(
        &p,
        Operation::Split {
            page: 1,
            axis: Axis::Vertical,
            at: 0.5,
        },
        &Control::default(),
    )
    .unwrap();
    assert_eq!(preview.created.len(), 2);
    let slots = preview.created.iter().map(|s| s.slot).collect::<Vec<_>>();
    let split = layout::apply(&p, &preview, &preview.token, &Control::default()).unwrap();
    assert_eq!(image::open(split.image(slots[0])).unwrap().width(), 51);
    assert_eq!(image::open(split.image(slots[1])).unwrap().width(), 50);
    assert!(
        split
            .load(slots[0])
            .unwrap()
            .content
            .plain()
            .contains("Left 1250")
    );
    assert!(
        !split
            .load(slots[0])
            .unwrap()
            .content
            .plain()
            .contains("Right 1550")
    );
    assert!(
        split
            .load(slots[1])
            .unwrap()
            .content
            .plain()
            .contains("Right 1550")
    );
    assert_eq!(fs::read(split.page_path(1)).unwrap(), checkpoint);
    let merged = apply(
        &split,
        Operation::Merge {
            pages: slots,
            axis: Axis::Horizontal,
        },
    );
    let merged_slot = merged.manifest.pages[0];
    assert_eq!(
        image::open(merged.image(merged_slot)).unwrap().to_rgba8(),
        original
    );
    let page = merged.load(merged_slot).unwrap();
    assert!(
        page.content.plain().contains("Left 1250") && page.content.plain().contains("Right 1550")
    );
    assert_eq!(
        layout::catalog(&merged)
            .unwrap()
            .unwrap()
            .sources
            .iter()
            .find(|s| s.slot == merged_slot)
            .unwrap()
            .parts
            .len(),
        2
    );
    assert_eq!(fs::read(merged.page_path(1)).unwrap(), checkpoint);
    assert!(
        merged
            .load(1)
            .unwrap()
            .candidates
            .iter()
            .any(|c| c.model_id == "original-reader")
    );
}
#[test]
fn masks_follow_derived_sources_and_stale_edits_or_manual_files_are_preserved() {
    let (temp, p) = fixture();
    let before = p.load(1).unwrap();
    let preview = redactions::preview_confirmation(
        &p,
        &before,
        &[redactions::MaskSpec {
            region: region(0.6, 0.9),
            reason: redactions::Reason::PersonalInformation,
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
    review::commit(&p, &before, &after, "Confirm cover").unwrap();
    let p = migrate(&p);
    let split = apply(
        &p,
        Operation::Split {
            page: 1,
            axis: Axis::Vertical,
            at: 0.5,
        },
    );
    let right = split.manifest.pages[1];
    let page = split.load(right).unwrap();
    assert!(page.content.plain().contains("[redacted]") && !page.content.plain().contains("1550"));
    assert_eq!(
        page.confirmed_redactions[0].source,
        redactions::source_identity(&split, &page, &Control::default()).unwrap()
    );
    let preview = layout::preview(
        &split,
        Operation::Merge {
            pages: split.manifest.pages[..2].to_vec(),
            axis: Axis::Horizontal,
        },
        &Control::default(),
    )
    .unwrap();
    let mut changed = split.load(right).unwrap();
    let before = changed.clone();
    changed.content.warnings.push("Concurrent user note".into());
    review::commit(&split, &before, &changed, "Concurrent edit").unwrap();
    assert!(layout::apply(&split, &preview, &preview.token, &Control::default()).is_err());
    assert!(
        split
            .load(right)
            .unwrap()
            .content
            .warnings
            .contains(&"Concurrent user note".into())
    );
    let manual = split.root.join("page-00005.png");
    fs::write(&manual, b"unknown manual data").unwrap();
    let input = temp.path().join("insert.png");
    image::RgbImage::from_pixel(20, 20, image::Rgb([255, 255, 255]))
        .save(&input)
        .unwrap();
    let inserted = apply(
        &split,
        Operation::Insert {
            image: input,
            position: 0,
        },
    );
    assert_ne!(inserted.manifest.pages[0], 5);
    assert_eq!(fs::read(manual).unwrap(), b"unknown manual data");
}

#[test]
fn stale_imported_masks_are_never_silently_reconfirmed_by_split_or_merge() {
    let (_temp, p) = fixture();
    let p = migrate(&p);
    let before = p.load(1).unwrap();
    let mut after = before.clone();
    let mut source = redactions::source_identity(&p, &before, &Control::default()).unwrap();
    source.sha256 = "0".repeat(64);
    after.confirmed_redactions = vec![redactions::ConfirmedMask {
        id: "c".repeat(64),
        source,
        region: region(0.1, 0.4),
        reason: redactions::Reason::Other,
    }];
    review::commit(&p, &before, &after, "Imported stale cover metadata").unwrap();
    let catalog = fs::read(p.root.join("source-catalog.json")).unwrap();
    for operation in [
        Operation::Split {
            page: 1,
            axis: Axis::Vertical,
            at: 0.5,
        },
        Operation::Merge {
            pages: vec![1, 2],
            axis: Axis::Horizontal,
        },
    ] {
        assert!(layout::preview(&p, operation, &Control::default()).is_err());
    }
    assert_eq!(
        fs::read(p.root.join("source-catalog.json")).unwrap(),
        catalog
    );
    assert_eq!(
        p.load(1).unwrap().confirmed_redactions[0].source.sha256,
        "0".repeat(64)
    );
    assert!(!p.image(3).exists());
}
#[test]
fn derived_crop_evidence_keeps_its_narrow_scope_and_concurrent_lock_is_preserved() {
    let (_temp, p) = fixture();
    let mut page = p.load(1).unwrap();
    page.candidates.push(Candidate {
        region: Some(region(0.05, 0.45)),
        model_id: "narrow-reader".into(),
        family: "narrow-family".into(),
        view: "crop".into(),
        content: PageContent {
            blocks: vec![page.content.blocks[0].clone()],
            ..Default::default()
        },
        reliability: 0.8,
    });
    p.save(&page).unwrap();
    let p = migrate(&p);
    let preview = layout::preview(
        &p,
        Operation::Split {
            page: 1,
            axis: Axis::Vertical,
            at: 0.5,
        },
        &Control::default(),
    )
    .unwrap();
    let split = layout::apply(&p, &preview, &preview.token, &Control::default()).unwrap();
    let left = split.load(split.manifest.pages[0]).unwrap();
    let narrow = left
        .candidates
        .iter()
        .find(|candidate| candidate.model_id == "narrow-reader")
        .unwrap();
    assert!(narrow.region.as_ref().unwrap().right < 0.95);
    assert!(
        !split
            .load(split.manifest.pages[1])
            .unwrap()
            .candidates
            .iter()
            .any(|candidate| candidate.model_id == "narrow-reader")
    );
    let preview = layout::preview(
        &split,
        Operation::Reorder {
            pages: split.manifest.pages.iter().rev().copied().collect(),
        },
        &Control::default(),
    )
    .unwrap();
    let manifest = fs::read(split.root.join("project.json")).unwrap();
    let lock = split.root.join("model-lock.json");
    fs::write(&lock, b"concurrent selected lock bytes").unwrap();
    assert!(layout::apply(&split, &preview, &preview.token, &Control::default()).is_err());
    assert_eq!(fs::read(lock).unwrap(), b"concurrent selected lock bytes");
    assert_eq!(fs::read(split.root.join("project.json")).unwrap(), manifest);
}

#[test]
fn a_new_slot_cannot_adopt_an_untracked_overlay_created_after_preview() {
    let (temp, p) = fixture();
    let p = migrate(&p);
    let input = temp.path().join("insert.png");
    image::RgbImage::from_pixel(20, 20, image::Rgb([255, 255, 255]))
        .save(&input)
        .unwrap();
    let preview = layout::preview(
        &p,
        Operation::Insert {
            image: input,
            position: 0,
        },
        &Control::default(),
    )
    .unwrap();
    let slot = preview.created[0].slot;
    let overlay = p.root.join(format!("page-{slot:05}.edits.json"));
    fs::write(&overlay, b"unrelated untracked user overlay").unwrap();
    let original = fs::read(p.root.join("project.json")).unwrap();
    assert!(layout::apply(&p, &preview, &preview.token, &Control::default()).is_err());
    assert_eq!(
        fs::read(overlay).unwrap(),
        b"unrelated untracked user overlay"
    );
    assert_eq!(fs::read(p.root.join("project.json")).unwrap(), original);
    assert!(!p.image(slot).exists());
}
