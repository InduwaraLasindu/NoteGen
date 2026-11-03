use image::{DynamicImage, GrayImage, Luma};
use notegen::{
    profile_suggestions::{self as suggestions, SuggestOptions},
    profile_tools,
    storage::Control,
    types::*,
    writers,
};
use std::{fs, sync::atomic::Ordering};
fn glyph(kind: usize) -> DynamicImage {
    let mut image = GrayImage::from_pixel(100, 120, Luma([255]));
    for y in 20..100 {
        for x in 20..80 {
            if if kind == 0 {
                x < 30 || y > 88
            } else {
                ((x as i32 - 20) - (y as i32 - 20) * 3 / 4).abs() < 5
                    || (x as i32 - 80 + (y as i32 - 20) * 3 / 4).abs() < 5
            } {
                image[(x, y)] = Luma([0]);
            }
        }
    }
    DynamicImage::ImageLuma8(image)
}
fn fixture(root: &std::path::Path) -> (writers::Profile, writers::Profile) {
    let first = writers::create(root, "Writer one").unwrap();
    writers::add_example(
        root,
        &first.id,
        &glyph(0),
        BoxRect::full(),
        "r",
        true,
        "known glyph",
    )
    .unwrap();
    let second = writers::create(root, "Writer two").unwrap();
    writers::add_example(
        root,
        &second.id,
        &glyph(1),
        BoxRect::full(),
        "r",
        true,
        "known glyph",
    )
    .unwrap();
    (
        writers::read(root, &first.id).unwrap(),
        writers::read(root, &second.id).unwrap(),
    )
}
fn enabled() -> SuggestOptions {
    SuggestOptions {
        enabled: true,
        ..Default::default()
    }
}

