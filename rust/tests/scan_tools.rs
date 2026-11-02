use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};
use notegen::{
    scan_tools::{self as tools, CleanupOptions, Point},
    storage::Control,
    types::BoxRect,
};
use std::{fs, sync::atomic::Ordering};
fn corners() -> [Point; 4] {
    [
        Point { x: 0., y: 0. },
        Point { x: 1., y: 0. },
        Point { x: 1., y: 1. },
        Point { x: 0., y: 1. },
    ]
}
fn gradient(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            (x * 17 % 256) as u8,
            (y * 31 % 256) as u8,
            ((x + y) * 11 % 256) as u8,
        ])
    }))
}

#[test]
fn identity_perspective_preserves_every_pixel_and_source_is_never_modified() {
    let original = gradient(17, 13);
    let before = original.to_rgb8();
    let output = tools::perspective(&original, corners(), 17, 13, &Control::default()).unwrap();
    assert_eq!(output.to_rgb8(), before);
    assert_eq!(original.to_rgb8(), before);
}
#[test]
fn perspective_crop_and_trapezoid_map_exact_source_corners() {
    let original = gradient(11, 11);
    let selected = [
        Point { x: 0.2, y: 0.2 },
        Point { x: 0.8, y: 0.2 },
        Point { x: 0.8, y: 0.8 },
        Point { x: 0.2, y: 0.8 },
    ];
    let output = tools::perspective(&original, selected, 7, 7, &Control::default())
        .unwrap()
        .to_rgb8();
    let source = original.to_rgb8();
    for y in 0..7 {
        for x in 0..7 {
            assert_eq!(output[(x, y)], source[(x + 2, y + 2)]);
        }
    }
    let selected = [
        Point { x: 0.2, y: 0. },
        Point { x: 0.8, y: 0. },
        Point { x: 1., y: 1. },
        Point { x: 0., y: 1. },
    ];
    let output = tools::perspective(&original, selected, 11, 11, &Control::default())
        .unwrap()
        .to_rgb8();
    assert_eq!(output[(0, 0)], source[(2, 0)]);
    assert_eq!(output[(10, 0)], source[(8, 0)]);
    assert_eq!(output[(10, 10)], source[(10, 10)]);
    assert_eq!(output[(0, 10)], source[(0, 10)]);
}
#[test]
fn perspective_rejects_self_intersection_concavity_nonfinite_or_excessive_output() {
    let image = gradient(10, 10);
    let c = Control::default();
    let mut crossed = corners();
    crossed.swap(1, 2);
    assert!(tools::perspective(&image, crossed, 10, 10, &c).is_err());
    let mut concave = corners();
    concave[2] = Point { x: 0.2, y: 0.2 };
    assert!(tools::perspective(&image, concave, 10, 10, &c).is_err());
    let mut invalid = corners();
    invalid[0].x = f64::NAN;
    assert!(tools::perspective(&image, invalid, 10, 10, &c).is_err());
    invalid[0].x = -0.1;
    assert!(tools::perspective(&image, invalid, 10, 10, &c).is_err());
    assert!(tools::perspective(&image, corners(), 1, 10, &c).is_err());
    assert!(tools::perspective(&image, corners(), 20000, 20000, &c).is_err());
}
#[test]
fn lighting_variant_reduces_uneven_paper_brightness_and_preserves_black_ink() {
    let mut source = RgbImage::from_fn(256, 128, |x, _| Rgb([if x < 128 { 140 } else { 230 }; 3]));
    for y in 40..88 {
        for x in 20..30 {
            source[(x, y)] = Rgb([0; 3]);
        }
        for x in 200..210 {
            source[(x, y)] = Rgb([0; 3]);
        }
    }
    let image = DynamicImage::ImageRgb8(source.clone());
    let comparison =
        tools::compare(&image, &CleanupOptions::default(), &Control::default()).unwrap();
    assert_eq!(comparison.variants.len(), 2);
    assert_eq!(comparison.variants[0].image.to_rgb8(), source);
    let normalized = comparison.variants[1].image.to_rgb8();
    let before_gap = source[(32, 10)][0].abs_diff(source[(224, 10)][0]);
    let after_gap = normalized[(32, 10)][0].abs_diff(normalized[(224, 10)][0]);
    assert!(after_gap < before_gap);
    assert_eq!(normalized[(25, 50)], Rgb([0; 3]));
    assert_eq!(normalized[(205, 50)], Rgb([0; 3]));
    assert_eq!(image.to_rgb8(), source);
    assert!(comparison.variants[1].report.changed_pixels > 0);
}
#[test]
fn optional_pale_grid_suppression_preserves_dark_strokes_and_dark_grid_lines() {
    let mut source = RgbImage::from_pixel(100, 100, Rgb([255; 3]));
    for x in 0..100 {
        source[(x, 50)] = Rgb([190; 3]);
        source[(x, 80)] = Rgb([0; 3]);
    }
    for y in 0..100 {
        source[(60, y)] = Rgb([190; 3]);
    }
    for y in 45..56 {
        for x in 45..56 {
            source[(x, y)] = Rgb([0; 3]);
        }
    }
    let comparison = tools::compare(
        &DynamicImage::ImageRgb8(source.clone()),
        &CleanupOptions {
            shadow_variant: false,
            pale_ruling_variant: true,
            ..Default::default()
        },
        &Control::default(),
    )
    .unwrap();
    let cleaned = comparison.variants[1].image.to_rgb8();
    assert_eq!(cleaned[(5, 50)], Rgb([255; 3]));
    assert_eq!(cleaned[(60, 5)], Rgb([255; 3]));
    assert_eq!(cleaned[(50, 50)], Rgb([0; 3]));
    assert_eq!(cleaned[(43, 50)], source[(43, 50)]);
    assert_eq!(cleaned[(5, 80)], Rgb([0; 3]));
    assert_eq!(
        comparison.variants[1].report.dark_ink_before,
        comparison.variants[1].report.dark_ink_after
    );
    assert!(
        comparison.variants[1]
            .report
            .warnings
            .iter()
            .any(|s| s.contains("diagram lines"))
    );
}
#[test]
fn comparison_retains_alpha_original_exports_independent_variants_and_protects_existing_folder() {
    let dir = tempfile::tempdir().unwrap();
    let original = DynamicImage::ImageRgba8(RgbaImage::from_pixel(20, 20, Rgba([20, 30, 40, 100])));
    let comparison = tools::compare(
        &original,
        &CleanupOptions {
            shadow_variant: false,
            ..Default::default()
        },
        &Control::default(),
    )
    .unwrap();
    assert_eq!(comparison.variants[0].image.to_rgba8(), original.to_rgba8());
    let output = dir.path().join("comparison");
    let manifest = tools::export_comparison(&comparison, &output, &Control::default()).unwrap();
    assert_eq!(manifest.files, vec!["files/original.png"]);
    let saved = image::open(output.join("files/original.png")).unwrap();
    assert_eq!(saved.to_rgba8(), original.to_rgba8());
    assert!(output.join("files/comparison.json").is_file());
    assert!(tools::export_comparison(&comparison, &output, &Control::default()).is_err());
}
#[test]
fn zero_width_gutter_preserves_every_pixel_across_odd_width_double_page_split() {
    let source = gradient(101, 19);
    let split = tools::split_double(&source, 0.5, 0.5, &Control::default()).unwrap();
    assert_eq!(split.left.width(), 50);
    assert_eq!(split.right.width(), 51);
    assert_eq!(
        split.relationship.gutter_start,
        split.relationship.gutter_end
    );
    let left = split.left.to_rgb8();
    let right = split.right.to_rgb8();
    let original = source.to_rgb8();
    for y in 0..19 {
        for x in 0..101 {
            assert_eq!(
                original[(x, y)],
                if x < 50 {
                    left[(x, y)]
                } else {
                    right[(x - 50, y)]
                }
            );
        }
    }
    assert_eq!(split.original.to_rgb8(), original);
}
#[test]
fn explicit_gutter_crops_have_source_relationship_and_original_is_exported() {
    let dir = tempfile::tempdir().unwrap();
    let source = gradient(100, 20);
    let split = tools::split_double(&source, 0.45, 0.55, &Control::default()).unwrap();
    assert_eq!(split.left.width(), 45);
    assert_eq!(split.right.width(), 45);
    assert_eq!(
        (
            split.relationship.gutter_start,
            split.relationship.gutter_end
        ),
        (45, 55)
    );
    assert!((split.relationship.right_source.left - 0.55).abs() < 1e-6);
    let output = dir.path().join("split");
    tools::export_split(&split, &output, &Control::default()).unwrap();
    for name in ["original.png", "left.png", "right.png", "relationship.json"] {
        assert!(output.join("files").join(name).is_file());
    }
    assert_eq!(
        image::open(output.join("files/original.png"))
            .unwrap()
            .to_rgb8(),
        source.to_rgb8()
    );
    assert!(tools::split_double(&source, 0., 0.5, &Control::default()).is_err());
    assert!(tools::split_double(&source, 0.6, 0.4, &Control::default()).is_err());
    assert!(tools::split_double(&source, f32::NAN, 0.6, &Control::default()).is_err());
}
#[test]
fn redaction_is_opaque_and_counts_union_without_touching_unselected_pixels_or_source() {
    let original =
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([210, 180, 150, 30])));
    let regions = [
        BoxRect {
            left: 0.,
            top: 0.,
            right: 0.5,
            bottom: 0.5,
        },
        BoxRect {
            left: 0.3,
            top: 0.3,
            right: 0.8,
            bottom: 0.8,
        },
    ];
    let (copy, report) = tools::redact(&original, &regions, &Control::default()).unwrap();
    assert_eq!(report.redacted_pixels, 46);
    let pixels = copy.to_rgba8();
    for y in 0..10 {
        for x in 0..10 {
            let selected = x < 5 && y < 5 || x >= 3 && x < 8 && y >= 3 && y < 8;
            assert_eq!(
                pixels[(x, y)],
                if selected {
                    Rgba([0, 0, 0, 255])
                } else {
                    Rgba([210, 180, 150, 255])
                }
            );
        }
    }
    assert_eq!(original.to_rgba8()[(0, 0)], Rgba([210, 180, 150, 30]));
    assert!(
        report
            .note
            .contains("Recognized text/project metadata require separate redaction")
    );
}
#[test]
fn redacted_png_copy_contains_only_replaced_pixels_and_never_overwrites_source_or_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = gradient(8, 8);
    let source_path = dir.path().join("source.png");
    source.save(&source_path).unwrap();
    let original_bytes = fs::read(&source_path).unwrap();
    let target = dir.path().join("redacted.png");
    let report =
        tools::export_redacted(&source, &[BoxRect::full()], &target, &Control::default()).unwrap();
    assert_eq!(report.redacted_pixels, 64);
    assert!(
        image::open(&target)
            .unwrap()
            .to_rgb8()
            .pixels()
            .all(|p| p.0 == [0; 3])
    );
    assert_eq!(fs::read(&source_path).unwrap(), original_bytes);
    assert!(
        tools::export_redacted(
            &source,
            &[BoxRect::full()],
            &source_path,
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&source_path).unwrap(), original_bytes);
}
#[test]
fn scan_tools_reject_invalid_boxes_options_and_cancellation_without_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let image = gradient(10, 10);
    let c = Control::default();
    assert!(tools::redact(&image, &[], &c).is_err());
    assert!(
        tools::redact(
            &image,
            &[BoxRect {
                left: 0.,
                top: 0.,
                right: 0.,
                bottom: 1.
            }],
            &c
        )
        .is_err()
    );
    assert!(
        tools::compare(
            &image,
            &CleanupOptions {
                line_coverage: f32::NAN,
                ..Default::default()
            },
            &c
        )
        .is_err()
    );
    assert!(
        tools::compare(
            &DynamicImage::new_rgb8(0, 0),
            &CleanupOptions::default(),
            &c
        )
        .is_err()
    );
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(tools::perspective(&image, corners(), 10, 10, &c).is_err());
    assert!(tools::compare(&image, &CleanupOptions::default(), &c).is_err());
    assert!(tools::split_double(&image, 0.5, 0.5, &c).is_err());
    let output = dir.path().join("cancelled.png");
    assert!(tools::export_redacted(&image, &[BoxRect::full()], &output, &c).is_err());
    assert!(!output.exists());
}
