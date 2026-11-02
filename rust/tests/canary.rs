use notegen::{
    canary,
    model_store::Store,
    planning,
    storage::{Control, fingerprint},
    types::*,
};
use std::fs;
fn table(value: &str) -> PageContent {
    PageContent {
        blocks: vec![Block {
            kind: "table".into(),
            rows: vec![
                vec!["Item".into(), "Amount".into()],
                vec!["Sales".into(), value.into()],
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}
#[test]
fn known_fixture_requires_printed_exact_number_and_complete_editable_cells() {
    let printed = PageContent {
        blocks: vec![Block::text("Revenue is 1250 in October.")],
        ..Default::default()
    };
    assert!(canary::evaluate(&printed, &table("1250")).passed());
    for value in ["12500", "125O", "1500", "[illegible]", "1250 1500"] {
        assert!(!canary::evaluate(&printed, &table(value)).passed());
    }
    for value in ["Revenue is 12500 in October.", "No number", ""] {
        assert!(
            !canary::evaluate(
                &PageContent {
                    blocks: vec![Block::text(value)],
                    ..Default::default()
                },
                &table("1250")
            )
            .passed()
        );
    }
    let only_description = PageContent {
        blocks: vec![Block {
            kind: "figure".into(),
            bbox: Some(BoxRect::full()),
            description: "1250".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(!canary::evaluate(&only_description, &table("1250")).passed());
}
#[test]
fn canary_preserves_review_uncertainty_and_rejects_unsafe_or_incomplete_topology() {
    let printed = PageContent {
        blocks: vec![Block::text("1250")],
        ..Default::default()
    };
    let mut input = table("1250");
    input.blocks[0].uncertain = true;
    let result = canary::evaluate(&printed, &input);
    assert!(result.passed() && result.table_uncertain);
    input.blocks[0].merges = vec![Merge {
        row: 0,
        col: 0,
        rowspan: 1,
        colspan: 2,
    }];
    assert!(!canary::evaluate(&printed, &input).passed());
    input.blocks[0].merges.clear();
    input.blocks[0].rows[1].pop();
    assert!(!canary::evaluate(&printed, &input).passed());
    assert!(canary::LIMITS.contains("does not establish general recognition accuracy"));
}
#[test]
fn runtime_identity_binds_all_adjacent_libraries_and_ignores_unrelated_user_files() {
    let root = tempfile::tempdir().unwrap();
    let server = root.path().join("llama-server.exe");
    fs::write(&server, b"MZsynthetic-native-executable").unwrap();
    let library = root.path().join("ggml.dll");
    fs::write(&library, b"library version one").unwrap();
    fs::write(
        root.path().join("private-notes.txt"),
        b"never part of runtime identity",
    )
    .unwrap();
    let one = canary::runtime_identity(&server, &Control::default()).unwrap();
    assert_eq!(one.files.len(), 2);
    assert!(one.files.iter().all(|f| f.name != "private-notes.txt"));
    fs::write(library, b"library version two").unwrap();
    let two = canary::runtime_identity(&server, &Control::default()).unwrap();
    assert_ne!(one, two);
    canary::validate_runtime_identity(&two).unwrap();
    let mut bad = two.clone();
    bad.files[0].name = "../escape.dll".into();
    assert!(canary::validate_runtime_identity(&bad).is_err());
    let mut duplicate = two;
    duplicate.files.push(duplicate.files[0].clone());
    duplicate.fingerprint = fingerprint(&(&duplicate.server, &duplicate.files)).unwrap();
    assert!(canary::validate_runtime_identity(&duplicate).is_err());
}
#[test]
fn canary_model_identity_binds_the_actual_pair_revision_and_reader_protocol() {
    let root = tempfile::tempdir().unwrap();
    let mut model = planning::catalog()
        .into_iter()
        .find(|m| m.id == "glm-ocr")
        .unwrap();
    model.weights = root.path().join("weights.gguf");
    model.projector = root.path().join("projector.gguf");
    model.sha256.clear();
    fs::write(&model.weights, b"GGUFweights").unwrap();
    fs::write(&model.projector, b"GGUFprojector").unwrap();
    let store = Store::open(&root.path().join("store")).unwrap();
    let mut stored = store
        .adopt_model(&model, &Control::default())
        .unwrap()
        .model;
    let one = canary::model_identity(&stored, &Control::default()).unwrap();
    stored.prompt = "structured".into();
    assert_ne!(
        one,
        canary::model_identity(&stored, &Control::default()).unwrap()
    );
    stored.sha256 = "a".repeat(64);
    assert!(canary::model_identity(&stored, &Control::default()).is_err());
}
#[test]
fn external_endpoint_cannot_create_a_local_activation_proof_or_touch_evidence() {
    let root = tempfile::tempdir().unwrap();
    let mut model = planning::catalog().remove(0);
    model.endpoint = Some("http://127.0.0.1:1".into());
    model.alias = "synthetic".into();
    let output = root.path().join("evidence");
    assert!(
        canary::run(
            &model,
            &Settings::default(),
            &output,
            &Control::default(),
            &Default::default()
        )
        .is_err()
    );
    assert!(!output.exists());
}
