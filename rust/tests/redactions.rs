use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};
use notegen::{
    redactions::{self, MaskSpec, Reason},
    review,
    storage::{Control, Manifest, Project, fingerprint},
    types::*,
};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
    sync::atomic::Ordering,
};

fn region(left: f32, top: f32, right: f32, bottom: f32) -> BoxRect {
    BoxRect {
        left,
        top,
        right,
        bottom,
    }
}
fn fixture(root: &Path) -> Project {
    let mut settings = Settings {
        subject: "PRIVATE SUBJECT secret-user-name".into(),
        ..Default::default()
    };
    settings.assistance.glossary = "PRIVATE GLOSSARY secret-user-name".into();
    settings.writer.root = root.join("private-writer-name");
    let project = Project::open(
        root.join("private-project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1],
            models: vec!["PRIVATE READER".into()],
            settings,
        },
    )
    .unwrap();
    let content = PageContent {
        blocks: vec![
            Block {
                bbox: Some(region(0.1, 0.05, 0.9, 0.15)),
                confidence: Some(1.),
                ..Block::text("Public heading Ω")
            },
            Block {
                bbox: Some(region(0.2, 0.25, 0.5, 0.45)),
                confidence: Some(1.),
                alternatives: vec!["PRIVATE ALTERNATIVE secret-user-name".into()],
                ..Block::text("secret-user-name")
            },
            Block {
                kind: "table".into(),
                bbox: Some(region(0.1, 0.55, 0.9, 0.85)),
                rows: vec![vec!["Account".into(), "private-1250".into()]],
                ..Default::default()
            },
            Block::text("Ungrounded private note"),
            Block {
                kind: "figure".into(),
                bbox: Some(region(0.65, 0.2, 0.95, 0.4)),
                description: "Public geometric figure".into(),
                interpretation: "A public shape".into(),
                ..Default::default()
            },
        ],
        summary: "PRIVATE SUMMARY secret-user-name".into(),
        warnings: vec!["PRIVATE WARNING secret-user-name".into()],
    };
    project
        .save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            candidates: vec![Candidate {
                region: None,
                model_id: "PRIVATE MODEL".into(),
                family: "PRIVATE FAMILY".into(),
                view: "full".into(),
                content: content.clone(),
                reliability: 1.,
            }],
            source_warnings: vec!["PRIVATE SOURCE WARNING".into()],
            content,
            ..Default::default()
        })
        .unwrap();
    let mut image = RgbImage::from_pixel(100, 80, Rgb([255, 255, 255]));
    for y in 20..36 {
        for x in 20..50 {
            image[(x, y)] = Rgb([190, 17, 90]);
        }
    }
    image.save(project.image(1)).unwrap();
    fs::write(
        project.root.join("guidance.json"),
        "PRIVATE GUIDANCE secret-user-name",
    )
    .unwrap();
    fs::create_dir(project.root.join("responses")).unwrap();
    fs::write(
        project.root.join("responses/reader.txt"),
        "PRIVATE RESPONSE secret-user-name",
    )
    .unwrap();
    project
}
fn add_mask(project: &Project) {
    let before = project.load(1).unwrap();
    let preview = redactions::preview_confirmation(
        project,
        &before,
        &[MaskSpec {
            region: region(0.2, 0.25, 0.5, 0.8),
            reason: Reason::AccountInformation,
        }],
        &Control::default(),
    )
    .unwrap();
    let mut after = before.clone();
    after.confirmed_redactions = redactions::confirm(
        project,
        &before,
        &preview,
        &preview.token,
        &[0],
        &Control::default(),
    )
    .unwrap();
    after.edited = true;
    after.reviewed = false;
    review::commit(project, &before, &after, "Confirm explicit source mask").unwrap();
}
fn bytes(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Vec<u8> {
    let mut value = vec![];
    zip.by_name(name).unwrap().read_to_end(&mut value).unwrap();
    value
}

#[test]
fn confirmation_is_explicit_source_bound_and_undoable_without_changing_originals() {
    let root = tempfile::tempdir().unwrap();
    let project = fixture(root.path());
    let original = fs::read(project.image(1)).unwrap();
    let before = project.load(1).unwrap();
    let proposal = MaskSpec {
        region: region(0.2, 0.25, 0.5, 0.8),
        reason: Reason::Other,
    };
    let preview =
        redactions::preview_confirmation(&project, &before, &[proposal], &Control::default())
            .unwrap();
    assert!(project.load(1).unwrap().confirmed_redactions.is_empty());
    assert!(
        redactions::confirm(
            &project,
            &before,
            &preview,
            "wrong",
            &[0],
            &Control::default()
        )
        .is_err()
    );
    assert!(
        redactions::confirm(
            &project,
            &before,
            &preview,
            &preview.token,
            &[],
            &Control::default()
        )
        .is_err()
    );
    assert!(
        redactions::confirm(
            &project,
            &before,
            &preview,
            &preview.token,
            &[0, 0],
            &Control::default()
        )
        .is_err()
    );
    let mut after = before.clone();
    after.confirmed_redactions = redactions::confirm(
        &project,
        &before,
        &preview,
        &preview.token,
        &[0],
        &Control::default(),
    )
    .unwrap();
    after.edited = true;
    review::commit(&project, &before, &after, "Confirm mask").unwrap();
    assert_eq!(project.load(1).unwrap().confirmed_redactions.len(), 1);
    assert!(
        project
            .load(1)
            .unwrap()
            .content
            .plain()
            .contains("secret-user-name")
    );
    review::undo(&project).unwrap();
    assert!(project.load(1).unwrap().confirmed_redactions.is_empty());
    review::redo(&project).unwrap();
    assert_eq!(project.load(1).unwrap().confirmed_redactions.len(), 1);
    assert_eq!(fs::read(project.image(1)).unwrap(), original);
}

#[test]
fn changed_page_source_or_tampered_mask_cannot_reuse_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let project = fixture(root.path());
    let before = project.load(1).unwrap();
    let preview = redactions::preview_confirmation(
        &project,
        &before,
        &[MaskSpec {
            region: BoxRect::full(),
            reason: Reason::Other,
        }],
        &Control::default(),
    )
    .unwrap();
    let mut tampered = preview.clone();
    tampered.proposals[0].region.right = 0.5;
    assert!(
        redactions::confirm(
            &project,
            &before,
            &tampered,
            &preview.token,
            &[0],
            &Control::default()
        )
        .is_err()
    );
    let mut changed = before.clone();
    changed.content.blocks[0].text.push('!');
    project.save(&changed).unwrap();
    assert!(
        redactions::confirm(
            &project,
            &before,
            &preview,
            &preview.token,
            &[0],
            &Control::default()
        )
        .is_err()
    );
    project.save(&before).unwrap();
    let mut image = image::open(project.image(1)).unwrap().to_rgb8();
    image[(0, 0)] = Rgb([1, 2, 3]);
    image.save(project.image(1)).unwrap();
    assert!(
        redactions::confirm(
            &project,
            &before,
            &preview,
            &preview.token,
            &[0],
            &Control::default()
        )
        .is_err()
    );
    add_mask(&project);
    let page = project.load(1).unwrap();
    let mut bad = page.confirmed_redactions.clone();
    bad[0].source.page = 2;
    assert!(redactions::validate_source(&bad, &page.confirmed_redactions[0].source).is_err());
    bad[0].region.right = f32::INFINITY;
    assert!(redactions::validate_metadata(&bad).is_err());
    let mut json = serde_json::to_value(&page.confirmed_redactions[0]).unwrap();
    json["hidden_text"] = "never store inferred hidden writing".into();
    assert!(serde_json::from_value::<redactions::ConfirmedMask>(json).is_err());
    image[(0, 0)] = Rgb([4, 5, 6]);
    image.save(project.image(1)).unwrap();
    assert!(redactions::effective_page(&project, &page, &Control::default()).is_err());
}

