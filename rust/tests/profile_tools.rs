use image::{DynamicImage, GrayImage, Luma};
use notegen::{
    fontpack::{self, FontOptions},
    profile_tools as tools,
    storage::Control,
    types::*,
    writers,
};
use std::{fs, io::Read, path::Path, sync::atomic::Ordering};

fn ink() -> DynamicImage {
    let mut im = GrayImage::from_pixel(100, 120, Luma([255]));
    for y in 20..100 {
        for x in 20..65 {
            if x < 30 || y < 32 || y > 50 && y < 62 {
                im[(x, y)] = Luma([0]);
            }
        }
    }
    DynamicImage::ImageLuma8(im)
}
fn font_bytes(root: &Path, id: &str, name: &str) -> (Vec<u8>, fontpack::FontCoverage) {
    let path = root.join(name);
    fontpack::export(
        root,
        id,
        &path,
        &FontOptions::default(),
        &Control::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut bytes = vec![];
    zip.by_name("Handwriting.ttf")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let coverage = serde_json::from_str(&read_zip(&mut zip, "font-coverage.json")).unwrap();
    (bytes, coverage)
}
fn shape(bytes: &[u8], text: &str, features: &[(&[u8; 4], u32)]) -> Vec<u32> {
    let font = harfrust::FontRef::new(bytes).unwrap();
    let data = harfrust::ShaperData::new(&font);
    let shaper = data.shaper(&font).build();
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let features = features
        .iter()
        .map(|(tag, value)| harfrust::Feature::new(harfrust::Tag::new(tag), *value, ..))
        .collect::<Vec<_>>();
    shaper
        .shape(buffer, harfrust::ShapeOptions::new().features(&features))
        .glyph_infos()
        .iter()
        .map(|glyph| glyph.glyph_id)
        .collect()
}
fn fixture(root: &Path) -> writers::Profile {
    let p = writers::create(root, "Private name").unwrap();
    writers::update(root, &p.id, &p.name, "Private notes").unwrap();
    for (label, glyph) in [
        ("r", true),
        ("n", true),
        ("1", true),
        ("l", true),
        ("Electrochemistry H2O", false),
        ("සිංහල Tamil", false),
    ] {
        writers::add_example(
            root,
            &p.id,
            &ink(),
            BoxRect::full(),
            label,
            glyph,
            "Private source description",
        )
        .unwrap();
    }
    writers::read(root, &p.id).unwrap()
}
fn read_zip(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> String {
    let mut value = String::new();
    zip.by_name(name)
        .unwrap()
        .read_to_string(&mut value)
        .unwrap();
    value
}

#[test]
fn old_profiles_still_load_and_coaching_counts_only_confirmed_active_examples() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let path = dir.path().join(&p.id).join("profile.json");
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for sample in json["samples"].as_array_mut().unwrap() {
        sample.as_object_mut().unwrap().remove("quarantine");
        sample.as_object_mut().unwrap().remove("conditions");
    }
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(
        writers::read(dir.path(), &p.id)
            .unwrap()
            .samples
            .iter()
            .all(|s| s.active())
    );
    let report = tools::coaching(dir.path(), &p.id).unwrap();
    assert_eq!(report.active_samples, 6);
    assert_eq!(report.glyph_counts["r"], 1);
    assert!(!report.missing_digits.contains('1'));
    assert!(report.missing_digits.contains('0'));
    assert!(report.observed_characters.keys().any(|s| !s.is_ascii()));
    assert!(
        report
            .suggestions
            .iter()
            .any(|s| s.contains("not recognition accuracy"))
    );
    let id = &p.samples[0].id;
    tools::set_conditions(
        dir.path(),
        &p.id,
        id,
        &["slanted".into(), "slanted".into(), "blur".into()],
    )
    .unwrap();
    let report = tools::coaching(dir.path(), &p.id).unwrap();
    assert_eq!(report.confirmed_conditions["slanted"], 1);
    assert!(!report.missing_conditions.contains(&"blur".into()));
    assert!(tools::set_conditions(dir.path(), &p.id, id, &["../../unsafe".into()]).is_err());
}
#[test]
fn quarantine_excludes_context_glyphs_coverage_and_cache_then_restores_reversibly() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let root = dir.path();
    let before = writers::digest(root, Some(&p.id)).unwrap();
    let first = &p.samples[0];
    assert!(writers::coverage(&p).contains('r'));
    let q = tools::quarantine(root, &p.id, &first.id, "Wrong label needs review").unwrap();
    assert!(
        q.samples[0]
            .quarantine
            .as_ref()
            .unwrap()
            .reason
            .contains("Wrong label")
    );
    assert!(root.join(&p.id).join(&first.image).exists());
    assert!(!writers::coverage(&q).contains('r'));
    assert!(writers::average_glyph(root, &q, 'r').is_err());
    assert!(
        !writers::reference(root, &p.id)
            .unwrap()
            .unwrap()
            .description
            .contains("= \"r\"")
    );
    let excluded = writers::digest(root, Some(&p.id)).unwrap();
    assert_ne!(before, excluded);
    tools::quarantine(root, &p.id, &first.id, "Second reason only").unwrap();
    assert_eq!(excluded, writers::digest(root, Some(&p.id)).unwrap());
    let report = tools::coaching(root, &p.id).unwrap();
    assert_eq!(report.quarantined_samples, 1);
    assert_eq!(report.active_samples, 5);
    tools::restore_example(root, &p.id, &first.id).unwrap();
    assert_eq!(before, writers::digest(root, Some(&p.id)).unwrap());
    assert!(tools::quarantine(root, &p.id, &first.id, "").is_err());
    assert!(tools::restore_example(root, &p.id, "missing").is_err());
}
#[test]
fn quarantined_examples_never_reach_generated_fonts_or_all_quarantined_references() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    for sample in p.samples.iter().filter(|s| s.glyph && s.label != "n") {
        tools::quarantine(root, &p.id, &sample.id, "Not trusted").unwrap();
    }
    let output = root.join("font.zip");
    notegen::fontpack::export(
        root,
        &p.id,
        &output,
        &FontOptions::default(),
        &Control::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    let mut bytes = vec![];
    zip.by_name("Handwriting.ttf")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
    assert!(face.glyph_index('n').is_some());
    assert!(face.glyph_index('r').is_none());
    for sample in &p.samples {
        tools::quarantine(root, &p.id, &sample.id, "Not trusted").unwrap();
    }
    assert!(writers::reference(root, &p.id).unwrap().is_none());
}
#[test]
fn confusion_and_glossary_suggestions_are_traceable_preview_only_and_exclude_quarantine() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    let before = fs::read(root.join(&p.id).join("profile.json")).unwrap();
    let exercises = tools::confusion_exercises(root, &p.id).unwrap();
    let rn = exercises
        .iter()
        .find(|e| e.first == "r" && e.second == "n")
        .unwrap();
    assert!(!rn.first_examples.is_empty());
    assert!(!rn.second_examples.is_empty());
    assert!(rn.prompt.contains("not a measured error"));
    let terms = tools::glossary_suggestions(root, &p.id).unwrap();
    let term = terms.iter().find(|s| s.term == "Electrochemistry").unwrap();
    assert_eq!(term.example_ids, vec![p.samples[4].id.clone()]);
    assert!(term.reason.contains("only after you approve"));
    assert_eq!(
        before,
        fs::read(root.join(&p.id).join("profile.json")).unwrap()
    );
    tools::quarantine(root, &p.id, &p.samples[4].id, "Uncertain transcription").unwrap();
    assert!(
        !tools::glossary_suggestions(root, &p.id)
            .unwrap()
            .iter()
            .any(|s| s.term == "Electrochemistry")
    );
}
#[test]
fn profile_versions_compare_changes_and_branch_after_original_images_were_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    let c = Control::default();
    let first = tools::snapshot(root, &p.id, "Before experiments", &c).unwrap();
    tools::quarantine(root, &p.id, &p.samples[0].id, "Compare without this glyph").unwrap();
    writers::delete_example(root, &p.id, &p.samples[1].id).unwrap();
    writers::add_example(root, &p.id, &ink(), BoxRect::full(), "new-term", false, "").unwrap();
    let second = tools::snapshot(root, &p.id, "After experiments", &c).unwrap();
    let delta = tools::compare_snapshots(root, &p.id, &first.id, &second.id, &c).unwrap();
    assert_eq!(delta.changed, vec![p.samples[0].id.clone()]);
    assert_eq!(delta.removed, vec![p.samples[1].id.clone()]);
    assert_eq!(delta.added.len(), 1);
    assert_eq!(tools::snapshots(root, &p.id).unwrap().len(), 2);
    let fork = tools::branch(root, &p.id, Some(&first.id), "Restored branch", &c).unwrap();
    assert_ne!(fork.id, p.id);
    assert_eq!(fork.name, "Restored branch");
    assert_eq!(fork.samples.len(), p.samples.len());
    assert!(root.join(&fork.id).join(&p.samples[1].image).is_file());
    assert!(fork.samples[0].active());
    assert!(!writers::read(root, &p.id).unwrap().samples[0].active());
    assert!(tools::branch(root, &p.id, Some("../escape"), "bad", &c).is_err());
}
#[test]
fn damaged_profile_snapshot_is_rejected_without_creating_a_branch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    let c = Control::default();
    let version = tools::snapshot(root, &p.id, "Stable", &c).unwrap();
    let path = root
        .join(&p.id)
        .join("versions")
        .join(&version.id)
        .join("profile.zip");
    fs::write(path, b"damaged").unwrap();
    assert!(tools::branch(root, &p.id, Some(&version.id), "bad", &c).is_err());
    assert_eq!(writers::list(root).unwrap().len(), 1);
}
#[test]
fn selective_sharing_previews_only_chosen_files_and_exports_private_metadata_only_when_selected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    let c = Control::default();
    let selection = tools::ShareSelection {
        sample_ids: vec![p.samples[4].id.clone()],
        ..Default::default()
    };
    let preview = tools::share_preview(root, &p.id, &selection, &c).unwrap();
    assert_eq!(preview.selected_samples, 1);
    assert_eq!(preview.labels, vec!["Electrochemistry H2O"]);
    assert!(preview.image_bytes > 0);
    assert_eq!(preview.files.len(), 4);
    assert!(
        !preview
            .included_metadata
            .contains(&"source descriptions".into())
    );
    let output = root.join("selected.zip");
    tools::export_selected(
        root,
        &p.id,
        &selection,
        &preview.confirmation_token,
        &output,
        &c,
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let metadata = read_zip(&mut zip, "profile.json");
    assert!(!metadata.contains("Private"));
    let shared: writers::Profile = serde_json::from_str(&metadata).unwrap();
    assert_eq!(shared.samples.len(), 1);
    assert_eq!(shared.name, "Shared writer");
    assert!(shared.samples[0].source.is_empty());
    assert!(shared.samples[0].created.is_empty());
    assert!(
        zip.by_name(&format!("samples/{}", p.samples[0].image))
            .is_err()
    );
    drop(zip);
    let imported = writers::import_profile(root, &output, &c).unwrap();
    assert_eq!(imported.samples.len(), 1);
    assert!(
        tools::export_selected(
            root,
            &p.id,
            &selection,
            &preview.confirmation_token,
            &output,
            &c
        )
        .is_err()
    );
    let private = tools::ShareSelection {
        include_name: true,
        include_notes: true,
        include_sources: true,
        include_dates: true,
        ..selection
    };
    let preview = tools::share_preview(root, &p.id, &private, &c).unwrap();
    let output = root.join("explicit-metadata.zip");
    tools::export_selected(
        root,
        &p.id,
        &private,
        &preview.confirmation_token,
        &output,
        &c,
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    assert!(read_zip(&mut zip, "profile.json").contains("Private source description"));
}
#[test]
fn sharing_rejects_stale_preview_quarantined_examples_and_cancelled_exports() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    let c = Control::default();
    let selection = tools::ShareSelection {
        sample_ids: vec![p.samples[0].id.clone()],
        ..Default::default()
    };
    let preview = tools::share_preview(root, &p.id, &selection, &c).unwrap();
    tools::set_conditions(root, &p.id, &p.samples[0].id, &["blur".into()]).unwrap();
    let target = root.join("stale.zip");
    assert!(
        tools::export_selected(
            root,
            &p.id,
            &selection,
            &preview.confirmation_token,
            &target,
            &c
        )
        .is_err()
    );
    assert!(!target.exists());
    tools::quarantine(root, &p.id, &p.samples[0].id, "Bad example").unwrap();
    assert!(tools::share_preview(root, &p.id, &selection, &c).is_err());
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(tools::snapshot(root, &p.id, "cancelled", &c).is_err());
    assert!(
        tools::font_proof_sheet(
            root,
            &p.id,
            &root.join("cancelled.html"),
            &FontOptions::default(),
            &c
        )
        .is_err()
    );
    assert!(!root.join("cancelled.html").exists());
}
#[test]
fn printable_font_proof_embeds_actual_ttf_and_marks_missing_glyphs_without_html_injection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    let c = Control::default();
    writers::add_example(
        root,
        &p.id,
        &ink(),
        BoxRect::full(),
        "<script>alert(1)</script>",
        false,
        "",
    )
    .unwrap();
    let target = root.join("proof.html");
    let summary = tools::font_proof_sheet(
        root,
        &p.id,
        &target,
        &FontOptions {
            family: "Name <script>".into(),
            ..Default::default()
        },
        &c,
    )
    .unwrap();
    assert_eq!(summary.glyph_count, 4);
    assert!(!summary.missing_letters_and_digits.contains('r'));
    assert!(summary.missing_letters_and_digits.contains('A'));
    let html = fs::read_to_string(&target).unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("@media print"));
    assert!(html.contains("class=\"missing\""));
    let encoded = html
        .split("data:font/ttf;base64,")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap();
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let font = ttf_parser::Face::parse(&bytes, 0).unwrap();
    assert!(font.glyph_index('r').is_some());
    assert!(font.glyph_index('A').is_none());
    assert!(tools::font_proof_sheet(root, &p.id, &target, &FontOptions::default(), &c).is_err());
}
#[test]
fn confirmed_unicode_ligatures_clusters_and_alternates_shape_with_real_opentype_engine() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = writers::create(root, "Unicode confirmed shapes").unwrap();
    for label in ["f", "i", "a", "Ж", "𝄞"] {
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), label, true, "").unwrap();
    }
    writers::add_example(
        root,
        &p.id,
        &ink(),
        BoxRect::full(),
        "fi",
        false,
        "ordinary word, not a font ligature",
    )
    .unwrap();
    let (before, _) = font_bytes(root, &p.id, "before.zip");
    assert_eq!(
        shape(&before, "fi", &[]).len(),
        2,
        "Ordinary word examples must never imply a ligature"
    );
    for (label, form) in [
        ("fi", writers::FontShape::Ligature),
        ("ffi", writers::FontShape::Ligature),
        ("කි", writers::FontShape::Cluster),
        ("a", writers::FontShape::Alternate { index: 1 }),
        ("a", writers::FontShape::Alternate { index: 2 }),
    ] {
        writers::add_font_example(
            root,
            &p.id,
            &ink(),
            BoxRect::full(),
            label,
            form,
            "explicitly confirmed complete shape",
        )
        .unwrap();
    }
    let (bytes, coverage) = font_bytes(root, &p.id, "shaped.zip");
    let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
    let id = |label: &str, form: writers::FontShape| {
        u32::from(
            coverage
                .shapes
                .iter()
                .find(|rule| rule.label == label && rule.shape == form)
                .unwrap()
                .glyph_id,
        )
    };
    assert_eq!(
        shape(&bytes, "fi", &[]),
        vec![id("fi", writers::FontShape::Ligature)]
    );
    assert_eq!(
        shape(&bytes, "ffi", &[]),
        vec![id("ffi", writers::FontShape::Ligature)],
        "Longest confirmed sequence must win before its suffix"
    );
    assert_eq!(shape(&bytes, "fi", &[(b"liga", 0)]).len(), 2);
    assert_eq!(
        shape(&bytes, "කි", &[]),
        vec![id("කි", writers::FontShape::Cluster)]
    );
    assert_eq!(shape(&bytes, "කි", &[(b"ccmp", 0)]).len(), 2);
    let base = u32::from(face.glyph_index('a').unwrap().0);
    assert_eq!(shape(&bytes, "a", &[]), vec![base]);
    assert_eq!(
        shape(&bytes, "a", &[(b"salt", 1)]),
        vec![id("a", writers::FontShape::Alternate { index: 1 })]
    );
    assert_eq!(
        shape(&bytes, "a", &[(b"salt", 2)]),
        vec![id("a", writers::FontShape::Alternate { index: 2 })]
    );
    assert_eq!(
        shape(&bytes, "a", &[(b"ss01", 1)]),
        vec![id("a", writers::FontShape::Alternate { index: 1 })]
    );
    assert_eq!(
        shape(&bytes, "a", &[(b"ss02", 1)]),
        vec![id("a", writers::FontShape::Alternate { index: 2 })]
    );
    for ch in ['Ж', '𝄞'] {
        assert_eq!(
            shape(&bytes, &ch.to_string(), &[]),
            vec![u32::from(face.glyph_index(ch).unwrap().0)]
        );
        assert!(
            face.tables()
                .os2
                .unwrap()
                .unicode_ranges()
                .contains_char(ch)
        );
    }
    assert!(
        face.tables()
            .os2
            .unwrap()
            .unicode_ranges()
            .contains_char('ක')
    );
    assert_eq!(shape(&bytes, "z", &[]), vec![0]);
    assert!(!coverage.default_glyphs.contains('ක'));
    assert!(coverage.component_placeholders.contains('ක'));
    let placeholder = face.glyph_index('ක').unwrap();
    let bbox = face.glyph_bounding_box(placeholder).unwrap();
    assert!(
        bbox.x_max > bbox.x_min && bbox.y_max > bbox.y_min,
        "Unconfirmed standalone components must render visible boxes"
    );
    let source = ttf_parser::Face::parse(&before, 0).unwrap();
    assert_eq!(
        source.glyph_bounding_box(source.glyph_index('a').unwrap()),
        face.glyph_bounding_box(face.glyph_index('a').unwrap()),
        "Alternates must not be averaged into the default outline"
    );
    assert_eq!(
        face.number_of_glyphs() as usize,
        2 + coverage.default_glyphs.chars().count()
            + coverage.component_placeholders.chars().count()
            + coverage.shapes.len()
    );
    assert!(face.tables().gsub.is_some());
    let (again, _) = font_bytes(root, &p.id, "again.zip");
    assert_eq!(
        bytes, again,
        "Identical confirmed input must produce deterministic font bytes"
    );
}

