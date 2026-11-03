use notegen::{
    planning,
    project_lock::{self, Origin, Preferences},
    storage::{Control, Manifest, Project},
    types::*,
};
use std::{fs, path::Path};
fn fixture(root: &Path) -> (Project, planning::Model, Settings) {
    let native = root.join("native");
    fs::create_dir(&native).unwrap();
    fs::write(native.join("llama-server.exe"), b"MZexact runtime fixture").unwrap();
    fs::write(native.join("ggml.dll"), b"native library fixture").unwrap();
    let settings = Settings {
        server: native.join("llama-server.exe"),
        backend: Backend::Cpu,
        shared_model_store: root.join("shared-store"),
        models_dir: root.join("global-models"),
        ..Default::default()
    };
    let project = Project::open(
        root.join("project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1],
            models: vec!["glm-ocr".into()],
            settings: settings.clone(),
        },
    )
    .unwrap();
    let mut model = planning::catalog()
        .into_iter()
        .find(|m| m.id == "glm-ocr")
        .unwrap();
    model.weights = root.join("weights.gguf");
    model.projector = root.join("projector.gguf");
    model.sha256.clear();
    model.hub = None;
    model.revision = "local-first".into();
    fs::write(&model.weights, b"GGUForiginal exact weights").unwrap();
    fs::write(&model.projector, b"GGUForiginal exact projector").unwrap();
    (project, model, settings)
}
#[test]
fn project_resolves_exact_old_bytes_without_global_registry_changes() {
    let root = tempfile::tempdir().unwrap();
    let (project, model, settings) = fixture(root.path());
    let lock = project_lock::capture(
        &project,
        std::slice::from_ref(&model),
        &settings,
        Origin::RecordedConversion,
        &Control::default(),
    )
    .unwrap();
    let receipt = project_lock::write(&project, &lock, None, &Control::default()).unwrap();
    fs::create_dir(&settings.models_dir).unwrap();
    let registration = settings.models_dir.join("glm-ocr.notegen.json");
    fs::write(
        &registration,
        b"different global registration remains untouched",
    )
    .unwrap();
    fs::write(&model.weights, b"GGUFnew global weights").unwrap();
    let selected = Settings {
        project_lock: Some(receipt.path),
        ..settings
    };
    let resolved = project_lock::resolve_settings(&selected, &Control::default())
        .unwrap()
        .unwrap();
    assert_eq!(resolved.models[0].revision, "local-first");
    assert_eq!(
        fs::read(&resolved.models[0].weights).unwrap(),
        b"GGUForiginal exact weights"
    );
    assert!(!resolved.settings.automatic_models);
    assert_eq!(resolved.settings.backend, Backend::Cpu);
    assert_ne!(resolved.settings.server, selected.server);
    assert_eq!(
        fs::read(registration).unwrap(),
        b"different global registration remains untouched"
    );
    assert_eq!(resolved.input_sha256, project.manifest.input_sha256);
}
#[test]
fn missing_custom_dependencies_restore_by_exact_file_without_substitution_or_activation() {
    let root = tempfile::tempdir().unwrap();
    let (project, model, settings) = fixture(root.path());
    let lock = project_lock::capture(
        &project,
        std::slice::from_ref(&model),
        &settings,
        Origin::FutureSelection,
        &Control::default(),
    )
    .unwrap();
    let receipt = project_lock::write(&project, &lock, None, &Control::default()).unwrap();
    let fresh_store = Settings {
        shared_model_store: root.path().join("new-pc-store"),
        project_lock: Some(receipt.path.clone()),
        ..settings
    };
    let first =
        project_lock::preview_restore(&receipt.path, &fresh_store, &Control::default()).unwrap();
    assert_eq!(first.missing.len(), 4);
    assert!(first.runtime_materialization_needed);
    assert!(first.additional_disk_bytes > first.missing_object_bytes);
    let report = project_lock::restore(
        &first,
        &first.token,
        &fresh_store,
        &Control::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(report.restored_objects, 2);
    assert_eq!(report.remaining_manual.len(), 2);
    assert!(project_lock::resolve_settings(&fresh_store, &Control::default()).is_err());
    loop {
        let preview =
            project_lock::preview_restore(&receipt.path, &fresh_store, &Control::default())
                .unwrap();
        if preview.missing.is_empty() {
            break;
        }
        let missing = &preview.missing[0];
        let source = match missing.role.as_str() {
            "weights" => model.weights.clone(),
            "projector" => model.projector.clone(),
            name => root.path().join("native").join(name),
        };
        assert!(
            project_lock::import_missing(
                &preview,
                &preview.token,
                0,
                &root.path().join("not-a-file"),
                &fresh_store,
                &Control::default()
            )
            .is_err()
        );
        project_lock::import_missing(
            &preview,
            &preview.token,
            0,
            &source,
            &fresh_store,
            &Control::default(),
        )
        .unwrap();
    }
    let resolved = project_lock::resolve_settings(&fresh_store, &Control::default())
        .unwrap()
        .unwrap();
    assert_eq!(resolved.models[0].revision, "local-first");
    assert!(resolved.settings.server.is_file());
    assert!(!fresh_store.models_dir.exists());
    assert!(model.weights.exists() && model.projector.exists());
}
#[test]
fn preferences_are_project_scoped_explicit_and_preserve_prior_lock_for_undo() {
    let root = tempfile::tempdir().unwrap();
    let (project, model, settings) = fixture(root.path());
    let lock = project_lock::capture(
        &project,
        &[model],
        &settings,
        Origin::FutureSelection,
        &Control::default(),
    )
    .unwrap();
    let initial = project_lock::write(&project, &lock, None, &Control::default()).unwrap();
    let prefs = Preferences {
        channel: UpdateChannel::Preview,
        prefer_locked: false,
        defer_until: None,
    };
    let changed =
        project_lock::set_preferences(&project, &lock.id, prefs, &Control::default()).unwrap();
    assert_ne!(initial.lock_id, changed.lock_id);
    let previous = changed.previous_copy.unwrap();
    assert_eq!(
        project_lock::read(&previous, &Control::default())
            .unwrap()
            .id,
        lock.id
    );
    assert!(
        project_lock::set_preferences(
            &project,
            &lock.id,
            Preferences::default(),
            &Control::default()
        )
        .is_err()
    );
    let restored =
        project_lock::write(&project, &lock, Some(&changed.lock_id), &Control::default()).unwrap();
    assert_eq!(restored.lock_id, lock.id);
    let now = chrono::Utc::now();
    assert!(!project_lock::update_allowed(
        &Preferences::default(),
        Some(false),
        false,
        now
    ));
    let deferred = Preferences {
        channel: UpdateChannel::Stable,
        defer_until: Some(now + chrono::Duration::days(1)),
        prefer_locked: false,
    };
    assert!(!project_lock::update_allowed(
        &deferred,
        Some(false),
        false,
        now
    ));
    assert!(project_lock::update_allowed(
        &deferred,
        Some(false),
        true,
        now
    ));
    assert!(!project_lock::update_allowed(
        &deferred,
        Some(true),
        true,
        now
    ));
    assert!(!project_lock::update_allowed(&deferred, None, false, now));
    assert!(project_lock::update_allowed(&deferred, None, true, now));
}
#[test]
fn stale_restore_or_wrong_source_never_changes_project_or_global_selection() {
    let root = tempfile::tempdir().unwrap();
    let (project, model, settings) = fixture(root.path());
    let lock = project_lock::capture(
        &project,
        &[model],
        &settings,
        Origin::FutureSelection,
        &Control::default(),
    )
    .unwrap();
    let receipt = project_lock::write(&project, &lock, None, &Control::default()).unwrap();
    let plan =
        project_lock::preview_restore(&receipt.path, &settings, &Control::default()).unwrap();
    project_lock::set_preferences(
        &project,
        &lock.id,
        Preferences {
            channel: UpdateChannel::Preview,
            ..Default::default()
        },
        &Control::default(),
    )
    .unwrap();
    assert!(
        project_lock::restore(
            &plan,
            &plan.token,
            &settings,
            &Control::default(),
            &Default::default()
        )
        .is_err()
    );
    let mut other = project.clone();
    other.manifest.input_sha256 = "c".repeat(64);
    let selected = Settings {
        project_lock: Some(receipt.path),
        ..settings
    };
    assert!(project_lock::resolve_for_project(&other, &selected, &Control::default()).is_err());
    assert!(project_lock::write(&other, &lock, None, &Control::default()).is_err());
}
#[test]
fn fast_mode_ignores_broken_ai_locks_and_tampered_metadata_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (project, model, settings) = fixture(root.path());
    let selected = Settings {
        preset: Preset::Fast,
        project_lock: Some(root.path().join("missing-lock.json")),
        ..settings.clone()
    };
    assert!(
        project_lock::resolve_settings(&selected, &Control::default())
            .unwrap()
            .is_none()
    );
    let mut lock = project_lock::capture(
        &project,
        &[model],
        &settings,
        Origin::FutureSelection,
        &Control::default(),
    )
    .unwrap();
    lock.models[0].weights.sha256 = "d".repeat(64);
    assert!(project_lock::validate(&lock).is_err());
}