#[test]
fn effective_reading_withholds_whole_tables_and_ungrounded_content_without_guessing() {
    let root = tempfile::tempdir().unwrap();
    let project = fixture(root.path());
    add_mask(&project);
    let page = project.load(1).unwrap();
    let (effective, image, report) =
        redactions::effective_page(&project, &page, &Control::default()).unwrap();
    assert_eq!(report.withheld_blocks, 3);
    assert_eq!(report.ungrounded_blocks, 1);
    assert_eq!(report.retained_blocks, 2);
    assert_eq!(report.placeholders, 1);
    assert_eq!(effective.content.blocks.len(), 3);
    assert_eq!(effective.content.blocks[0].text, "Public heading Ω");
    assert_eq!(effective.content.blocks[1].kind, "figure");
    assert_eq!(effective.content.blocks[2].text, "[redacted]");
    assert_eq!(
        effective.content.blocks[2].bbox,
        Some(region(0.2, 0.25, 0.5, 0.8))
    );
    assert!(effective.content.summary.is_empty());
    assert!(
        effective.candidates.is_empty()
            && effective.manual_layers.is_empty()
            && effective.source_warnings.is_empty()
    );
    let json = serde_json::to_string(&effective.content).unwrap();
    for secret in [
        "secret-user-name",
        "private-1250",
        "Ungrounded private note",
        "PRIVATE SUMMARY",
        "PRIVATE WARNING",
    ] {
        assert!(!json.contains(secret));
    }
    let pixels = image.to_rgb8();
    assert_eq!(pixels[(20, 20)], Rgb([0, 0, 0]));
    assert_eq!(pixels[(49, 63)], Rgb([0, 0, 0]));
    assert_eq!(pixels[(19, 20)], Rgb([255, 255, 255]));
    assert!(
        project
            .load(1)
            .unwrap()
            .content
            .plain()
            .contains("private-1250")
    );
    assert_eq!(page.content.blocks.len(), 5);
    let safe =
        redactions::recognition_settings(&project.manifest.settings, &page.confirmed_redactions)
            .unwrap();
    assert!(
        safe.subject.is_empty()
            && safe.assistance.glossary.is_empty()
            && safe.assistance.sources.is_empty()
    );
    assert!(
        !safe.assistance.ask_questions && !safe.assistance.web_lookup && !safe.writer.use_examples
    );
    assert!(safe.writer_reference.is_none());
    assert!(project.manifest.settings.subject.contains("PRIVATE"));
}