#[test]
fn quarantined_shaping_forms_leave_no_active_rule_and_profile_sharing_preserves_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = writers::create(root, "Explicit forms").unwrap();
    writers::add_example(root, &p.id, &ink(), BoxRect::full(), "a", true, "").unwrap();
    for index in [1, 2] {
        writers::add_font_example(
            root,
            &p.id,
            &ink(),
            BoxRect::full(),
            "a",
            writers::FontShape::Alternate { index },
            "",
        )
        .unwrap();
    }
    let p = writers::add_font_example(
        root,
        &p.id,
        &ink(),
        BoxRect::full(),
        "fi",
        writers::FontShape::Ligature,
        "",
    )
    .unwrap();
    let rejected = p
        .samples
        .iter()
        .find(|sample| sample.font_shape == Some(writers::FontShape::Alternate { index: 1 }))
        .unwrap()
        .id
        .clone();
    tools::quarantine(root, &p.id, &rejected, "wrong first alternate").unwrap();
    let (bytes, coverage) = font_bytes(root, &p.id, "quarantine.zip");
    assert!(
        !coverage
            .shapes
            .iter()
            .any(|rule| rule.sample_ids.contains(&rejected))
    );
    let base = ttf_parser::Face::parse(&bytes, 0)
        .unwrap()
        .glyph_index('a')
        .unwrap()
        .0;
    assert_eq!(
        shape(&bytes, "a", &[(b"salt", 1)]),
        vec![u32::from(base)],
        "A quarantined alternate index must not be renumbered to another form"
    );
    let second = coverage
        .shapes
        .iter()
        .find(|rule| rule.shape == writers::FontShape::Alternate { index: 2 })
        .unwrap()
        .glyph_id;
    assert_eq!(shape(&bytes, "a", &[(b"salt", 2)]), vec![u32::from(second)]);
    let share = root.join("profile.zip");
    writers::export_profile(root, &p.id, &share, &Control::default()).unwrap();
    let imported =
        writers::import_profile(&root.join("imported"), &share, &Control::default()).unwrap();
    let copy = fontpack::coverage(&root.join("imported"), &imported.id).unwrap();
    assert_eq!(
        serde_json::to_value(&coverage).unwrap(),
        serde_json::to_value(copy).unwrap()
    );
    let proof = root.join("forms.html");
    tools::font_proof_sheet(
        root,
        &p.id,
        &proof,
        &FontOptions::default(),
        &Control::default(),
    )
    .unwrap();
    let html = fs::read_to_string(proof).unwrap();
    assert!(html.contains("font-feature-settings:'salt' 2"));
    assert!(html.contains("font-feature-settings:'ss02' 1"));
    assert!(html.contains("Unconfirmed standalone components"));
    assert!(html.contains("font-feature-settings:'liga' 1\">fi</p>"));
}

