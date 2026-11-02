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
    #[serde(default)]
    pub clear: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    SourceChanged,
    SourceRetained,
    Draft,
    Cleared,
    Ignored,
}
impl Preview {
    pub fn source(image: PathBuf, page: usize, reader: &str, region: Option<BoxRect>) -> Self {
        Self {
            image: Some(image),
            page,
            reader: reader.into(),
            region,
            draft: Some(String::new()),
            clear: false,
        }
    }
    pub fn draft(page: usize, text: &str) -> Self {
        Self {
            page,
            draft: Some(visible_draft(text)),
            ..Self::default()
        }
    }
    pub fn clear() -> Self {
        Self {
            clear: true,
            ..Self::default()
        }
    }
    /// Accept draft text only while its source page is still displayed. Clearing
    /// a view prevents queued drafts from bringing that page back without a source.
    pub fn apply(&mut self, update: Self) -> Change {
        // Accept the old empty-path reset as well as explicit reset events.
        if update.clear
            || update
                .image
                .as_ref()
                .is_some_and(|p| p.as_os_str().is_empty())
        {
            *self = Self::clear();
            return Change::Cleared;
        }
        if update.image.is_some() {
            let changed = self.image != update.image;
            *self = update;
            return if changed {
                Change::SourceChanged
            } else {
                Change::SourceRetained
            };
        }
        if self.image.is_some()
            && self.page == update.page
            && let Some(text) = update.draft
        {
            self.draft = Some(text);
            return Change::Draft;
        }
        Change::Ignored
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
