use notegen::{
    bundles, models, setup,
    storage::{Control, hash_file},
    types::*,
};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::Ordering,
};

fn fixture(root: &Path) -> Settings {
    fs::create_dir_all(root).unwrap();
    let weights = root.join("weights.gguf");
    let projector = root.join("projector.gguf");
    fs::write(&weights, b"GGUFconfirmed fixture weights").unwrap();
    fs::write(&projector, b"GGUFconfirmed fixture projector").unwrap();
    let (mut model, _) = setup::pinned("glm-ocr").unwrap();
    model.id = "fixture-reader".into();
    model.name = "Offline fixture".into();
    model.family = "fixture".into();
    model.revision = "local-fixture-1".into();
    model.weights = weights;
    model.projector = projector;
    let models_dir = root.join("registry");
    models::save_custom(&models_dir, model, &Control::default()).unwrap();
    Settings {
        models_dir,
        server: root.join("runtime/llama-server.exe"),
        subject: "PRIVATE DOCUMENT CONTEXT".into(),
        ..Default::default()
    }
}
fn ids() -> Vec<String> {
    vec!["fixture-reader".into()]
}
fn bundle(s: &Settings, root: &Path) -> std::path::PathBuf {
    let output = root.join("bundle.zip");
    bundles::export(s, &ids(), None, &output, &Control::default()).unwrap();
    output
}
fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut item = zip.by_index(i).unwrap();
            let name = item.name().to_string();
            let mut bytes = vec![];
            item.read_to_end(&mut bytes).unwrap();
            (name, bytes)
        })
        .collect()
}
fn altered(input: &Path, output: &Path, change: impl FnOnce(&mut Vec<(String, Vec<u8>)>)) {
    let mut all = entries(input);
    change(&mut all);
    let mut zip = zip::ZipWriter::new(fs::File::create(output).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in all {
        zip.start_file(name, options).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
}
fn change_inventory(all: &mut [(String, Vec<u8>)], change: impl FnOnce(&mut serde_json::Value)) {
    let entry = all.iter_mut().find(|(n, _)| n == "inventory.json").unwrap();
    let mut v = serde_json::from_slice(&entry.1).unwrap();
    change(&mut v);
    entry.1 = serde_json::to_vec(&v).unwrap();
}

#[test]
fn offline_preview_lists_exact_bytes_hashes_and_excludes_private_settings_paths_and_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let c = Control::default();
    let preview = bundles::preview(&s, &ids(), None, &c).unwrap();
    assert_eq!(preview.readers.len(), 1);
    assert_eq!(preview.files.len(), 2);
    assert_eq!(
        preview.total_bytes,
        preview.files.iter().map(|f| f.bytes).sum()
    );
    assert!(preview.readers[0].model.weights.as_os_str().is_empty());
    assert!(preview.readers[0].model.projector.as_os_str().is_empty());
    assert!(preview.readers[0].model.endpoint.is_none());
    let output = bundle(&s, dir.path());
    let contents = entries(&output);
    let metadata = String::from_utf8(
        contents
            .iter()
            .find(|(n, _)| n == "inventory.json")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    assert!(!metadata.contains("PRIVATE DOCUMENT CONTEXT"));
    assert!(!metadata.contains("source/registry"));
    assert_eq!(contents.len(), 3);
    let checked = bundles::inspect(&output, &c).unwrap();
    assert_eq!(checked.total_bytes, preview.total_bytes);
}
#[test]
fn offline_import_isolated_registry_activates_only_via_returned_settings_and_keeps_old_versions() {
    let dir = tempfile::tempdir().unwrap();
    let source = fixture(&dir.path().join("source"));
    let original = fixture(&dir.path().join("destination-original"));
    let c = Control::default();
    let output = bundle(&source, dir.path());
    let old_descriptor = original.models_dir.join("fixture-reader.notegen.json");
    let before = fs::read(&old_descriptor).unwrap();
    let destination = dir.path().join("imported");
    let next = bundles::import(&original, &output, &destination, &c).unwrap();
    assert_eq!(fs::read(&old_descriptor).unwrap(), before);
    assert_ne!(next.models_dir, original.models_dir);
    assert_eq!(next.server, original.server);
    let models = notegen::runtime::inventory(&next.models_dir).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].revision, "local-fixture-1");
    assert!(models[0].weights.starts_with(&destination));
    assert!(
        next.models_dir
            .join("fixture-reader.previous.json")
            .is_file()
    );
    assert_eq!(
        hash_file(&models[0].weights, &c).unwrap(),
        bundles::inspect(&output, &c)
            .unwrap()
            .files
            .iter()
            .find(|f| f.path.ends_with(".gguf")
                && f.bytes == models[0].weights.metadata().unwrap().len())
            .unwrap()
            .sha256
    );
}
#[test]
fn repeated_objects_are_stored_once_and_nontransferred_existing_readers_are_retained() {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let c = Control::default();
    let mut model = notegen::runtime::inventory(&s.models_dir)
        .unwrap()
        .remove(0);
    model.id = "another-reader".into();
    model.projector = model.weights.clone();
    models::save_custom(&s.models_dir, model, &c).unwrap();
    let only_other = vec!["another-reader".into()];
    let preview = bundles::preview(&s, &only_other, None, &c).unwrap();
    assert_eq!(preview.files.len(), 1);
    let output = dir.path().join("dedup.zip");
    bundles::export(&s, &only_other, None, &output, &c).unwrap();
    let next = bundles::import(&s, &output, &dir.path().join("dedup-import"), &c).unwrap();
    let models = notegen::runtime::inventory(&next.models_dir).unwrap();
    assert_eq!(models.len(), 2);
    assert!(models.iter().any(|m| m.id == "fixture-reader"));
}
#[test]
fn altered_checksums_fail_transactionally_and_existing_output_or_destination_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let output = bundle(&s, dir.path());
    let c = Control::default();
    let old = fs::read(&output).unwrap();
    assert!(bundles::export(&s, &ids(), None, &output, &c).is_err());
    assert_eq!(fs::read(&output).unwrap(), old);
    let bad = dir.path().join("corrupt.zip");
    altered(&output, &bad, |all| {
        all.iter_mut()
            .find(|(n, _)| n.ends_with(".gguf"))
            .unwrap()
            .1[4] ^= 1;
    });
    let active = fs::read(s.models_dir.join("fixture-reader.notegen.json")).unwrap();
    let destination = dir.path().join("bad-import");
    assert!(bundles::import(&s, &bad, &destination, &c).is_err());
    assert!(!destination.exists());
    assert!(bundles::inspect(&bad, &c).is_err());
    assert_eq!(
        fs::read(s.models_dir.join("fixture-reader.notegen.json")).unwrap(),
        active
    );
    let existing = dir.path().join("exists");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep.txt"), b"keep").unwrap();
    assert!(bundles::import(&s, &output, &existing, &c).is_err());
    assert_eq!(fs::read(existing.join("keep.txt")).unwrap(), b"keep");
}
#[test]
fn bundle_parser_rejects_traversal_duplicate_entries_extra_content_and_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let output = bundle(&s, dir.path());
    let c = Control::default();
    for (i, name) in [
        "../escape.dll",
        "objects/../../escape.dll",
        "C:/escape.dll",
        "objects\\escape.dll",
        "extra.txt",
        "INVENTORY.JSON",
    ]
    .iter()
    .enumerate()
    {
        let bad = dir.path().join(format!("unsafe-{i}.zip"));
        altered(&output, &bad, |all| {
            all.push((name.to_string(), b"bad".to_vec()))
        });
        assert!(bundles::inspect(&bad, &c).is_err(), "accepted {name}");
        assert!(
            bundles::import(&s, &bad, &dir.path().join(format!("unsafe-import-{i}")), &c).is_err()
        );
    }
    let bad = dir.path().join("symlink.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&bad).unwrap());
    for (name, bytes) in entries(&output) {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.add_symlink(
        "objects/link.gguf",
        "../outside",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.finish().unwrap();
    assert!(bundles::inspect(&bad, &c).is_err());
    assert!(!dir.path().join("escape.dll").exists());
}
#[test]
fn oversized_manifest_unpinned_runtime_and_private_endpoint_metadata_are_rejected_before_extraction()
 {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let output = bundle(&s, dir.path());
    let c = Control::default();
    for kind in 0..4 {
        let bad = dir.path().join(format!("metadata-{kind}.zip"));
        altered(&output, &bad, |all| {
            change_inventory(all, |v| match kind {
                0 => {
                    v["files"][0]["bytes"] = serde_json::json!(100_000_000_001u64);
                }
                1 => {
                    v["readers"][0]["model"]["endpoint"] =
                        serde_json::json!("http://127.0.0.1:1234");
                    v["readers"][0]["model"]["alias"] = serde_json::json!("secret endpoint");
                }
                2 => {
                    v["runtime"] = serde_json::json!({"backend":"cpu","build":"b1","archives":[]});
                }
                _ => {
                    v["files"][0]["path"] = serde_json::json!("objects/../escape.gguf");
                }
            })
        });
        assert!(bundles::inspect(&bad, &c).is_err());
        let destination = dir.path().join(format!("bad-meta-{kind}"));
        assert!(bundles::import(&s, &bad, &destination, &c).is_err());
        assert!(!destination.exists());
    }
}
#[test]
fn cancellation_and_changed_registered_models_preserve_all_active_data() {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let output = bundle(&s, dir.path());
    let c = Control::default();
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(bundles::preview(&s, &ids(), None, &c).is_err());
    assert!(bundles::inspect(&output, &c).is_err());
    let destination = dir.path().join("cancelled");
    assert!(bundles::import(&s, &output, &destination, &c).is_err());
    assert!(!destination.exists());
    let model = notegen::runtime::inventory(&s.models_dir)
        .unwrap()
        .remove(0);
    fs::write(&model.weights, b"GGUFtampered weights").unwrap();
    assert!(bundles::preview(&s, &ids(), None, &Control::default()).is_err());
    assert!(!dir.path().join("no-output.zip").exists());
}
#[test]
fn preflight_counts_pinned_remaining_bytes_and_cards_do_not_invent_license_or_languages() {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let c = Control::default();
    let (_, pin) = setup::pinned("glm-ocr").unwrap();
    let check = bundles::preflight(&s, &["glm-ocr".into()], Some("cpu"), &c).unwrap();
    assert!(check.selected_bytes > pin.weights.bytes + pin.projector.bytes);
    assert_eq!(check.verified_reusable_bytes, 0);
    assert_eq!(check.remaining_download_bytes, check.selected_bytes);
    assert!(check.runtime_expansion_is_upper_bound);
    assert!(check.additional_disk_bytes >= check.remaining_download_bytes + 4_000_000_000);
    let cards = bundles::capability_cards().unwrap();
    assert_eq!(cards.len(), notegen::planning::catalog().len());
    for card in cards {
        assert_eq!(card.pinned_revision.len(), 40);
        assert!(card.tested_languages.is_empty());
        assert!(card.license.contains("Not recorded"));
        assert!(card.source.starts_with("https://"));
        assert!(card.download_bytes > 0);
    }
    assert!(bundles::preview(&s, &ids(), Some("unsupported"), &c).is_err());
    assert!(bundles::preview(&s, &[], None, &c).is_err());
}
#[cfg(unix)]
#[test]
fn source_and_destination_symlinks_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let s = fixture(&dir.path().join("source"));
    let output = bundle(&s, dir.path());
    let c = Control::default();
    let link = dir.path().join("bundle-link.zip");
    std::os::unix::fs::symlink(&output, &link).unwrap();
    assert!(bundles::inspect(&link, &c).is_err());
    let target = dir.path().join("actual");
    fs::create_dir(&target).unwrap();
    let link = dir.path().join("linked-dir");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(bundles::import(&s, &output, &link.join("new"), &c).is_err());
}
