use notegen::{
    models, planning, setup,
    storage::{self, Activity, Control},
    types::*,
};
use std::{fs, path::Path, sync::atomic::Ordering};

fn settings(root: &Path) -> Settings {
    Settings {
        models_dir: root.join("registry"),
        shared_model_store: root.join("shared-store"),
        server: root.join("runtime/missing-native-server"),
        backend: Backend::Cpu,
        ..Default::default()
    }
}
fn reader(root: &Path, label: &str) -> planning::Model {
    fs::create_dir_all(root).unwrap();
    let mut model = planning::catalog()
        .into_iter()
        .find(|model| model.id == "glm-ocr")
        .unwrap();
    model.weights = root.join(format!("{label}-weights.gguf"));
    model.projector = root.join(format!("{label}-projector.gguf"));
    model.revision = format!("local-{label}");
    model.sha256.clear();
    fs::write(&model.weights, format!("GGUFsynthetic-{label}-weights")).unwrap();
    fs::write(&model.projector, format!("GGUFsynthetic-{label}-projector")).unwrap();
    model
}
#[test]
fn persisted_candidate_uses_immutable_objects_and_never_changes_working_registration() {
    let root = tempfile::tempdir().unwrap();
    let s = settings(root.path());
    models::save_custom(
        &s.models_dir,
        reader(root.path(), "working"),
        &Control::default(),
    )
    .unwrap();
    let active = s.models_dir.join("glm-ocr.notegen.json");
    let original = fs::read(&active).unwrap();
    let source = reader(root.path(), "candidate");
    let source_weights = fs::read(&source.weights).unwrap();
    let prepared = models::prepare_custom(&s, source.clone(), &Control::default()).unwrap();
    assert_eq!(fs::read(&active).unwrap(), original);
    assert!(!s.models_dir.join("glm-ocr.previous.json").exists());
    assert!(prepared.summary().status.contains("not active"));
    assert!(
        prepared
            .model()
            .weights
            .starts_with(s.shared_model_store.canonicalize().unwrap())
    );
    assert_ne!(prepared.model().weights, source.weights);
    fs::write(&source.weights, b"GGUFsource changed independently").unwrap();
    let reopened =
        models::read_prepared(&prepared.summary().record, &s, &Control::default()).unwrap();
    assert_eq!(fs::read(&reopened.model().weights).unwrap(), source_weights);
    assert_eq!(fs::read(&active).unwrap(), original);
}
#[test]
fn failed_native_preflight_preserves_active_previous_settings_sources_and_pending_candidate() {
    let root = tempfile::tempdir().unwrap();
    let s = settings(root.path());
    models::save_custom(
        &s.models_dir,
        reader(root.path(), "old"),
        &Control::default(),
    )
    .unwrap();
    models::save_custom(
        &s.models_dir,
        reader(root.path(), "working"),
        &Control::default(),
    )
    .unwrap();
    let active = s.models_dir.join("glm-ocr.notegen.json");
    let previous = s.models_dir.join("glm-ocr.previous.json");
    let old_active = fs::read(&active).unwrap();
    let old_previous = fs::read(&previous).unwrap();
    let config = root.path().join("chosen-settings.json");
    storage::protected_json(&config, &s).unwrap();
    let old_config = fs::read(&config).unwrap();
    let candidate = reader(root.path(), "candidate");
    let original_weights = fs::read(&candidate.weights).unwrap();
    let original_projector = fs::read(&candidate.projector).unwrap();
    let prepared = models::prepare_custom(&s, candidate.clone(), &Control::default()).unwrap();
    assert!(
        models::activate_prepared(&prepared, &s, &Control::default(), &Activity::default())
            .is_err()
    );
    assert_eq!(fs::read(active).unwrap(), old_active);
    assert_eq!(fs::read(previous).unwrap(), old_previous);
    assert_eq!(fs::read(config).unwrap(), old_config);
    assert_eq!(fs::read(candidate.weights).unwrap(), original_weights);
    assert_eq!(fs::read(candidate.projector).unwrap(), original_projector);
    assert!(models::read_prepared(&prepared.summary().record, &s, &Control::default()).is_ok());
    assert!(
        !s.models_dir
            .join(".notegen-activation")
            .read_dir()
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("transaction-"))
    );
}
#[test]
fn cancelled_first_install_and_external_override_never_publish_a_model_pointer() {
    let root = tempfile::tempdir().unwrap();
    let mut s = settings(root.path());
    let prepared =
        models::prepare_custom(&s, reader(root.path(), "first"), &Control::default()).unwrap();
    let cancelled = Control::default();
    cancelled.cancelled.store(true, Ordering::Release);
    let error =
        models::activate_prepared(&prepared, &s, &cancelled, &Activity::default()).unwrap_err();
    assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
    assert!(!s.models_dir.join("glm-ocr.notegen.json").exists());
    s.external_server = true;
    let error = models::activate_prepared(&prepared, &s, &Control::default(), &Activity::default())
        .unwrap_err();
    assert!(error.to_string().contains("remains pending"));
    assert!(!s.models_dir.join("glm-ocr.notegen.json").exists());
}
#[test]
fn edited_or_moved_candidate_record_is_not_an_activation_proof() {
    let root = tempfile::tempdir().unwrap();
    let s = settings(root.path());
    let candidate =
        models::prepare_custom(&s, reader(root.path(), "candidate"), &Control::default()).unwrap();
    let path = candidate.summary().record;
    let copied = root.path().join("copied-candidate.json");
    fs::copy(&path, &copied).unwrap();
    assert!(models::read_prepared(&copied, &s, &Control::default()).is_err());
    let mut value: serde_json::Value = storage::read_json(&path, 4_000_000).unwrap();
    value["model"]["name"] = "tampered candidate".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(models::read_prepared(&path, &s, &Control::default()).is_err());
    assert!(!s.models_dir.join("glm-ocr.notegen.json").exists());
}
#[test]
fn setup_reuse_commits_only_the_explicit_settings_output_and_preserves_preset_choice() {
    let root = tempfile::tempdir().unwrap();
    let s = settings(root.path());
    models::save_custom(
        &s.models_dir,
        reader(root.path(), "working"),
        &Control::default(),
    )
    .unwrap();
    let active = fs::read(s.models_dir.join("glm-ocr.notegen.json")).unwrap();
    let output = root.path().join("custom-data/selected-settings.json");
    let next = setup::install_to(
        &s,
        &setup::Options {
            models_only: true,
            model_ids: vec!["glm-ocr".into()],
            output_preset: Some(Preset::Maximum),
            ..Default::default()
        },
        &output,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    let saved: Settings = storage::read_json(&output, 1_000_000).unwrap();
    assert_eq!(saved.server, s.server);
    assert_eq!(saved.preset, Preset::Maximum);
    assert_eq!(next.preset, Preset::Maximum);
    assert_eq!(
        fs::read(s.models_dir.join("glm-ocr.notegen.json")).unwrap(),
        active
    );
    assert!(!s.models_dir.join("glm-ocr.previous.json").exists());
}
#[test]
fn unknown_settings_file_is_never_overwritten_by_reused_setup() {
    let root = tempfile::tempdir().unwrap();
    let s = settings(root.path());
    models::save_custom(
        &s.models_dir,
        reader(root.path(), "working"),
        &Control::default(),
    )
    .unwrap();
    let output = root.path().join("user-document.json");
    let original = br#"{"notes":"preserve user content"}"#;
    fs::write(&output, original).unwrap();
    assert!(
        setup::install_to(
            &s,
            &setup::Options {
                models_only: true,
                model_ids: vec!["glm-ocr".into()],
                ..Default::default()
            },
            &output,
            &Control::default(),
            &Activity::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(output).unwrap(), original);
    assert!(!s.models_dir.join("glm-ocr.previous.json").exists());
}
#[test]
fn runtime_rollback_without_a_local_canary_remains_pending_and_preserves_pointers() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    let previous_server = runtime.join("ai-backends/previous/llama-server.exe");
    fs::create_dir_all(previous_server.parent().unwrap()).unwrap();
    fs::write(&previous_server, b"MZsynthetic-not-executed").unwrap();
    let s = Settings {
        server: runtime.join("ai-backends/current/llama-server.exe"),
        ..settings(root.path())
    };
    let active = runtime.join("active-ai-runtime.json");
    let previous = runtime.join("previous-ai-runtime.json");
    storage::protected_json(
        &active,
        &setup::RuntimeRecord {
            server: s.server.clone(),
            backend: Backend::Cpu,
            build: "b12032".into(),
        },
    )
    .unwrap();
    storage::protected_json(
        &previous,
        &setup::RuntimeRecord {
            server: previous_server,
            backend: Backend::Cpu,
            build: "b12031".into(),
        },
    )
    .unwrap();
    let old_active = fs::read(&active).unwrap();
    let old_previous = fs::read(&previous).unwrap();
    let output = root.path().join("settings.json");
    storage::protected_json(&output, &s).unwrap();
    let old_settings = fs::read(&output).unwrap();
    let error = setup::rollback_runtime_to(&s, &output, &Control::default(), &Activity::default())
        .unwrap_err();
    assert!(error.to_string().contains("remains pending"));
    assert_eq!(fs::read(active).unwrap(), old_active);
    assert_eq!(fs::read(previous).unwrap(), old_previous);
    assert_eq!(fs::read(output).unwrap(), old_settings);
}