#[test]
fn no_confirmed_masks_preserve_readings_and_limit_overflow_fails_closed() {
    let identity = redactions::SourceIdentity {
        page: 1,
        sha256: "a".repeat(64),
        width: 100,
        height: 100,
    };
    let content = PageContent {
        blocks: vec![Block::text("Unconfirmed proposals cannot remove this")],
        warnings: vec!["Original warning".into()],
        summary: "Original summary".into(),
    };
    let (same, _) =
        redactions::enforce_view(&content, &identity, &[], None, &Control::default()).unwrap();
    assert_eq!(
        serde_json::to_value(&same).unwrap(),
        serde_json::to_value(&content).unwrap()
    );
    let mask = redactions::ConfirmedMask {
        id: "b".repeat(64),
        source: identity.clone(),
        region: region(0.5, 0.5, 1., 1.),
        reason: Reason::Other,
    };
    let visible = Block {
        bbox: Some(region(0., 0., 0.2, 0.2)),
        ..Block::text("Visible text")
    };
    let full = PageContent {
        blocks: vec![visible; 500],
        ..Default::default()
    };
    assert!(
        redactions::enforce_view(
            &full,
            &identity,
            std::slice::from_ref(&mask),
            None,
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(full.blocks.len(), 500);
    assert!(
        redactions::validate_metadata(&vec![mask; redactions::MAX_MASKS_PER_PAGE + 1]).is_err()
    );
}

#[test]
fn cropped_masks_flatten_transparency_and_enforce_local_boxes() {
    let identity = redactions::SourceIdentity {
        page: 1,
        sha256: "a".repeat(64),
        width: 100,
        height: 100,
    };
    let masks = vec![redactions::ConfirmedMask {
        id: "b".repeat(64),
        source: identity.clone(),
        region: region(0.4, 0.4, 0.6, 0.6),
        reason: Reason::Other,
    }];
    let view = region(0.25, 0.25, 0.75, 0.75);
    let crop = DynamicImage::ImageRgba8(RgbaImage::from_pixel(50, 50, Rgba([17, 66, 100, 0])));
    let result = redactions::mask_view(&crop, &identity, &masks, Some(&view), &Control::default())
        .unwrap()
        .to_rgb8();
    assert_eq!(result[(0, 0)], Rgb([255, 255, 255]));
    assert_eq!(result[(20, 20)], Rgb([0, 0, 0]));
    assert_eq!(result[(40, 40)], Rgb([255, 255, 255]));
    let content = PageContent {
        blocks: vec![
            Block {
                bbox: Some(region(0.4, 0.4, 0.6, 0.6)),
                ..Block::text("Do not guess this")
            },
            Block::text("Ungrounded bypass"),
        ],
        ..Default::default()
    };
    let (clean, report) = redactions::enforce_view(
        &content,
        &identity,
        &masks,
        Some(&view),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(report.withheld_blocks, 2);
    assert_eq!(clean.blocks.len(), 1);
    assert_eq!(clean.blocks[0].text, "[redacted]");
    let bounds = clean.blocks[0].bbox.as_ref().unwrap();
    assert!((bounds.left - 0.3).abs() < 0.00001 && (bounds.right - 0.7).abs() < 0.00001);
    assert!(redactions::mask_view(&crop, &identity, &masks, None, &Control::default()).is_err());
}

#[test]
fn opaque_cover_proposals_never_confirm_or_classify_a_figure() {
    let mut image = RgbImage::from_pixel(200, 120, Rgb([255, 255, 255]));
    for y in 30..45 {
        for x in 25..155 {
            image[(x, y)] = Rgb([0, 0, 0]);
        }
    }
    let image = DynamicImage::ImageRgb8(image);
    let before = image.clone();
    let suggestions = redactions::suggest_opaque_covers(&image, &Control::default()).unwrap();
    assert_eq!(suggestions.len(), 1);
    assert!(suggestions[0].note.contains("legitimate figure"));
    assert!(suggestions[0].note.contains("explicitly confirm"));
    assert!(suggestions[0].region.valid());
    assert_eq!(image, before);
    let transparent = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 120, Rgba([0, 0, 0, 0])));
    assert!(
        redactions::suggest_opaque_covers(&transparent, &Control::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sharing_transaction_has_masked_pixels_public_text_and_no_private_sidecars_or_settings() {
    let root = tempfile::tempdir().unwrap();
    let project = fixture(root.path());
    add_mask(&project);
    let original = fs::read(project.image(1)).unwrap();
    let original_page = fingerprint(&project.load(1).unwrap()).unwrap();
    let preview = redactions::preview_share(&project, &[1], &Control::default()).unwrap();
    assert_eq!(preview.pages[0].withheld_blocks, 3);
    let output = root.path().join("public.zip");
    let receipt = redactions::export_share(
        &project,
        &preview,
        &preview.token,
        &output,
        &Control::default(),
    )
    .unwrap();
    assert_eq!(receipt.pages, 1);
    assert_eq!(receipt.masks, 1);
    assert!(receipt.checked_private_fragments >= 4);
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let names = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "project/project.json",
            "project/page-00001.json",
            "project/page-00001.png",
            "shared.docx",
            "sharing-report.json"
        ]
    );
    let page_json = String::from_utf8(bytes(&mut zip, "project/page-00001.json")).unwrap();
    let shared: Page = serde_json::from_str(&page_json).unwrap();
    shared.validate().unwrap();
    assert!(shared.content.plain().contains("Public heading Ω"));
    assert!(shared.content.plain().contains("[redacted]"));
    assert!(shared.candidates.is_empty() && shared.manual_layers.is_empty());
    let manifest = String::from_utf8(bytes(&mut zip, "project/project.json")).unwrap();
    for text in [&page_json, &manifest] {
        for secret in [
            "secret-user-name",
            "private-1250",
            "PRIVATE",
            "private-project",
            "private-writer-name",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
    }
    assert!(!manifest.contains(&project.manifest.input_sha256));
    let png = bytes(&mut zip, "project/page-00001.png");
    let public_image = image::load_from_memory(&png).unwrap().to_rgb8();
    assert_eq!(public_image[(20, 20)], Rgb([0, 0, 0]));
    assert_eq!(public_image[(19, 20)], Rgb([255, 255, 255]));
    use sha2::{Digest, Sha256};
    assert_eq!(
        shared.confirmed_redactions[0].source.sha256,
        hex::encode(Sha256::digest(&png))
    );
    assert_ne!(
        shared.confirmed_redactions[0].source.sha256,
        preview.pages[0].source.sha256
    );
    let word = bytes(&mut zip, "shared.docx");
    let mut word = zip::ZipArchive::new(Cursor::new(word)).unwrap();
    for i in 0..word.len() {
        let mut entry = word.by_index(i).unwrap();
        if entry.name().ends_with(".xml") {
            let mut xml = String::new();
            entry.read_to_string(&mut xml).unwrap();
            assert!(!xml.contains("secret-user-name") && !xml.contains("private-1250"));
        }
    }
    assert_eq!(fs::read(project.image(1)).unwrap(), original);
    assert_eq!(
        fingerprint(&project.load(1).unwrap()).unwrap(),
        original_page
    );
    assert!(project.root.join("responses/reader.txt").is_file());
    assert!(project.root.join("guidance.json").is_file());
    assert!(
        redactions::export_share(
            &project,
            &preview,
            &preview.token,
            &output,
            &Control::default()
        )
        .is_err()
    );
}

#[test]
fn sharing_rejects_stale_tampered_cancelled_and_leaking_copies_without_publication() {
    let root = tempfile::tempdir().unwrap();
    let project = fixture(root.path());
    add_mask(&project);
    let preview = redactions::preview_share(&project, &[1], &Control::default()).unwrap();
    let output = root.path().join("never-published.zip");
    let mut tampered = preview.clone();
    tampered.pages[0].withheld_blocks = 0;
    assert!(
        redactions::export_share(
            &project,
            &tampered,
            &preview.token,
            &output,
            &Control::default()
        )
        .is_err()
    );
    let cancelled = Control::default();
    cancelled.cancelled.store(true, Ordering::Relaxed);
    assert!(
        redactions::export_share(&project, &preview, &preview.token, &output, &cancelled).is_err()
    );
    assert!(
        redactions::export_share(
            &project,
            &preview,
            &preview.token,
            &project.root.join("unsafe.zip"),
            &Control::default()
        )
        .is_err()
    );
    let mut changed = project.load(1).unwrap();
    changed.content.blocks[0].text = "Public heading mentions secret-user-name".into();
    project.save(&changed).unwrap();
    assert!(
        redactions::export_share(
            &project,
            &preview,
            &preview.token,
            &output,
            &Control::default()
        )
        .is_err()
    );
    let fresh = redactions::preview_share(&project, &[1], &Control::default()).unwrap();
    let error =
        redactions::export_share(&project, &fresh, &fresh.token, &output, &Control::default())
            .unwrap_err()
            .to_string();
    assert!(error.contains("leak check"));
    assert!(!error.contains("secret-user-name"));
    assert!(!output.exists());
    assert!(!fs::read_dir(root.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".notegen-share-")
    }));
}

#[cfg(unix)]
#[test]
fn sharing_rejects_source_and_destination_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let project = fixture(root.path());
    add_mask(&project);
    let preview = redactions::preview_share(&project, &[1], &Control::default()).unwrap();
    let elsewhere = root.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let link = root.path().join("linked");
    std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
    assert!(
        redactions::export_share(
            &project,
            &preview,
            &preview.token,
            &link.join("copy.zip"),
            &Control::default()
        )
        .is_err()
    );
    let original = root.path().join("original.png");
    fs::rename(project.image(1), &original).unwrap();
    std::os::unix::fs::symlink(&original, project.image(1)).unwrap();
    assert!(redactions::preview_share(&project, &[1], &Control::default()).is_err());
    assert!(original.is_file());
}
