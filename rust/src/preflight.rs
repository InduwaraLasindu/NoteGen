//! Conservative scan observations. These are rescan suggestions, never recognition scores.
use crate::types::*;
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub code: String,
    pub message: String,
    pub suggestion: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<BoxRect>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub contrast: u8,
    pub observations: Vec<Observation>,
}
pub fn analyze(im: &DynamicImage) -> Report {
    let gray = im
        .thumbnail(im.width().min(1000), im.height().min(1000))
        .to_luma8();
    let mut histogram = [0usize; 256];
    for v in gray.as_raw() {
        histogram[*v as usize] += 1;
    }
    let count = gray.as_raw().len().max(1);
    let quantile = |target: usize| {
        let mut n = 0;
        for (v, h) in histogram.iter().enumerate() {
            n += h;
            if n >= target {
                return v as u8;
            }
        }
        255
    };
    let contrast =
        quantile((count * 99 / 100).max(1)).saturating_sub(quantile((count / 100).max(1)));
    let mut out = Report {
        width: im.width(),
        height: im.height(),
        contrast,
        observations: vec![],
    };
    let mut add = |code: &str, message: &str, suggestion: &str| {
        out.observations.push(Observation {
            code: code.into(),
            message: message.into(),
            suggestion: suggestion.into(),
            region: None,
        })
    };
    if im.width() < 600 || im.height() < 600 {
        add(
            "small-image",
            "Small source image may lose fine writing.",
            "Capture the page closer or scan at 300–400 DPI; keep the whole page visible.",
        );
    }
    if contrast < 45 {
        add(
            "low-contrast",
            "Little separation between light and dark pixels.",
            "Use even lighting, a contrasting background and a sharper original scan.",
        );
    }
    if crate::scan::blur_warning(im) {
        add(
            "blur",
            "Low edge detail may indicate blur or a blank page.",
            "Check the original at full size. Stabilize the camera and focus on the writing if blurred.",
        );
    }
    let saturated = gray.as_raw().iter().filter(|v| **v >= 253).count();
    let dark = gray.as_raw().iter().filter(|v| **v < 100).count();
    if saturated * 100 / count > 95 && dark * 1000 / count < 2 {
        add(
            "blank-or-overexposed",
            "The source is almost entirely white; glare or a blank page is possible.",
            "Check for missing ink. Reduce direct light and photograph the page from another angle.",
        );
    }
    // These are bounded pixel observations, not diagnoses of missing writing.
    // A dark frame, blank gap or decoration may produce the same signal.
    let width = gray.width() as usize;
    let height = gray.height() as usize;
    if width >= 20 && height >= 20 {
        let border = 2usize;
        let mut touching = 0usize;
        let mut border_pixels = 0usize;
        for y in 0..height {
            for x in 0..width {
                if x < border || y < border || x + border >= width || y + border >= height {
                    border_pixels += 1;
                    touching += usize::from(gray[(x as u32, y as u32)][0] < 100);
                }
            }
        }
        if touching >= 12 && touching * 1000 >= border_pixels * 5 {
            add(
                "possible-clipping",
                "Dark content reaches the image boundary; writing or the page edge may be clipped.",
                "Inspect all four edges. Include a clear margin around the entire page and avoid a dark camera frame.",
            );
        }
        // Connected ink components at source-pixel scale. Rules and long strokes
        // are excluded; very dense/noisy pages stop at a fixed component budget.
        let mut seen = vec![false; width * height];
        let mut components = 0usize;
        let mut small = 0usize;
        let mut text_like = 0usize;
        let scale = (im.width() as f64 / width as f64).max(im.height() as f64 / height as f64);
        let mut queue = VecDeque::new();
        for index in 0..seen.len() {
            if seen[index] || gray.as_raw()[index] >= 130 {
                continue;
            }
            components += 1;
            if components > 50_000 {
                break;
            }
            seen[index] = true;
            queue.push_back(index);
            let (mut left, mut right, mut top, mut bottom, mut area) = (
                index % width,
                index % width,
                index / width,
                index / width,
                0usize,
            );
            while let Some(pixel) = queue.pop_front() {
                let x = pixel % width;
                let y = pixel / width;
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
                area += 1;
                for next in [
                    x.checked_sub(1).map(|x| y * width + x),
                    (x + 1 < width).then_some(y * width + x + 1),
                    y.checked_sub(1).map(|y| y * width + x),
                    (y + 1 < height).then_some((y + 1) * width + x),
                ]
                .into_iter()
                .flatten()
                {
                    if !seen[next] && gray.as_raw()[next] < 130 {
                        seen[next] = true;
                        queue.push_back(next);
                    }
                }
            }
            let h = bottom - top + 1;
            let w = right - left + 1;
            if area >= 3 && (2..=80).contains(&h) && w <= h * 5 && w >= 1 && area <= 4000 {
                text_like += 1;
                if h as f64 * scale < 10. {
                    small += 1;
                }
            }
        }
        if small >= 20 && small * 2 >= text_like {
            add(
                "tiny-writing",
                "Many separate ink components are under ten source pixels high; small writing may be undersampled.",
                "Check letters at full resolution. Move closer or scan at a higher DPI while keeping page margins; compare punctuation and decimal points.",
            );
        }
        let tile = (width.min(height) / 20).clamp(12, 48);
        let stride = (tile / 2).max(1);
        let mut glare = None;
        'tiles: for y in (tile..height.saturating_sub(tile * 2)).step_by(stride) {
            for x in (tile..width.saturating_sub(tile * 2)).step_by(stride) {
                let mut bright = 0usize;
                let mut sum = 0usize;
                let mut ring_sum = 0usize;
                let mut ring_ink = 0usize;
                let mut ring_count = 0usize;
                for yy in y - tile..y + tile * 2 {
                    for xx in x - tile..x + tile * 2 {
                        let value = gray[(xx as u32, yy as u32)][0] as usize;
                        if xx >= x && xx < x + tile && yy >= y && yy < y + tile {
                            sum += value;
                            bright += usize::from(value >= 253);
                        } else {
                            ring_sum += value;
                            ring_ink += usize::from(value < 130);
                            ring_count += 1;
                        }
                    }
                }
                let area = tile * tile;
                if bright * 100 >= area * 98
                    && sum / area >= 253
                    && ring_count > 0
                    && ring_sum / ring_count <= 233
                    && ring_ink * 100 >= ring_count * 2
                {
                    glare = Some(BoxRect {
                        left: x as f32 / width as f32,
                        top: y as f32 / height as f32,
                        right: (x + tile) as f32 / width as f32,
                        bottom: (y + tile) as f32 / height as f32,
                    });
                    break 'tiles;
                }
            }
        }
        if let Some(region) = glare {
            out.observations.push(Observation{code:"possible-local-glare".into(),message:"A nearly saturated white patch has a darker, ink-bearing surround; glare or a blank patch is possible.".into(),suggestion:"Inspect the highlighted source area. Rephotograph with diffuse light and a changed camera angle if ink disappears; cleanup cannot recover destroyed detail.".into(),region:Some(region)});
        }
    }
    out
}
pub fn numeric_observations(content: &PageContent) -> Vec<String> {
    let mut out = vec![];
    for (index, b) in content.blocks.iter().enumerate() {
        for token in b.plain().split_whitespace() {
            if token.chars().any(|c| c.is_ascii_digit())
                && (token.contains(['O', 'o', 'I', 'l'])
                    && token
                        .chars()
                        .all(|c| c.is_ascii_digit() || "OoIl.,+-/:%()".contains(c))
                    || token.contains("..")
                    || token.contains(",,"))
            {
                out.push(format!("Block {}: confirm numeric token {token:?}; possible letter/digit or decimal ambiguity.",index+1));
            }
            if out.len() >= 100 {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};
    fn has(report: &Report, code: &str) -> bool {
        report.observations.iter().any(|item| item.code == code)
    }
    #[test]
    fn clipping_reports_visible_edge_contact_without_modifying_pixels() {
        let mut image = GrayImage::from_pixel(800, 800, Luma([255]));
        for y in 100..300 {
            for x in 0..4 {
                image[(x, y)] = Luma([0]);
            }
        }
        let image = DynamicImage::ImageLuma8(image);
        let before = image.to_luma8();
        let report = analyze(&image);
        assert!(has(&report, "possible-clipping"));
        assert_eq!(image.to_luma8(), before);
    }
    #[test]
    fn local_saturation_observation_has_bounded_source_region_and_no_recovery_claim() {
        let mut image = GrayImage::from_pixel(720, 720, Luma([210]));
        for y in 250..410 {
            for x in 250..410 {
                if x % 14 < 2 {
                    image[(x, y)] = Luma([60]);
                }
            }
        }
        for y in 300..360 {
            for x in 300..360 {
                image[(x, y)] = Luma([255]);
            }
        }
        let report = analyze(&DynamicImage::ImageLuma8(image));
        let item = report
            .observations
            .iter()
            .find(|item| item.code == "possible-local-glare")
            .unwrap();
        assert!(item.region.as_ref().unwrap().valid());
        assert!(item.message.contains("possible"));
        assert!(item.suggestion.contains("cannot recover"));
    }
    #[test]
    fn tiny_writing_uses_source_pixels_and_does_not_confuse_a_larger_capture_with_the_thumbnail() {
        let mut image = GrayImage::from_pixel(800, 800, Luma([255]));
        for row in 0..5 {
            for column in 0..8 {
                for y in 0..4 {
                    for x in 0..2 {
                        image[(50 + column * 50 + x, 50 + row * 50 + y)] = Luma([0]);
                    }
                }
            }
        }
        let image = DynamicImage::ImageLuma8(image);
        assert!(has(&analyze(&image), "tiny-writing"));
        let larger = image.resize_exact(2400, 2400, image::imageops::FilterType::Nearest);
        assert!(!has(&analyze(&larger), "tiny-writing"));
    }
    #[test]
    fn blank_white_pages_do_not_invent_clipping_tiny_ink_or_local_glare() {
        let report = analyze(&DynamicImage::ImageLuma8(GrayImage::from_pixel(
            800,
            800,
            Luma([255]),
        )));
        assert!(has(&report, "blank-or-overexposed"));
        for code in ["possible-clipping", "tiny-writing", "possible-local-glare"] {
            assert!(!has(&report, code));
        }
    }
}