#[test]
fn profile_matching_is_off_by_default_and_unknown_or_multiple_labels_do_not_scan_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let not_directory = dir.path().join("not-a-profile-folder");
    fs::write(&not_directory, b"not a directory").unwrap();
    let invalid = DynamicImage::new_rgb8(0, 0);
    let report = suggestions::suggest(
        &not_directory,
        &invalid,
        Some("r"),
        &SuggestOptions::default(),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(report.state, "disabled");
    assert!(report.suggestions.is_empty());
    assert!(!report.enabled);
    assert!(report.root_identity.is_empty());
    let report = suggestions::suggest(
        &not_directory,
        &invalid,
        None,
        &enabled(),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(report.state, "known_label_required");
    assert!(report.root_identity.is_empty());
    let report = suggestions::suggest(
        &not_directory,
        &invalid,
        Some("word"),
        &enabled(),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(report.state, "single_glyph_required");
    let defaults: SuggestOptions = serde_json::from_str("{}").unwrap();
    assert!(!defaults.enabled);
}
#[test]
fn local_ink_overlap_ranks_confirmed_same_glyph_without_claiming_identity_or_mutating_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = fixture(dir.path());
    let before = fs::read(dir.path().join(&first.id).join("profile.json")).unwrap();
    let report = suggestions::suggest(
        dir.path(),
        &glyph(0),
        Some("r"),
        &enabled(),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(report.state, "candidate_matches");
    assert_eq!(report.suggestions.len(), 2);
    assert_eq!(report.suggestions[0].writer_id, first.id);
    assert_eq!(report.suggestions[0].ink_overlap_similarity, 1.0);
    assert!(
        report.suggestions[0].ink_overlap_similarity
            > report
                .suggestions
                .iter()
                .find(|s| s.writer_id == second.id)
                .unwrap()
                .ink_overlap_similarity
    );
    assert!(report.note.contains("not identity confidence"));
    assert_eq!(report.suggestions[0].examples_compared, 1);
    assert_eq!(report.suggestions[0].example_hashes.len(), 1);
    assert_eq!(
        fs::read(dir.path().join(&first.id).join("profile.json")).unwrap(),
        before
    );
}
#[test]
fn explicit_confirmation_returns_writer_options_but_requires_exact_token_and_known_proposed_id() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = fixture(dir.path());
    let c = Control::default();
    let report = suggestions::suggest(dir.path(), &glyph(0), Some("r"), &enabled(), &c).unwrap();
    let original = Settings::default();
    assert!(suggestions::confirm(dir.path(), &report, &first.id, "wrong token", &c).is_err());
    assert!(
        suggestions::confirm(
            dir.path(),
            &report,
            "not-a-proposed-writer",
            &report.confirmation_token,
            &c
        )
        .is_err()
    );
    let selected = suggestions::confirm(
        dir.path(),
        &report,
        &first.id,
        &report.confirmation_token,
        &c,
    )
    .unwrap();
    assert_eq!(selected.id.as_deref(), Some(first.id.as_str()));
    assert!(selected.use_examples);
    assert_eq!(selected.root, dir.path().canonicalize().unwrap());
    assert!(original.writer.id.is_none());
    let mut tampered = report.clone();
    tampered.suggestions[0].name = "Edited proposal".into();
    assert!(
        suggestions::confirm(
            dir.path(),
            &tampered,
            &first.id,
            &report.confirmation_token,
            &c
        )
        .is_err()
    );
}
#[test]
fn quarantine_excludes_examples_and_stale_profile_revisions_cannot_be_confirmed() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = fixture(dir.path());
    let c = Control::default();
    let report = suggestions::suggest(dir.path(), &glyph(0), Some("r"), &enabled(), &c).unwrap();
    profile_tools::quarantine(
        dir.path(),
        &first.id,
        &first.samples[0].id,
        "Questionable example",
    )
    .unwrap();
    assert!(
        suggestions::confirm(
            dir.path(),
            &report,
            &first.id,
            &report.confirmation_token,
            &c
        )
        .is_err()
    );
    let current = suggestions::suggest(dir.path(), &glyph(0), Some("r"), &enabled(), &c).unwrap();
    assert!(!current.suggestions.iter().any(|s| s.writer_id == first.id));
    profile_tools::restore_example(dir.path(), &first.id, &first.samples[0].id).unwrap();
    let report = suggestions::suggest(dir.path(), &glyph(0), Some("r"), &enabled(), &c).unwrap();
    writers::update(dir.path(), &first.id, &first.name, "updated context").unwrap();
    assert!(
        suggestions::confirm(
            dir.path(),
            &report,
            &first.id,
            &report.confirmation_token,
            &c
        )
        .is_err()
    );
}
#[test]
fn word_labels_unknown_glyphs_and_empty_collections_do_not_become_matching_glyph_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let p = writers::create(dir.path(), "Only words").unwrap();
    writers::add_example(
        dir.path(),
        &p.id,
        &glyph(0),
        BoxRect::full(),
        "r",
        false,
        "a phrase-labelled sample",
    )
    .unwrap();
    for label in ["r", "z"] {
        let report = suggestions::suggest(
            dir.path(),
            &glyph(0),
            Some(label),
            &enabled(),
            &Control::default(),
        )
        .unwrap();
        assert_eq!(report.state, "no_matching_confirmed_glyphs");
        assert!(report.suggestions.is_empty());
        assert!(
            suggestions::confirm(
                dir.path(),
                &report,
                &p.id,
                &report.confirmation_token,
                &Control::default()
            )
            .is_err()
        );
    }
    let missing = dir.path().join("missing");
    assert!(
        suggestions::suggest(
            &missing,
            &glyph(0),
            Some("r"),
            &enabled(),
            &Control::default()
        )
        .unwrap()
        .suggestions
        .is_empty()
    );
    assert!(!missing.exists());
}
#[test]
fn changed_example_images_and_another_collection_cannot_reuse_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = fixture(dir.path());
    let c = Control::default();
    let report = suggestions::suggest(dir.path(), &glyph(0), Some("r"), &enabled(), &c).unwrap();
    let other = tempfile::tempdir().unwrap();
    assert!(
        suggestions::confirm(
            other.path(),
            &report,
            &first.id,
            &report.confirmation_token,
            &c
        )
        .is_err()
    );
    let image = dir.path().join(&first.id).join(&first.samples[0].image);
    DynamicImage::ImageLuma8(GrayImage::from_pixel(64, 80, Luma([255])))
        .save(image)
        .unwrap();
    assert!(
        suggestions::confirm(
            dir.path(),
            &report,
            &first.id,
            &report.confirmation_token,
            &c
        )
        .is_err()
    );
}
#[test]
fn invalid_image_options_and_cancellation_fail_without_applying_any_selection() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = fixture(dir.path());
    assert!(
        suggestions::suggest(
            dir.path(),
            &glyph(0),
            Some("r"),
            &SuggestOptions {
                enabled: true,
                max_results: 0
            },
            &Control::default()
        )
        .is_err()
    );
    assert!(
        suggestions::suggest(
            dir.path(),
            &DynamicImage::new_rgb8(0, 0),
            Some("r"),
            &enabled(),
            &Control::default()
        )
        .is_err()
    );
    assert!(
        suggestions::suggest(
            dir.path(),
            &DynamicImage::ImageLuma8(GrayImage::from_pixel(30, 30, Luma([255]))),
            Some("r"),
            &enabled(),
            &Control::default()
        )
        .is_err()
    );
    let report = suggestions::suggest(
        dir.path(),
        &glyph(0),
        Some("r"),
        &enabled(),
        &Control::default(),
    )
    .unwrap();
    let c = Control::default();
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(suggestions::suggest(dir.path(), &glyph(0), Some("r"), &enabled(), &c).is_err());
    assert!(
        suggestions::confirm(
            dir.path(),
            &report,
            &first.id,
            &report.confirmation_token,
            &c
        )
        .is_err()
    );
}
