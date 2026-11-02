use notegen::{evaluation::*, types::*};

fn content(s: &str) -> PageContent {
    PageContent {
        blocks: vec![Block::text(s)],
        ..Default::default()
    }
}
fn fixture() -> (Corpus, Vec<Prediction>) {
    (
        Corpus {
            schema: 1,
            version: "synthetic-1".into(),
            permission: "Synthetic test strings; no real handwriting accuracy claim".into(),
            samples: vec![Sample {
                id: "a".into(),
                writer: "synthetic-test".into(),
                split: "test".into(),
                reference: content("Total -12.50 kg"),
            }],
        },
        vec![Prediction {
            id: "a".into(),
            content: content("Total 12.50 kg"),
            uncertainty: 0.2,
        }],
    )
}
#[test]
fn character_word_and_exact_number_failures_are_separate() {
    let (c, p) = fixture();
    let s = score(&c, &p, "test").unwrap();
    assert_eq!(s.characters.errors, 1);
    assert_eq!(s.words.errors, 1);
    assert_eq!(s.exact_numbers.errors, 1);
    assert_eq!(s.exact_numbers.reference_units, 1);
    assert_eq!(s.table_cells.error_rate, None);
}
#[test]
fn unicode_and_empty_references_do_not_divide_by_zero() {
    let (mut c, mut p) = fixture();
    c.samples[0].reference = content("සිංහල தமிழ்");
    p[0].content = c.samples[0].reference.clone();
    assert_eq!(score(&c, &p, "test").unwrap().characters.errors, 0);
    c.samples[0].reference = content("");
    p[0].content = content("added");
    let s = score(&c, &p, "test").unwrap();
    assert_eq!(s.characters.errors, 5);
    assert_eq!(s.characters.error_rate, None);
}
#[test]
fn tables_measure_cells_and_missing_rows() {
    let (mut c, mut p) = fixture();
    let table = Block {
        kind: "table".into(),
        rows: vec![vec!["A".into(), "1".into()], vec!["B".into(), "2".into()]],
        ..Default::default()
    };
    c.samples[0].reference.blocks = vec![table.clone()];
    p[0].content.blocks = vec![table];
    p[0].content.blocks[0].rows.pop();
    let s = score(&c, &p, "test").unwrap();
    assert_eq!(s.table_cells.errors, 2);
    assert_eq!(s.table_cells.error_rate, Some(0.5));
}
#[test]
fn corpus_rejects_permission_gaps_duplicate_ids_and_writer_leakage() {
    let (mut c, _) = fixture();
    c.permission.clear();
    assert!(c.validate().is_err());
    c.permission = "synthetic".into();
    c.samples.push(c.samples[0].clone());
    assert!(c.validate().is_err());
    c.samples[1].id = "b".into();
    c.samples[1].split = "calibration".into();
    assert!(c.validate().is_err());
}
#[test]
fn predictions_must_cover_selected_split_exactly() {
    let (c, mut p) = fixture();
    assert!(score(&c, &[], "test").is_err());
    p.push(p[0].clone());
    assert!(score(&c, &p, "test").is_err());
    p.pop();
    p[0].uncertainty = f64::NAN;
    assert!(score(&c, &p, "test").is_err());
}
#[test]
fn calibration_uses_only_held_out_calibration_and_reports_sparse_intervals() {
    let (mut c, mut p) = fixture();
    assert!(calibrate(&c, &p).is_err());
    c.samples[0].split = "calibration".into();
    p[0].uncertainty = 1.;
    let bins = calibrate(&c, &p).unwrap();
    assert_eq!(bins[9].samples, 1);
    assert_eq!(bins[9].observed_error_rate, Some(1.));
    assert!(bins[9].wilson_lower.unwrap() < 0.3);
    assert_eq!(bins[0].observed_error_rate, None);
}
#[test]
fn huge_samples_fail_boundedly() {
    let (mut c, mut p) = fixture();
    c.samples[0].reference = content(&"a".repeat(6000));
    p[0].content = content(&"b".repeat(6000));
    assert!(score(&c, &p, "test").is_err());
}
