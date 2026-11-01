//! Only observable source data and generated answer text. Reasoning channels are never read.
use crate::types::BoxRect;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Preview {
    pub page: usize,
    pub image: Option<PathBuf>,
    pub region: Option<BoxRect>,
    pub reader: String,
    pub draft: Option<String>,
}
impl Preview {
    pub fn source(image: PathBuf, page: usize, reader: &str, region: Option<BoxRect>) -> Self {
        Self {
            image: Some(image),
            page,
            reader: reader.into(),
            region,
            draft: Some(String::new()),
        }
    }
    pub fn draft(page: usize, text: &str) -> Self {
        Self {
            page,
            draft: Some(visible_draft(text)),
            ..Self::default()
        }
    }
}
/// Some external servers put reasoning tags in content. Hide complete and incomplete
/// reasoning sections, and cap the visible text independently of the response parser.
pub fn visible_draft(raw: &str) -> String {
    let mut result = String::new();
    let mut tail = raw;
    while let Some(start) = tail.find("<think>") {
        result.push_str(&tail[..start]);
        let rest = &tail[start + 7..];
        let Some(end) = rest.find("</think>") else {
            return result.chars().take(16000).collect();
        };
        tail = &rest[end + 8..];
    }
    // Do not reveal the beginning of a tag split across stream packets.
    let cut = (1..7)
        .rev()
        .find(|&n| tail.ends_with(&"<think>"[..n]))
        .unwrap_or(0);
    result.push_str(&tail[..tail.len() - cut]);
    result.chars().take(16000).collect()
}