#[test]
fn font_shape_confirmation_rejects_invalid_metadata_and_legacy_profiles_remain_usable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    let before = fs::read(root.join(&p.id).join("profile.json")).unwrap();
    for (label, form) in [
        ("fi", writers::FontShape::Cluster),
        ("a", writers::FontShape::Ligature),
        ("a", writers::FontShape::Alternate { index: 0 }),
        ("a", writers::FontShape::Alternate { index: 17 }),
        ("x\u{0}", writers::FontShape::Ligature),
        ("x\u{202e}", writers::FontShape::Ligature),
        ("abcdefghijklmnopq", writers::FontShape::Ligature),
    ] {
        assert!(
            writers::add_font_example(root, &p.id, &ink(), BoxRect::full(), label, form, "")
                .is_err()
        );
    }
    assert_eq!(
        before,
        fs::read(root.join(&p.id).join("profile.json")).unwrap()
    );
    assert!(
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), "\u{200d}", true, "").is_err()
    );
    let mut value: serde_json::Value = serde_json::from_slice(&before).unwrap();
    for sample in value["samples"].as_array_mut().unwrap() {
        sample.as_object_mut().unwrap().remove("font_shape");
    }
    fs::write(
        root.join(&p.id).join("profile.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(
        writers::read(root, &p.id).unwrap().samples.len(),
        p.samples.len()
    );
    let (_, coverage) = font_bytes(root, &p.id, "legacy.zip");
    assert!(coverage.shapes.is_empty());
    assert_eq!(coverage.default_glyphs, "1lnr");
    value["samples"][0]["font_shape"] = serde_json::json!({"kind":"alternate","index":0});
    fs::write(
        root.join(&p.id).join("profile.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(writers::read(root, &p.id).is_err());
}

#[test]
fn oversized_unicode_font_is_rejected_before_reading_samples_or_creating_output() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = writers::create(root, "Large Unicode font").unwrap();
    let mut metadata = serde_json::to_value(&p).unwrap();
    metadata["samples"] = serde_json::Value::Array(
        (0..4097)
            .map(|i| {
                let id = format!("glyph-{i}");
                serde_json::json!({"id":id,"label":char::from_u32(0x4e00+i).unwrap().to_string(),
            "glyph":true,"image":format!("glyph-{i}.png"),"created":"","source":""})
            })
            .collect(),
    );
    let path = root.join(&p.id).join("profile.json");
    fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let output = root.join("too-large.zip");
    let error = fontpack::export(
        root,
        &p.id,
        &output,
        &FontOptions::default(),
        &Control::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("4,096 mapped scalars"));
    assert!(!output.exists());
    assert_eq!(before, fs::read(path).unwrap());
}
#[test]
fn duplicate_sample_ids_and_invalid_quarantine_metadata_are_rejected_on_import_or_read() {
    let dir = tempfile::tempdir().unwrap();
    let p = fixture(dir.path());
    let path = dir.path().join(&p.id).join("profile.json");
    let mut value = serde_json::to_value(&p).unwrap();
    let first = value["samples"][0].clone();
    value["samples"].as_array_mut().unwrap().push(first);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(writers::read(dir.path(), &p.id).is_err());
    let mut value = serde_json::to_value(&p).unwrap();
    value["samples"][0]["quarantine"] = serde_json::json!({"reason":"", "created":"now"});
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(writers::read(dir.path(), &p.id).is_err());
}

#[test]
fn quarantine_reason_changes_leave_exact_recognition_reference_unchanged_and_revision_overflow_fails_safely()
 {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = fixture(root);
    tools::quarantine(root, &p.id, &p.samples[0].id, "first reason").unwrap();
    let first = writers::reference(root, &p.id).unwrap().unwrap();
    tools::quarantine(
        root,
        &p.id,
        &p.samples[0].id,
        "second administrative reason",
    )
    .unwrap();
    let second = writers::reference(root, &p.id).unwrap().unwrap();
    assert_eq!(first.description, second.description);
    assert_eq!(first.png, second.png);
    assert_eq!(first.revision, second.revision);
    let path = root.join(&p.id).join("profile.json");
    let mut value = serde_json::to_value(writers::read(root, &p.id).unwrap()).unwrap();
    value["revision"] = serde_json::json!(u64::MAX);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(tools::restore_example(root, &p.id, &p.samples[0].id).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}
