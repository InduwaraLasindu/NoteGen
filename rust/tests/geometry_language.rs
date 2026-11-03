use notegen::{
    geometry::{self, BlockGeometry, Origin, Point, Quad, TableGrid, WordBox},
    geometry_repair::{self, CellMove, GridRepair},
    language_routing::{self, RegionLayout},
    review, scan,
    storage::{Control, Manifest, Project, fingerprint},
    types::*,
};
use std::{fs, path::Path, sync::atomic::Ordering};
fn fixture(table: bool) -> (tempfile::TempDir, Project) {
    let temp = tempfile::tempdir().unwrap();
    let project = Project::open(
        temp.path().join("project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    image::RgbImage::from_pixel(40, 40, image::Rgb([210, 30, 60]))
        .save(project.image(1))
        .unwrap();
    let block = if table {
        Block {
            kind: "table".into(),
            rows: vec![vec!["A".into(), "B".into()], vec!["C".into(), "D".into()]],
            bbox: Some(BoxRect::full()),
            ..Default::default()
        }
    } else {
        Block {
            bbox: Some(BoxRect::full()),
            ..Block::text("Original paragraph")
        }
    };
    project
        .save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks: vec![block],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    (temp, project)
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.000_001, "{a} differs from {b}");
}
#[test]
fn fine_heatmap_uses_supplied_boxes_and_only_different_families_establish_word_disagreement() {
    let (_temp, project) = fixture(false);
    let mut page = project.load(1).unwrap();
    let source = geometry::source_identity(&project.image(1), 1, &Control::default()).unwrap();
    let content = |text: &str, confidence: f32| PageContent {
        blocks: vec![Block {
            geometry: Some(BlockGeometry {
                source: Some(source.clone()),
                words: vec![WordBox {
                    text: text.into(),
                    region: BoxRect {
                        left: 0.1,
                        top: 0.1,
                        right: 0.3,
                        bottom: 0.2,
                    },
                    confidence: Some(confidence),
                    line: None,
                    cell: None,
                    origin: Origin::ReaderSupplied,
                }],
                ..Default::default()
            }),
            ..Block::text(text)
        }],
        ..Default::default()
    };
    page.candidates = vec![
        Candidate {
            model_id: "first".into(),
            family: "same".into(),
            view: "full".into(),
            region: None,
            reliability: 0.65,
            content: content("1250", 0.95),
        },
        Candidate {
            model_id: "same-reader-crop".into(),
            family: "same".into(),
            view: "crop".into(),
            region: None,
            reliability: 0.65,
            content: content("1950", 0.95),
        },
    ];
    assert!(
        geometry::heatmap(&project, &page, 0.6, &Control::default())
            .unwrap()
            .spots
            .is_empty()
    );
    page.candidates.push(Candidate {
        model_id: "independent".into(),
        family: "different".into(),
        view: "full".into(),
        region: None,
        reliability: 0.65,
        content: content("125O", 0.2),
    });
    let report = geometry::heatmap(&project, &page, 0.6, &Control::default()).unwrap();
    assert!(
        report
            .spots
            .iter()
            .any(|spot| spot.kind == geometry::HeatKind::WordDisagreement)
    );
    assert!(
        report
            .spots
            .iter()
            .any(|spot| spot.kind == geometry::HeatKind::LowWordConfidence)
    );
    assert!(report.spots.iter().all(|spot| spot.region
        == BoxRect {
            left: 0.1,
            top: 0.1,
            right: 0.3,
            bottom: 0.2
        }));
    assert_eq!(report.missing_precision_blocks, 1);
    page.candidates.clear();
    assert!(
        geometry::heatmap(&project, &page, 0.6, &Control::default())
            .unwrap()
            .spots
            .is_empty()
    );
}
#[test]
fn legacy_metadata_stays_absent_and_observed_crop_boxes_map_to_exact_global_frame() {
    let (_temp, project) = fixture(false);
    let before = fs::read(project.page_path(1)).unwrap();
    let old = project.load(1).unwrap();
    assert!(
        old.script_detection.is_none()
            && old.region_languages.is_empty()
            && old.content.blocks[0].geometry.is_none()
    );
    assert!(!String::from_utf8(before).unwrap().contains("geometry"));
    let identity = geometry::source_identity(&project.image(1), 1, &Control::default()).unwrap();
    let mut content = PageContent {
        blocks: vec![Block {
            bbox: Some(BoxRect::full()),
            geometry: Some(BlockGeometry {
                words: vec![WordBox {
                    text: "1250".into(),
                    region: BoxRect {
                        left: 0.1,
                        top: 0.2,
                        right: 0.3,
                        bottom: 0.4,
                    },
                    confidence: Some(0.75),
                    line: None,
                    cell: None,
                    origin: Origin::ReaderSupplied,
                }],
                ..Default::default()
            }),
            ..Block::text("1250")
        }],
        ..Default::default()
    };
    geometry::map_content(
        &mut content,
        &BoxRect {
            left: 0.25,
            top: 0.5,
            right: 0.75,
            bottom: 1.,
        },
        &identity,
    )
    .unwrap();
    let word = &content.blocks[0].geometry.as_ref().unwrap().words[0];
    close(word.region.left, 0.3);
    close(word.region.top, 0.6);
    close(word.region.right, 0.4);
    close(word.region.bottom, 0.7);
    assert_eq!(word.text, "1250");
    assert_eq!(word.confidence, Some(0.75));
    let mut page = old;
    page.content = content;
    page.validate().unwrap();
    geometry::verify_metadata_source(&project, &page, &Control::default()).unwrap();
    page.content.blocks[0].geometry.as_mut().unwrap().words[0].confidence = Some(f32::NAN);
    assert!(page.validate().is_err());
    page.content.blocks[0].geometry.as_mut().unwrap().words[0].confidence = Some(0.75);
    page.content.blocks[0].geometry.as_mut().unwrap().source = None;
    assert!(page.validate().is_err());
    page.content.blocks[0].geometry.as_mut().unwrap().source = Some(identity);
    image::RgbImage::from_pixel(40, 40, image::Rgb([0, 255, 0]))
        .save(project.image(1))
        .unwrap();
    assert!(geometry::verify_metadata_source(&project, &page, &Control::default()).is_err());
}
#[test]
fn osd_scores_remain_raw_and_human_mixed_language_corrections_are_source_bound_and_undoable() {
    let parsed=language_routing::parse_osd("Orientation in degrees: 0\nRotate: 0\nOrientation confidence: 22.50\nScript: Latin\nScript confidence: 4.25\n").unwrap();
    assert_eq!(parsed.script.as_deref(), Some("Latin"));
    assert_eq!(parsed.script_confidence, Some(4.25));
    assert_eq!(parsed.suggested_languages, vec!["eng"]);
    assert!(parsed.note.contains("not probabilities"));
    assert!(parsed.note.contains("many languages"));
    for bad in [
        "Script: Latin\nScript confidence: NaN",
        "Script: Latin\nScript: Tamil",
        "Rotate: 45",
    ] {
        assert!(language_routing::parse_osd(bad).is_err());
    }
    let (_temp, project) = fixture(false);
    let before = project.load(1).unwrap();
    let source = fs::read(project.image(1)).unwrap();
    let selection = language_routing::select_region(
        &project,
        &before,
        BoxRect::full(),
        "eng+sin",
        RegionLayout::Paragraph,
        Some(0),
        &Control::default(),
    )
    .unwrap();
    language_routing::commit_selections(&project, &before, vec![selection], &Control::default())
        .unwrap();
    let corrected = project.load(1).unwrap();
    assert_eq!(corrected.region_languages[0].languages, "eng+sin");
    assert_eq!(corrected.content.plain(), before.content.plain());
    review::undo(&project).unwrap();
    assert!(project.load(1).unwrap().region_languages.is_empty());
    review::redo(&project).unwrap();
    assert_eq!(project.load(1).unwrap().region_languages.len(), 1);
    assert_eq!(fs::read(project.image(1)).unwrap(), source);
    image::RgbImage::from_pixel(40, 40, image::Rgb([0, 0, 0]))
        .save(project.image(1))
        .unwrap();
    assert!(
        language_routing::commit_selections(
            &project,
            &corrected,
            corrected.region_languages.clone(),
            &Control::default()
        )
        .is_err()
    );
}
#[test]
fn slanted_line_repair_is_undoable_and_reader_view_whitens_other_lines_but_keeps_masks_opaque() {
    let (_temp, project) = fixture(false);
    let original = fs::read(project.image(1)).unwrap();
    let before = project.load(1).unwrap();
    let shape = Quad {
        points: [
            Point { x: 0.1, y: 0.1 },
            Point { x: 0.9, y: 0.3 },
            Point { x: 0.9, y: 0.6 },
            Point { x: 0.1, y: 0.4 },
        ],
    };
    geometry_repair::repair_lines(&project, &before, 0, &[shape], &Control::default()).unwrap();
    let repaired = project.load(1).unwrap();
    assert_eq!(repaired.content.plain(), before.content.plain());
    assert_eq!(
        repaired.content.blocks[0].geometry.as_ref().unwrap().lines[0].origin,
        Origin::HumanConfirmed
    );
    review::undo(&project).unwrap();
    assert!(
        project.load(1).unwrap().content.blocks[0]
            .geometry
            .is_none()
    );
    review::redo(&project).unwrap();
    let mut page = project.load(1).unwrap();
    let proposal = notegen::redactions::MaskSpec {
        region: BoxRect {
            left: 0.45,
            top: 0.3,
            right: 0.6,
            bottom: 0.5,
        },
        reason: notegen::redactions::Reason::PersonalInformation,
    };
    let preview = notegen::redactions::preview_confirmation(
        &project,
        &page,
        &[proposal],
        &Control::default(),
    )
    .unwrap();
    page.confirmed_redactions = notegen::redactions::confirm(
        &project,
        &page,
        &preview,
        &preview.token,
        &[0],
        &Control::default(),
    )
    .unwrap();
    project.save(&page).unwrap();
    let view = geometry_repair::line_view(&project, &page, 0, 0, &Control::default()).unwrap();
    let rgb = view.image.to_rgb8();
    assert_eq!(rgb[(31, 0)].0, [255, 255, 255]);
    assert_eq!(rgb[(6, 8)].0, [210, 30, 60]);
    assert_eq!(rgb[(16, 10)].0, [0, 0, 0]);
    assert_eq!(rgb.dimensions(), (32, 20));
    close(view.region.left, 0.1);
    close(view.region.top, 0.1);
    close(view.region.right, 0.9);
    close(view.region.bottom, 0.6);
    assert_eq!(fs::read(project.image(1)).unwrap(), original);
    let mut crossing = Quad::rectangle(&BoxRect::full());
    crossing.points.swap(1, 2);
    assert!(crossing.validate().is_err());
    image::RgbImage::from_pixel(40, 40, image::Rgb([1, 2, 3]))
        .save(project.image(1))
        .unwrap();
    assert!(geometry_repair::line_view(&project, &page, 0, 0, &Control::default()).is_err());
}
fn mapping() -> Vec<CellMove> {
    vec![
        CellMove {
            from_row: 0,
            from_col: 0,
            to_row: 0,
            to_col: 0,
        },
        CellMove {
            from_row: 0,
            from_col: 1,
            to_row: 0,
            to_col: 2,
        },
        CellMove {
            from_row: 1,
            from_col: 0,
            to_row: 1,
            to_col: 0,
        },
        CellMove {
            from_row: 1,
            from_col: 1,
            to_row: 1,
            to_col: 2,
        },
    ]
}
#[test]
fn table_grid_split_and_explicit_combination_keep_every_value_and_reject_hidden_merged_cells() {
    let (_temp, project) = fixture(true);
    let before = project.load(1).unwrap();
    let image = fs::read(project.image(1)).unwrap();
    let repair = GridRepair {
        grid: TableGrid {
            row_edges: vec![0., 0.5, 1.],
            column_edges: vec![0., 0.3, 0.7, 1.],
            origin: Origin::ReaderSupplied,
        },
        mapping: mapping(),
        merges: vec![],
        header_rows: 1,
    };
    let preview =
        geometry_repair::preview_grid(&project, &before, 0, &repair, &Control::default()).unwrap();
    assert_eq!(
        preview.after.content.blocks[0].rows,
        vec![vec!["A", "", "B"], vec!["C", "", "D"]]
    );
    assert_eq!(
        preview.after.content.blocks[0]
            .geometry
            .as_ref()
            .unwrap()
            .cells
            .len(),
        6
    );
    geometry_repair::apply_grid(
        &project,
        &before,
        &preview,
        &preview.token,
        &Control::default(),
    )
    .unwrap();
    review::undo(&project).unwrap();
    assert_eq!(
        project.load(1).unwrap().content.blocks[0].rows,
        before.content.blocks[0].rows
    );
    review::redo(&project).unwrap();
    assert_eq!(
        project.load(1).unwrap().content.blocks[0].rows,
        preview.after.content.blocks[0].rows
    );
    let mut incomplete = repair.clone();
    incomplete.mapping.pop();
    assert!(
        geometry_repair::preview_grid(
            &project,
            &project.load(1).unwrap(),
            0,
            &incomplete,
            &Control::default()
        )
        .is_err()
    );
    // Return to the original exact grid before testing alternative repair plans.
    review::undo(&project).unwrap();
    let original = project.load(1).unwrap();
    let mut hiding = repair.clone();
    hiding.merges = vec![Merge {
        row: 0,
        col: 0,
        rowspan: 1,
        colspan: 3,
    }];
    assert!(
        geometry_repair::preview_grid(&project, &original, 0, &hiding, &Control::default())
            .is_err()
    );
    let combined = GridRepair {
        grid: TableGrid {
            row_edges: vec![0., 1.],
            column_edges: vec![0., 1.],
            origin: Origin::HumanConfirmed,
        },
        mapping: (0..2)
            .flat_map(|row| {
                (0..2).map(move |col| CellMove {
                    from_row: row,
                    from_col: col,
                    to_row: 0,
                    to_col: 0,
                })
            })
            .collect(),
        merges: vec![],
        header_rows: 0,
    };
    let combined =
        geometry_repair::preview_grid(&project, &original, 0, &combined, &Control::default())
            .unwrap();
    assert_eq!(
        combined.after.content.blocks[0].rows,
        vec![vec!["A\nB\nC\nD"]]
    );
    let mut tampered = combined.clone();
    tampered.after.content.blocks[0].rows[0][0] = "LOST VALUES".into();
    assert!(
        geometry_repair::apply_grid(
            &project,
            &original,
            &tampered,
            &combined.token,
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(project.image(1)).unwrap(), image);
    assert_eq!(
        fingerprint(&project.load(1).unwrap()).unwrap(),
        fingerprint(&original).unwrap()
    );
}
#[test]
fn real_native_script_detection_and_two_region_routing_override_the_default_resource_and_keep_global_words()
 {
    use ab_glyph::FontArc;
    let (_temp, project) = fixture(false);
    let font_path = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .expect("native fixture font");
    let font = FontArc::try_from_vec(fs::read(font_path).unwrap()).unwrap();
    let mut image = image::RgbImage::from_pixel(1400, 900, image::Rgb([255, 255, 255]));
    for (row, text) in [
        (40, "REGION ALPHA total 1250 and printed Latin text"),
        (170, "Compare original source before approving words"),
        (300, "Geometry preserves the actual observed letters"),
        (500, "REGION BETA total 9750 and printed Latin text"),
        (630, "Different paragraphs can select their resources"),
        (760, "Do not turn confidence into measured accuracy"),
    ] {
        imageproc::drawing::draw_text_mut(
            &mut image,
            image::Rgb([0, 0, 0]),
            45,
            row,
            42.,
            &font,
            text,
        );
    }
    image.save(project.image(1)).unwrap();
    let source = fs::read(project.image(1)).unwrap();
    let settings = Settings {
        languages: "tam".into(),
        auto_rotate: false,
        deskew: false,
        ..Default::default()
    };
    let detection = scan::detect_script(
        &image::DynamicImage::ImageRgb8(image),
        &settings,
        &Control::default(),
    )
    .unwrap();
    assert_eq!(detection.script.as_deref(), Some("Latin"));
    assert!(detection.script_confidence.is_some_and(f32::is_finite));
    let mut page = project.load(1).unwrap();
    page.content.blocks = vec![
        Block {
            bbox: Some(BoxRect {
                left: 0.02,
                top: 0.02,
                right: 0.98,
                bottom: 0.48,
            }),
            ..Block::text("first old draft")
        },
        Block {
            bbox: Some(BoxRect {
                left: 0.02,
                top: 0.52,
                right: 0.98,
                bottom: 0.98,
            }),
            ..Block::text("second old draft")
        },
    ];
    project.save(&page).unwrap();
    let selected = page
        .content
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            language_routing::select_region(
                &project,
                &page,
                block.bbox.clone().unwrap(),
                "eng",
                RegionLayout::Paragraph,
                Some(index),
                &Control::default(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let preview = language_routing::reread_preview(
        &project,
        &page,
        &selected,
        &settings,
        &Control::default(),
    )
    .unwrap();
    assert!(preview.after.content.plain().contains("1250"));
    assert!(preview.after.content.plain().contains("9750"));
    assert_eq!(preview.after.candidates.len(), 2);
    let expected_sha = geometry::source_identity(&project.image(1), 1, &Control::default())
        .unwrap()
        .sha256;
    for (index, candidate) in preview.after.candidates.iter().enumerate() {
        assert_eq!(candidate.model_id, "tesseract:eng");
        let words = candidate
            .content
            .blocks
            .iter()
            .filter_map(|block| block.geometry.as_ref())
            .flat_map(|data| data.words.iter())
            .collect::<Vec<_>>();
        assert!(!words.is_empty());
        assert!(
            words
                .iter()
                .all(|word| word.region.top >= if index == 0 { 0.02 } else { 0.52 })
        );
        assert!(
            words
                .iter()
                .all(|word| word.region.bottom <= if index == 0 { 0.48 } else { 0.98 })
        );
        assert!(
            candidate
                .content
                .blocks
                .iter()
                .filter_map(|block| block.geometry.as_ref())
                .all(|data| data.source.as_ref().unwrap().sha256 == expected_sha)
        );
    }
    language_routing::apply_preview(
        &project,
        &page,
        &preview,
        &preview.token,
        &Control::default(),
    )
    .unwrap();
    review::undo(&project).unwrap();
    assert_eq!(
        project.load(1).unwrap().content.plain(),
        page.content.plain()
    );
    let cancelled = Control::default();
    cancelled.cancelled.store(true, Ordering::Relaxed);
    let current = project.load(1).unwrap();
    assert!(
        language_routing::reread_preview(&project, &current, &selected, &settings, &cancelled)
            .is_err()
    );
    assert_eq!(fs::read(project.image(1)).unwrap(), source);
    assert_eq!(
        fingerprint(&project.load(1).unwrap()).unwrap(),
        fingerprint(&current).unwrap()
    );
    let mut masked = current.clone();
    let proposals = [
        notegen::redactions::MaskSpec {
            region: selected[0].region.clone(),
            reason: notegen::redactions::Reason::PersonalInformation,
        },
        notegen::redactions::MaskSpec {
            region: BoxRect {
                left: 0.2,
                top: 0.6,
                right: 0.3,
                bottom: 0.7,
            },
            reason: notegen::redactions::Reason::PersonalInformation,
        },
    ];
    let confirmation = notegen::redactions::preview_confirmation(
        &project,
        &masked,
        &proposals,
        &Control::default(),
    )
    .unwrap();
    masked.confirmed_redactions = notegen::redactions::confirm(
        &project,
        &masked,
        &confirmation,
        &confirmation.token,
        &[0, 1],
        &Control::default(),
    )
    .unwrap();
    masked.edited = true;
    project.save(&masked).unwrap();
    let hidden = language_routing::reread_preview(
        &project,
        &masked,
        &selected[..1],
        &settings,
        &Control::default(),
    )
    .unwrap();
    let candidate = hidden.after.candidates.last().unwrap();
    let view = candidate.region.as_ref().unwrap();
    assert!(!candidate.content.plain().contains("1250"));
    assert!(!candidate.content.plain().contains("9750"));
    assert!(candidate.content.blocks.iter().all(|block| {
        block.geometry.is_none()
            && block.alternatives.is_empty()
            && block.bbox.as_ref().is_some_and(|region| {
                region.left >= view.left
                    && region.top >= view.top
                    && region.right <= view.right
                    && region.bottom <= view.bottom
            })
    }));
    assert_eq!(fs::read(project.image(1)).unwrap(), source);
}