#[test]
fn missing_runtime_is_restored_from_verified_cached_publisher_archive_without_global_activation() {
    use notegen::{
        model_store::{Kind, Store},
        project_lock::RuntimeSource,
        setup::Download,
    };
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let (project, model, settings) = fixture(root.path());
    let archive = root.path().join("runtime.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for name in ["llama-server.exe", "ggml.dll"] {
        zip.start_file(format!("bin/{name}"), options).unwrap();
        zip.write_all(&fs::read(root.path().join("native").join(name)).unwrap())
            .unwrap();
    }
    zip.start_file("NOTICE", options).unwrap();
    zip.write_all(&vec![b'x'; 1500]).unwrap();
    zip.finish().unwrap();
    let source=RuntimeSource{build:"b123".into(),backend:Backend::Cpu,assets:vec![Download{file:"llama-b123-bin-win-cpu-x64.zip".into(),bytes:fs::metadata(&archive).unwrap().len(),sha256:notegen::storage::hash_file(&archive,&Control::default()).unwrap(),url:"https://github.com/ggml-org/llama.cpp/releases/download/b123/llama-b123-bin-win-cpu-x64.zip".into()}]};
    fs::write(
        root.path().join("native/notegen-runtime-source.json"),
        serde_json::to_vec(&source).unwrap(),
    )
    .unwrap();
    let lock = project_lock::capture(
        &project,
        &[model],
        &settings,
        Origin::FutureSelection,
        &Control::default(),
    )
    .unwrap();
    let receipt = project_lock::write(&project, &lock, None, &Control::default()).unwrap();
    let selected = Settings {
        shared_model_store: root.path().join("fresh-archive-store"),
        server: root.path().join("absent-native/llama-server.exe"),
        project_lock: Some(receipt.path.clone()),
        ..settings
    };
    let store = Store::open(&selected.shared_model_store).unwrap();
    store
        .import_file(&archive, Kind::Archive, &Control::default())
        .unwrap();
    let preview =
        project_lock::preview_restore(&receipt.path, &selected, &Control::default()).unwrap();
    assert_eq!(preview.download_bytes, 0);
    let report = project_lock::restore(
        &preview,
        &preview.token,
        &selected,
        &Control::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(report.restored_objects, 2);
    assert_eq!(report.remaining_manual.len(), 2);
    assert!(report.remaining_manual.iter().all(|m| m.kind == Kind::Gguf));
    assert!(!selected.models_dir.exists());
    assert!(archive.exists());
}
