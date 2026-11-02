use notegen::{
    model_store::{self, Kind, ObjectId, Store},
    planning,
    storage::Control,
};
use std::{fs, path::Path, sync::atomic::Ordering, thread};

fn model(root: &Path) -> planning::Model {
    let mut m = planning::catalog()
        .into_iter()
        .find(|m| m.id == "glm-ocr")
        .unwrap();
    m.weights = root.join("weights.gguf");
    m.projector = root.join("projector.gguf");
    m.sha256.clear();
    m.revision = "local-confirmed-a".into();
    fs::write(&m.weights, b"GGUFimmutable-weight-fixture").unwrap();
    fs::write(&m.projector, b"GGUFimmutable-projector-fixture").unwrap();
    m
}
#[test]
fn per_user_default_is_absolute_and_explicit_portable_stores_remain_supported() {
    assert!(model_store::default_root().unwrap().is_absolute());
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("portable-model-store")).unwrap();
    assert!(store.root.is_absolute());
    assert!(store.root.starts_with(root.path().canonicalize().unwrap()));
}
#[test]
fn identical_weights_and_projectors_share_objects_across_versions_without_linking_originals() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("store")).unwrap();
    let m = model(root.path());
    let one = store.adopt_model(&m, &Control::default()).unwrap();
    let mut next = m.clone();
    next.revision = "local-confirmed-b".into();
    let two = store.adopt_model(&next, &Control::default()).unwrap();
    assert_eq!(one.model.weights, two.model.weights);
    assert_eq!(one.model.projector, two.model.projector);
    assert_ne!(one.reference.id, two.reference.id);
    assert_eq!(store.versions(&m.id, &Control::default()).unwrap().len(), 2);
    assert!(
        fs::metadata(&one.model.weights)
            .unwrap()
            .permissions()
            .readonly()
    );
    assert!(
        one.reference.model.weights.as_os_str().is_empty()
            && one.reference.model.projector.as_os_str().is_empty()
    );
    let original = fs::read(&one.model.weights).unwrap();
    fs::write(&m.weights, b"GGUFexternal-original-can-change").unwrap();
    assert_eq!(fs::read(&one.model.weights).unwrap(), original);
    let resolved = store
        .resolve_reference(&one.reference, &Control::default())
        .unwrap();
    assert_eq!(resolved.weights, one.model.weights);
    assert_eq!(resolved.revision, "local-confirmed-a");
    let count = fs::read_dir(store.root.join("objects"))
        .unwrap()
        .flat_map(|d| fs::read_dir(d.unwrap().path()).unwrap())
        .count();
    assert_eq!(count, 2);
}
#[test]
fn one_content_object_can_serve_both_gguf_roles_and_concurrent_ingest_is_noclobber() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("store")).unwrap();
    let mut m = model(root.path());
    m.projector = m.weights.clone();
    let adopted = store.adopt_model(&m, &Control::default()).unwrap();
    assert_eq!(adopted.model.weights, adopted.model.projector);
    let source = root.path().join("other.gguf");
    fs::write(&source, b"GGUFconcurrent-object").unwrap();
    let a = store.clone();
    let pa = source.clone();
    let b = store.clone();
    let pb = source;
    let first = thread::spawn(move || a.import_file(&pa, Kind::Gguf, &Control::default()).unwrap());
    let second =
        thread::spawn(move || b.import_file(&pb, Kind::Gguf, &Control::default()).unwrap());
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    assert_eq!(first, second);
    assert_eq!(fs::read(first.1).unwrap(), b"GGUFconcurrent-object");
}
#[test]
fn wrong_bytes_signature_and_cancellation_preserve_sources_and_publish_no_object() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("store")).unwrap();
    let source = root.path().join("source.gguf");
    let original = b"GGUForiginal-preserved";
    fs::write(&source, original).unwrap();
    let id = ObjectId {
        sha256: "a".repeat(64),
        bytes: original.len() as u64,
    };
    assert!(
        store
            .ingest(&source, &id, Kind::Gguf, &Control::default())
            .is_err()
    );
    assert!(!store.object_path(&id).unwrap().exists());
    assert_eq!(fs::read(&source).unwrap(), original);
    let cancel = Control::default();
    cancel.cancelled.store(true, Ordering::Relaxed);
    assert!(store.import_file(&source, Kind::Gguf, &cancel).is_err());
    fs::write(&source, b"not GGUF").unwrap();
    assert!(
        store
            .import_file(&source, Kind::Gguf, &Control::default())
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"not GGUF");
    assert!(
        store
            .object_path(&ObjectId {
                sha256: "../escape".into(),
                bytes: 8
            })
            .is_err()
    );
}
#[test]
fn version_metadata_is_exact_bounded_and_never_silently_reassigned() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("store")).unwrap();
    let m = model(root.path());
    let stored = store.adopt_model(&m, &Control::default()).unwrap();
    let mut oversized = stored.model.clone();
    oversized.source = "x".repeat(100000);
    assert!(
        store
            .write_reference(
                &oversized,
                stored.reference.weights.clone(),
                stored.reference.projector.clone(),
                &Control::default()
            )
            .is_err()
    );
    assert_eq!(store.versions(&m.id, &Control::default()).unwrap().len(), 1);
    let path = store
        .root
        .join("references")
        .join(&m.id)
        .join(format!("{}.json", stored.reference.id));
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["model"]["revision"] = "tampered".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        store
            .read_reference(&m.id, &stored.reference.id, &Control::default())
            .is_err()
    );
    assert!(
        store
            .resolve_reference(&stored.reference, &Control::default())
            .is_err()
    );
    assert_eq!(
        fs::read(&m.weights).unwrap(),
        b"GGUFimmutable-weight-fixture"
    );
}
#[cfg(unix)]
#[test]
fn immutable_store_refuses_symlink_sources_and_object_paths() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("store")).unwrap();
    let m = model(root.path());
    let link = root.path().join("linked.gguf");
    std::os::unix::fs::symlink(&m.weights, &link).unwrap();
    assert!(
        store
            .import_file(&link, Kind::Gguf, &Control::default())
            .is_err()
    );
    let (id, _) = store
        .import_file(&m.weights, Kind::Gguf, &Control::default())
        .unwrap();
    let path = store.object_path(&id).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&m.weights, &path).unwrap();
    assert!(store.resolve(&id, Kind::Gguf, &Control::default()).is_err());
    assert!(m.weights.exists());
}
