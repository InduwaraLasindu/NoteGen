use notegen::{
    pipeline,
    project_reuse::{self, Choice},
    redactions, review,
    storage::{Control, Manifest, Project},
    types::*,
};
use std::fs;
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Settings, Project) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("original.png");
    image::RgbImage::from_pixel(120, 80, image::Rgb([255, 255, 255]))
        .save(&source)
        .unwrap();
    let settings = Settings {
        preset: Preset::Fast,
        backend: Backend::Cpu,
        dpi: 150,
        image_edge: 1024,
        context: 8192,
        cpu_threads: 2,
        models_dir: temp.path().join("isolated-models"),
        ..Default::default()
    };
    let identity = pipeline::planned_identity(&source, &settings, &Control::default()).unwrap();
    assert!(
        identity.ocr_executable_verified,
        "Native test requires the installed Tesseract executable"
    );
    let project = Project::open(
        temp.path().join("private-project"),
        Manifest {
            schema: 2,
            input_sha256: identity.input_sha256,
            fingerprint: identity.fingerprint,
            pages: vec![1],
            models: vec![],
            settings: settings.clone(),
        },
    )
    .unwrap();
    fs::copy(&source, project.image(1)).unwrap();
    let mut page = Page {
        page: 1,
        image: "page-00001.png".into(),
        content: PageContent {
            blocks: vec![Block {
                bbox: Some(BoxRect::full()),
                ..Block::text("Invoice 1250")
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    page.content.blocks[0].geometry = Some(notegen::geometry::BlockGeometry {
        source: Some(redactions::source_identity(&project, &page, &Control::default()).unwrap()),
        ..Default::default()
    });
    project.save(&page).unwrap();
    fs::write(
        project.root.join("manual-private.txt"),
        b"private original evidence",
    )
    .unwrap();
    fs::write(
        project.root.join("project.lock"),
        b"opaque private control bytes",
    )
    .unwrap();
    (temp, source, settings, project)
}
#[test]
fn renamed_duplicate_reuses_exact_project_or_preserves_private_history_in_a_new_revision() {
    let (temp, source, settings, project) = fixture();
    let before = project.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[0].text = "Confirmed 1250".into();
    review::commit(&project, &before, &after, "Confirm invoice value").unwrap();
    let original = fs::read(project.page_path(1)).unwrap();
    let history = fs::read(project.root.join("review-history.json")).unwrap();
    let renamed = temp.path().join("different-name.png");
    fs::copy(&source, &renamed).unwrap();
    let preview = project_reuse::preview(
        &renamed,
        &settings,
        std::slice::from_ref(&project.root),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(preview.candidates.len(), 1);
    assert!(
        preview.candidates[0].compatible,
        "{:?}",
        preview.candidates[0].reasons
    );
    let reused = project_reuse::choose(
        &preview,
        &preview.token,
        0,
        Choice::Reuse,
        None,
        &Control::default(),
    )
    .unwrap();
    assert_eq!(reused.root, project.root.canonicalize().unwrap());
    assert_eq!(
        reused.load(1).unwrap().content.blocks[0].text,
        "Confirmed 1250"
    );
    let revision = temp.path().join("separate-revision");
    let fork = project_reuse::choose(
        &preview,
        &preview.token,
        0,
        Choice::SeparateRevision,
        Some(&revision),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(fs::read(fork.page_path(1)).unwrap(), original);
    assert_eq!(
        fs::read(fork.root.join("review-history.json")).unwrap(),
        history
    );
    assert_eq!(
        fs::read(fork.root.join("manual-private.txt")).unwrap(),
        b"private original evidence"
    );
    assert_eq!(
        fs::read(fork.root.join("project.lock")).unwrap(),
        b"opaque private control bytes"
    );
    assert_eq!(
        fs::read(fork.image(1)).unwrap(),
        fs::read(project.image(1)).unwrap()
    );
    assert!(
        project_reuse::compare(&project, &fork, &Control::default())
            .unwrap()
            .differences
            .is_empty()
    );
    let before = fork.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[0].text = "Separate revision 1550".into();
    review::commit(&fork, &before, &after, "Revise only the fork").unwrap();
    let comparison = project_reuse::compare(&project, &fork, &Control::default()).unwrap();
    assert!(comparison.same_input && comparison.same_recognition);
    assert!(comparison.differences.iter().any(|d| d.field == "text"));
    assert_eq!(
        project.load(1).unwrap().content.blocks[0].text,
        "Confirmed 1250"
    );
    let prior_origins = fs::read_dir(&fork.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("revision-origin-")
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_owned(),
                fs::read(&path).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(prior_origins.len(), 1);
    let nested_preview = project_reuse::preview(
        &renamed,
        &settings,
        std::slice::from_ref(&fork.root),
        &Control::default(),
    )
    .unwrap();
    let nested = project_reuse::choose(
        &nested_preview,
        &nested_preview.token,
        0,
        Choice::SeparateRevision,
        Some(&temp.path().join("nested-revision")),
        &Control::default(),
    )
    .unwrap();
    for (name, bytes) in prior_origins {
        assert_eq!(fs::read(nested.root.join(name)).unwrap(), bytes);
    }
    assert_eq!(
        fs::read_dir(&nested.root)
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("revision-origin-"))
            .count(),
        2
    );
    assert!(
        project_reuse::choose(
            &preview,
            &preview.token,
            0,
            Choice::SeparateRevision,
            Some(&revision),
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(
        fs::read(project.root.join("review-history.json")).unwrap(),
        history
    );
}
#[test]
fn reuse_rejects_different_recognition_settings_and_a_changed_confirmed_duplicate() {
    let (_temp, source, settings, project) = fixture();
    let mut changed = settings.clone();
    changed.deskew = !settings.deskew;
    let different = project_reuse::preview(
        &source,
        &changed,
        std::slice::from_ref(&project.root),
        &Control::default(),
    )
    .unwrap();
    assert!(!different.candidates[0].compatible);
    assert!(
        project_reuse::choose(
            &different,
            &different.token,
            0,
            Choice::Reuse,
            None,
            &Control::default()
        )
        .is_err()
    );
    let preview = project_reuse::preview(
        &source,
        &settings,
        std::slice::from_ref(&project.root),
        &Control::default(),
    )
    .unwrap();
    let before = project.load(1).unwrap();
    let mut after = before.clone();
    after.content.blocks[0].text = "Concurrent corrected 999".into();
    review::commit(&project, &before, &after, "Later correction").unwrap();
    assert!(
        project_reuse::choose(
            &preview,
            &preview.token,
            0,
            Choice::Reuse,
            None,
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(
        project.load(1).unwrap().content.blocks[0].text,
        "Concurrent corrected 999"
    );
}

#[test]
fn a_mask_on_either_side_withholds_both_sides_of_a_duplicate_comparison() {
    let (temp, source, settings, project) = fixture();
    let preview = project_reuse::preview(
        &source,
        &settings,
        std::slice::from_ref(&project.root),
        &Control::default(),
    )
    .unwrap();
    let fork = project_reuse::choose(
        &preview,
        &preview.token,
        0,
        Choice::SeparateRevision,
        Some(&temp.path().join("covered-revision")),
        &Control::default(),
    )
    .unwrap();
    let before = fork.load(1).unwrap();
    let confirmation = redactions::preview_confirmation(
        &fork,
        &before,
        &[redactions::MaskSpec {
            region: BoxRect::full(),
            reason: redactions::Reason::PersonalInformation,
        }],
        &Control::default(),
    )
    .unwrap();
    let mut after = before.clone();
    after.confirmed_redactions = redactions::confirm(
        &fork,
        &before,
        &confirmation,
        &confirmation.token,
        &[0],
        &Control::default(),
    )
    .unwrap();
    review::commit(&fork, &before, &after, "Confirm comparison cover").unwrap();
    for (a, b) in [(&project, &fork), (&fork, &project)] {
        let report = project_reuse::compare(a, b, &Control::default()).unwrap();
        assert!(report.content_withheld);
        assert!(report.differences.is_empty());
        assert!(report.note.contains("metadata-only"));
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("Invoice 1250")
        );
    }
}
