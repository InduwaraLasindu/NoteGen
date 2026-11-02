//! Conservative scan observations. These are rescan suggestions, never recognition scores.
use crate::types::*;
use image::DynamicImage;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub code: String,
    pub message: String,
    pub suggestion: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub contrast: u8,
    pub observations: Vec<Observation>,
}
pub fn analyze(im: &DynamicImage) -> Report {
    let gray = im.thumbnail(1000, 1000).to_luma8();
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
