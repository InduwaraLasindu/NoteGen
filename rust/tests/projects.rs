use notegen::{
    projects::{self, Filter, Metadata},
    storage::{Control, Manifest, Project, atomic_json, hash_file},
    types::*,
};
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    projects: Vec<Project>,
}
fn fixture(count: usize) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let mut projects = vec![];
    for index in 0..count {
        let p = Project::open(
            root.join(format!("project-{index}")),
            Manifest {
                schema: 2,
                input_sha256: format!("{index:064x}"),
                fingerprint: "test-settings".into(),
                pages: vec![1, 2],
                models: vec![],
                settings: Settings::default(),
            },
        )
        .unwrap();
        p.save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks: vec![Block::text(format!(
                    "Project {index} Sinhala සිංහල science 42"
                ))],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        image::RgbImage::from_pixel(2, 2, image::Rgb([255, 255, 255]))
            .save(p.image(1))
            .unwrap();
        projects.push(p);
    }
    Fixture {
        _temp: temp,
        root,
        projects,
    }
}
fn paths(f: &Fixture) -> Vec<PathBuf> {
    f.projects.iter().map(|p| p.root.clone()).collect()
}

#[test]
fn metadata_names_tags_folders_and_status_filters_are_persistent() {
    let f = fixture(2);
    let p = &f.projects[0];
    let metadata = Metadata {
        name: "Physics notes".into(),
        folder: "School".into(),
        tags: vec!["science".into(), "exam".into()],
        status: "paused".into(),
        ..Default::default()
    };
    projects::save_metadata(p, &metadata).unwrap();
    let read = projects::metadata(p).unwrap();
    assert_eq!(read.name, "Physics notes");
    assert!(!read.updated.is_empty());
    let filter = Filter {
        query: "PHYSICS".into(),
        tag: Some("exam".into()),
        folder: Some("School".into()),
        status: Some("paused".into()),
    };
    let entries = projects::catalog(&paths(&f), &filter).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].metadata.name, "Physics notes");
    assert_eq!(entries[0].completed, 1);
    assert_eq!(entries[0].pending_review, 1);
    assert!(entries[0].error.as_ref().unwrap().contains("Page 2"));
}
#[test]
fn invalid_metadata_never_replaces_current_values() {
    let f = fixture(1);
    let p = &f.projects[0];
    let mut m = projects::metadata(p).unwrap();
    m.name = "Keep me".into();
    projects::save_metadata(p, &m).unwrap();
    m.status = "unknown-status".into();
    assert!(projects::save_metadata(p, &m).is_err());
    m.status = "active".into();
    m.tags = vec!["duplicate".into(), "duplicate".into()];
    assert!(projects::save_metadata(p, &m).is_err());
    assert_eq!(projects::metadata(p).unwrap().name, "Keep me");
}
#[test]
fn catalog_is_explicit_deduplicated_and_reports_missing_projects() {
    let f = fixture(2);
    let roots = vec![
        f.projects[0].root.clone(),
        f.projects[0].root.clone(),
        f.root.join("missing"),
    ];
    let entries = projects::catalog(&roots, &Filter::default()).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .any(|e| e.error.is_some() && e.completed == 0)
    );
    assert!(!entries.iter().any(|e| e.root == f.projects[1].root));
}
#[test]
fn catalog_reports_reviewed_counts_and_filters_without_mutating_recognition() {
    let f = fixture(1);
    let p = &f.projects[0];
    let original = std::fs::read(p.page_path(1)).unwrap();
    let mut page = p.load(1).unwrap();
    page.reviewed = true;
    p.save(&page).unwrap();
    let entries = projects::catalog(&paths(&f), &Filter::default()).unwrap();
    assert_eq!(entries[0].completed, 1);
    assert_eq!(entries[0].pending_review, 0);
    assert_eq!(std::fs::read(p.page_path(1)).unwrap(), original);
    let no_match = Filter {
        query: "Unknown subject".into(),
        ..Default::default()
    };
    assert!(projects::catalog(&paths(&f), &no_match).unwrap().is_empty());
}
#[test]
fn search_returns_source_page_block_geometry_and_unicode() {
    let f = fixture(2);
    let p = &f.projects[0];
    let mut page = p.load(1).unwrap();
    page.content.blocks[0].bbox = Some(BoxRect {
        left: 0.1,
        top: 0.2,
        right: 0.8,
        bottom: 0.9,
    });
    p.save(&page).unwrap();
    let report = projects::search(&paths(&f), "සිංහල", 10).unwrap();
    assert_eq!(report.hits.len(), 2);
    let hit = &report.hits[0];
    assert_eq!(hit.page, 1);
    assert_eq!(hit.block, 0);
    assert_eq!(hit.region, page.content.blocks[0].bbox);
    assert!(hit.image.ends_with("page-00001.png"));
    assert!(hit.excerpt.contains("සිංහල"));
    assert_eq!(report.warnings.len(), 2);
}
#[test]
fn search_is_bounded_case_insensitive_and_only_indexes_output() {
    let f = fixture(1);
    let p = &f.projects[0];
    let mut page = p.load(1).unwrap();
    page.content
        .blocks
        .push(Block::text("SCIENCE second result"));
    page.candidates.push(Candidate {
        region: None,
        model_id: "evidence".into(),
        family: "evidence".into(),
        view: "full".into(),
        content: PageContent {
            blocks: vec![Block::text("secret alternative")],
            ..Default::default()
        },
        reliability: 1.0,
    });
    p.save(&page).unwrap();
    let report = projects::search(&paths(&f), "science", 1).unwrap();
    assert_eq!(report.hits.len(), 1);
    assert!(report.truncated);
    assert!(
        projects::search(&paths(&f), "secret alternative", 10)
            .unwrap()
            .hits
            .is_empty()
    );
    assert!(projects::search(&paths(&f), "", 10).is_err());
    assert!(projects::search(&paths(&f), "science", 0).is_err());
    assert!(projects::search(&paths(&f), "science", 10001).is_err());
}
#[test]
fn search_includes_table_cells_and_figure_context_with_long_text_excerpts() {
    let f = fixture(1);
    let p = &f.projects[0];
    let mut page = p.load(1).unwrap();
    page.content.blocks = vec![
        Block {
            kind: "table".into(),
            rows: vec![vec!["cell needle".into()]],
            ..Default::default()
        },
        Block {
            kind: "figure".into(),
            bbox: Some(BoxRect::full()),
            description: format!("{} NEEDLE {}", "あ".repeat(400), "字".repeat(400)),
            ..Default::default()
        },
    ];
    p.save(&page).unwrap();
    let report = projects::search(&paths(&f), "needle", 10).unwrap();
    assert_eq!(report.hits.len(), 2);
    assert!(report.hits[1].excerpt.contains("NEEDLE"));
    assert!(report.hits[1].excerpt.chars().count() <= 282);
}
#[test]
fn duplicates_use_content_hashes_across_filenames_and_skip_placeholders() {
    let f = fixture(3);
    let mut p = f.projects[1].clone();
    p.manifest.input_sha256 = f.projects[0].manifest.input_sha256.clone();
    atomic_json(&p.root.join("project.json"), &p.manifest).unwrap();
    let groups = projects::duplicates(&paths(&f)).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].projects.len(), 2);
    assert_eq!(groups[0].input_sha256, f.projects[0].manifest.input_sha256);
    p.manifest.input_sha256 = "synthetic".into();
    atomic_json(&p.root.join("project.json"), &p.manifest).unwrap();
    assert!(projects::duplicates(&paths(&f)).unwrap().is_empty());
}
#[test]
fn recovery_distinguishes_pending_missing_and_invalid_stages() {
    let f = fixture(1);
    let p = &f.projects[0];
    let report = projects::recovery(p).unwrap();
    assert_eq!(report.completed, 1);
    assert_eq!(report.pending_review, 1);
    assert_eq!(report.pages[0].state, "pending_review");
    assert_eq!(report.pages[1].state, "render_pending");
    assert!(report.input_state.contains("not recorded"));
    image::RgbImage::from_pixel(2, 2, image::Rgb([255, 255, 255]))
        .save(p.image(2))
        .unwrap();
    assert_eq!(
        projects::recovery(p).unwrap().pages[1].state,
        "recognition_pending"
    );
    std::fs::write(p.page_path(2), b"invalid JSON").unwrap();
    let report = projects::recovery(p).unwrap();
    assert_eq!(report.pages[1].state, "invalid_checkpoint");
    assert!(report.pages[1].error.is_some());
    assert!(!report.pages[1].reusable);
    std::fs::remove_file(p.image(1)).unwrap();
    let report = projects::recovery(p).unwrap();
    assert_eq!(report.pages[0].state, "missing_source");
    assert!(report.pages[0].reusable);
}
#[test]
fn recovery_verifies_a_recorded_source_without_guessing_its_location() {
    let f = fixture(1);
    let mut p = f.projects[0].clone();
    let input = f.root.join("original.pdf");
    std::fs::write(&input, b"source bytes").unwrap();
    p.manifest.input_sha256 = hash_file(&input, &Control::default()).unwrap();
    atomic_json(&p.root.join("project.json"), &p.manifest).unwrap();
    let mut m = projects::metadata(&p).unwrap();
    m.source = Some(input.clone());
    projects::save_metadata(&p, &m).unwrap();
    assert!(
        projects::recovery(&p)
            .unwrap()
            .input_state
            .contains("matches")
    );
    std::fs::write(&input, b"changed").unwrap();
    assert!(
        projects::recovery(&p)
            .unwrap()
            .input_state
            .contains("changed")
    );
    std::fs::remove_file(input).unwrap();
    assert!(
        projects::recovery(&p)
            .unwrap()
            .input_state
            .contains("missing")
    );
}
#[test]
fn attached_references_are_separate_from_evidence_and_detect_changes() {
    let f = fixture(1);
    let p = &f.projects[0];
    let input = f.root.join("diagram.png");
    std::fs::write(&input, b"reference bytes").unwrap();
    let original = std::fs::read(p.page_path(1)).unwrap();
    let reference = projects::attach_reference(
        p,
        &input,
        "Related diagram",
        "background",
        &Control::default(),
    )
    .unwrap();
    assert_eq!(reference.role, "background");
    assert_eq!(reference.bytes, 15);
    assert!(
        projects::attach_reference(p, &input, "Duplicate", "background", &Control::default())
            .is_err()
    );
    assert_eq!(
        projects::recovery(p).unwrap().references[0].state,
        "verified"
    );
    assert_eq!(std::fs::read(p.page_path(1)).unwrap(), original);
    std::fs::write(&input, b"different reference").unwrap();
    assert_eq!(
        projects::recovery(p).unwrap().references[0].state,
        "changed"
    );
    std::fs::remove_file(input).unwrap();
    assert_eq!(
        projects::recovery(p).unwrap().references[0].state,
        "missing"
    );
}
#[test]
fn references_reject_directories_invalid_roles_and_cancelled_hashing() {
    let f = fixture(1);
    let p = &f.projects[0];
    assert!(
        projects::attach_reference(p, &f.root, "Directory", "reference", &Control::default())
            .is_err()
    );
    let input = f.root.join("ref.txt");
    std::fs::write(&input, b"reference").unwrap();
    assert!(
        projects::attach_reference(
            p,
            &input,
            "Ref",
            "transcription evidence",
            &Control::default()
        )
        .is_err()
    );
    let control = Control::default();
    control
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(projects::attach_reference(p, &input, "Ref", "reference", &control).is_err());
    assert!(projects::metadata(p).unwrap().references.is_empty());
}
#[test]
fn templates_roundtrip_policy_and_protect_existing_outputs() {
    let f = fixture(1);
    let output = f.root.join("school-template.json");
    let mut s = Settings {
        subject: "Physics".into(),
        font: "Cambria".into(),
        font_size: 12.0,
        ..Default::default()
    };
    s.assistance.glossary = "momentum".into();
    s.writer.id = Some("writer-123".into());
    s.writer.root = f.root.join("private-writers");
    s.models_dir = f.root.join("private-models");
    s.shared_model_store = f.root.join("private-user-model-store");
    s.project_lock = Some(f.root.join("private-project/model-lock.json"));
    s.assistance.sources.push(notegen::assistance::WebSource {
        title: "Private context".into(),
        excerpt: "Not portable".into(),
        ..Default::default()
    });
    projects::save_template(&output, "School notes", &s).unwrap();
    let template = projects::load_template(&output).unwrap();
    assert_eq!(template.name, "School notes");
    assert_eq!(template.settings.subject, "Physics");
    assert_eq!(template.settings.font, "Cambria");
    assert_eq!(template.settings.assistance.glossary, "momentum");
    assert!(template.settings.writer.id.is_none());
    assert!(template.settings.assistance.sources.is_empty());
    assert_ne!(template.settings.models_dir, s.models_dir);
    assert_eq!(
        template.settings.shared_model_store,
        std::path::PathBuf::from("models/shared")
    );
    assert!(template.settings.project_lock.is_none());
    let bytes = std::fs::read(&output).unwrap();
    let json = String::from_utf8(bytes.clone()).unwrap();
    for private in [
        "private-user-model-store",
        "private-project",
        "private-writers",
        "private-models",
    ] {
        assert!(!json.contains(private), "{private}");
    }
    assert!(projects::save_template(&output, "Overwrite", &s).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
}
#[test]
fn template_schema_settings_and_name_are_validated() {
    let f = fixture(1);
    let output = f.root.join("bad.json");
    let s = Settings {
        dpi: 1,
        ..Default::default()
    };
    assert!(projects::save_template(&output, "Bad settings", &s).is_err());
    assert!(!output.exists());
    assert!(projects::save_template(&output, "", &Settings::default()).is_err());
    std::fs::write(&output, br#"{"schema":99,"name":"Future","settings":{}}"#).unwrap();
    assert!(projects::load_template(&output).is_err());
}
