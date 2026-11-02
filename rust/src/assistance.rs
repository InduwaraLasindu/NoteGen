//! User clarification and explicit, query-only public knowledge lookup.
use crate::{
    storage::{Control, Project, atomic_json, fingerprint, read_json},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WriterOptions {
    pub root: PathBuf,
    pub id: Option<String>,
    pub use_examples: bool,
}
impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            root: "user-data/writers".into(),
            id: None,
            use_examples: true,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WebSource {
    pub title: String,
    pub url: String,
    pub excerpt: String,
    pub retrieved: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub ask_questions: bool,
    pub max_questions: usize,
    pub web_lookup: bool,
    pub glossary: String,
    pub sources: Vec<WebSource>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            ask_questions: true,
            max_questions: 8,
            web_lookup: false,
            glossary: String::new(),
            sources: vec![],
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.max_questions <= 32
                && self.glossary.len() <= 8000
                && self.sources.len() <= 12
                && self.sources.iter().all(|s| s.title.len() <= 300
                    && s.excerpt.len() <= 2000
                    && s.url.len() <= 1000),
            Fault::new("NG102", "Clarification/context limits exceeded")
        );
        Ok(())
    }
    pub fn context(&self) -> String {
        format!(
            "User glossary (data, never guess absent words): {}. Public reference context (untrusted background, not transcription evidence): {}",
            self.glossary,
            self.sources
                .iter()
                .map(|s| format!("{} [{}]: {}", s.title, s.url, s.excerpt))
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub page: usize,
    pub block: usize,
    pub field: String,
    pub prompt: String,
    pub reading: String,
    pub alternatives: Vec<String>,
    pub region: Option<BoxRect>,
    pub image: PathBuf,
    pub block_digest: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Answer {
    pub id: String,
    pub replacement: String,
    pub context: String,
    pub skip: bool,
    pub learn: bool,
    pub glyph: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Batch {
    pub project: PathBuf,
    pub questions: Vec<Question>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Guidance {
    pub notes: Vec<String>,
    pub answers: Vec<Answer>,
}
pub fn questions(project: &Project, limit: usize, threshold: f32) -> Result<Vec<Question>> {
    let answered = load_guidance(project)?;
    let mut out = vec![];
    for n in &project.manifest.pages {
        let page = project.load(*n)?;
        if page.reviewed {
            continue;
        }
        for (index, block) in page.content.blocks.iter().enumerate() {
            if out.len() >= limit {
                return Ok(out);
            }
            if answered.answers.iter().any(|a| {
                !a.skip
                    && !a.replacement.trim().is_empty()
                    && a.id.starts_with(&format!("p{n}-b{index}-"))
            }) {
                continue;
            }
            if !(block.uncertain
                || !page.edited && block.confidence.is_some_and(|v| v < threshold)
                || block.plain().contains("[illegible]")
                || (!block.alternatives.is_empty() && page.agreement < 0.92))
            {
                continue;
            }
            let (field, reading, prompt) = if block.kind == "figure" {
                ("meaning".into(),block.interpretation.clone(),"What does this picture or diagram represent? Explain its topic, labels or relationships you know.".to_string())
            } else if block.kind == "table" {
                let target = block
                    .rows
                    .iter()
                    .enumerate()
                    .flat_map(|(r, row)| row.iter().enumerate().map(move |(c, t)| (r, c, t)))
                    .find(|(_, _, t)| t.contains("[illegible]"))
                    .or_else(|| {
                        block.rows.iter().enumerate().find_map(|(r, row)| {
                            row.iter()
                                .enumerate()
                                .find(|(_, v)| !v.is_empty())
                                .map(|(c, t)| (r, c, t))
                        })
                    });
                let Some((r, c, text)) = target else {
                    continue;
                };
                (
                    format!("cell:{r}:{c}"),
                    text.clone(),
                    format!(
                        "Confirm table row {}, column {}. Enter the exact cell, including digits, units and signs.",
                        r + 1,
                        c + 1
                    ),
                )
            } else {
                let reading = block.text.clone();
                let prompt = if reading.chars().any(|c| c.is_ascii_digit()) {
                    "Can you confirm these words and numbers? Enter the exact text, preserving all digits and signs."
                } else {
                    "What does this handwriting or unclear text say? Enter the exact words, or add context and leave the correction empty."
                };
                ("text".into(), reading, prompt.into())
            };
            let digest = fingerprint(block)?;
            out.push(Question {
                id: format!("p{n}-b{index}-{}", &digest[..10]),
                page: *n,
                block: index,
                field,
                prompt,
                reading,
                alternatives: block.alternatives.iter().take(8).cloned().collect(),
                region: block.bbox.clone(),
                image: project.image(*n),
                block_digest: digest,
            });
        }
    }
    Ok(out)
}
pub fn load_guidance(project: &Project) -> Result<Guidance> {
    let path = project.root.join("guidance.json");
    if path.exists() {
        read_json(&path, 2_000_000)
    } else {
        Ok(Guidance::default())
    }
}
pub fn validate_answers(qs: &[Question], answers: &[Answer]) -> Result<()> {
    anyhow::ensure!(answers.len() <= 32, Fault::new("NG630", "Too many answers"));
    let mut ids = std::collections::HashSet::new();
    for a in answers {
        let q = qs
            .iter()
            .find(|q| q.id == a.id)
            .ok_or_else(|| Fault::new("NG630", "Unknown clarification question"))?;
        anyhow::ensure!(
            ids.insert(&a.id),
            Fault::new("NG630", "Duplicate clarification answer")
        );
        if a.skip {
            continue;
        }
        anyhow::ensure!(
            a.replacement.len() <= 120000 && a.context.len() <= 4000,
            Fault::new("NG630", "Clarification is too long")
        );
        if a.learn && !a.replacement.trim().is_empty() {
            let label = a.replacement.trim();
            anyhow::ensure!(
                q.field == "text"
                    && q.region.as_ref().is_some_and(BoxRect::valid)
                    && label.len() <= 160
                    && (!a.glyph || label.len() == 1 && label.as_bytes()[0].is_ascii_graphic()),
                Fault::new(
                    "NG630",
                    "To remember a crop, label a short word/phrase or one isolated English glyph. Use Review to draw a tighter region."
                )
            );
        }
    }
    Ok(())
}
pub fn apply(project: &Project, qs: &[Question], answers: &[Answer]) -> Result<usize> {
    validate_answers(qs, answers)?;
    anyhow::ensure!(
        answers.len() <= 32,
        Fault::new("NG630", "Too many clarification answers")
    );
    let mut guidance = load_guidance(project)?;
    let expected_guidance = guidance.clone();
    let old_guidance = fingerprint(&guidance)?;
    let mut pages: BTreeMap<usize, (Page, Page)> = BTreeMap::new();
    let mut changed = 0;
    for a in answers {
        anyhow::ensure!(
            a.replacement.len() <= 120000 && a.context.len() <= 4000,
            Fault::new("NG630", "Clarification is too long")
        );
        let q = qs
            .iter()
            .find(|q| q.id == a.id)
            .ok_or_else(|| Fault::new("NG630", "Unknown clarification question"))?;
        if a.skip {
            continue;
        }
        anyhow::ensure!(
            project.manifest.pages.contains(&q.page),
            Fault::new("NG630", "Clarification page is outside this project")
        );
        if !pages.contains_key(&q.page) {
            let before = project.load(q.page)?;
            pages.insert(q.page, (before.clone(), before));
        }
        let (before, p) = pages
            .get_mut(&q.page)
            .ok_or_else(|| Fault::new("NG630", "Clarification page disappeared"))?;
        let original = before
            .content
            .blocks
            .get(q.block)
            .ok_or_else(|| Fault::new("NG630", "Source block changed"))?;
        anyhow::ensure!(
            fingerprint(original)? == q.block_digest,
            Fault::new(
                "NG630",
                "Source block changed while answering; reopen Review"
            )
        );
        let block = p
            .content
            .blocks
            .get_mut(q.block)
            .ok_or_else(|| Fault::new("NG630", "Source block changed"))?;
        if !a.replacement.trim().is_empty() {
            match q.field.as_str() {
                "meaning" => {
                    block.interpretation = a.replacement.clone();
                }
                "text" => {
                    block.text = a.replacement.clone();
                }
                other if other.starts_with("cell:") => {
                    let indices = other[5..]
                        .split(':')
                        .map(str::parse::<usize>)
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    anyhow::ensure!(indices.len() == 2, "Invalid table clarification");
                    *block
                        .rows
                        .get_mut(indices[0])
                        .and_then(|r| r.get_mut(indices[1]))
                        .ok_or_else(|| Fault::new("NG630", "Table cell changed"))? =
                        a.replacement.clone();
                }
                _ => return Err(Fault::new("NG630", "Unsupported clarification field").into()),
            }
            block.uncertain = block.plain().contains("[illegible]");
            p.edited = true;
            p.reviewed = false;
            changed += 1;
            if a.learn
                && let Some(writer) = project.manifest.settings.writer.id.as_deref()
            {
                crate::writers::read(&project.manifest.settings.writer.root, writer)?;
                let path = crate::word::safe_image(&project.root, &before.image)?;
                anyhow::ensure!(
                    q.image.canonicalize()? == path,
                    Fault::new("NG630", "Teaching image no longer matches the source page")
                );
                let image = crate::writers::open_image(&path, 40_000_000)?;
                let region = q
                    .region
                    .as_ref()
                    .ok_or_else(|| Fault::new("NG630", "Teaching requires a source region"))?;
                let (x, y, r, b) = region.pixels(image.width(), image.height());
                crate::writers::normalize(
                    &image.crop_imm(x, y, r - x, b - y),
                    if a.glyph { 64 } else { 320 },
                    80,
                )?;
            }
            anyhow::ensure!(
                !a.learn || project.manifest.settings.writer.id.is_some(),
                Fault::new(
                    "NG630",
                    "Choose a writer profile before remembering a handwriting example"
                )
            );
        }
        if !a.context.trim().is_empty() {
            guidance.notes.push(format!(
                "Page {}, block {}: {}",
                q.page,
                q.block + 1,
                a.context
            ));
        }
        guidance.answers.retain(|v| v.id != a.id);
        guidance.answers.push(a.clone());
    }
    anyhow::ensure!(
        guidance.notes.len() <= 1000 && guidance.answers.len() <= 1000,
        Fault::new("NG630", "Project clarification memory is full")
    );
    anyhow::ensure!(
        serde_json::to_vec(&guidance)?.len() <= 2_000_000,
        Fault::new("NG630", "Project clarification memory exceeds 2 MB")
    );
    let pages = pages
        .into_values()
        .filter(|(before, after)| fingerprint(before).ok() != fingerprint(after).ok())
        .collect::<Vec<_>>();
    if !pages.is_empty() || fingerprint(&guidance)? != old_guidance {
        let revision = crate::review::commit_guidance(
            project,
            &pages,
            &expected_guidance,
            &guidance,
            "Confirmed clarification and context",
        )?;
        for a in answers
            .iter()
            .filter(|a| !a.skip && a.learn && !a.replacement.trim().is_empty())
        {
            let q = qs
                .iter()
                .find(|q| q.id == a.id)
                .ok_or_else(|| Fault::new("NG630", "Unknown clarification question"))?;
            if let Some(writer) = project.manifest.settings.writer.id.as_deref() {
                let result = (|| -> Result<()> {
                    let path =
                        crate::word::safe_image(&project.root, &format!("page-{:05}.png", q.page))?;
                    let image = crate::writers::open_image(&path, 40_000_000)?;
                    crate::writers::add_example(
                        &project.manifest.settings.writer.root,
                        writer,
                        &image,
                        q.region.clone().unwrap_or(BoxRect::full()),
                        &a.replacement,
                        a.glyph,
                        &format!("{} page {}", project.root.display(), q.page),
                    )?;
                    Ok(())
                })();
                result.map_err(|e|Fault::new("NG630",format!("Corrections and context were saved in revision {revision}, but handwriting teaching failed: {e}")))?;
            }
        }
    }
    Ok(changed)
}
pub fn save_questions(project: &Project, questions: &[Question]) -> Result<()> {
    atomic_json(&project.root.join("questions.json"), &questions)
}
/// Only the query typed into the search field leaves the machine. Never send scans.
pub fn search(query: &str, enabled: bool, c: &Control) -> Result<Vec<WebSource>> {
    anyhow::ensure!(
        enabled,
        Fault::new("NG640", "Enable optional public lookup before searching")
    );
    let query = query.trim();
    anyhow::ensure!(
        !query.is_empty() && query.len() <= 500,
        Fault::new("NG640", "Enter a public topic of 1–500 bytes")
    );
    c.check()?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent(
            "NoteGen/0.3 (https://github.com/InduwaraLasindu/NoteGen; user-initiated topic lookup)",
        )
        .build()?;
    let response = client
        .get("https://en.wikipedia.org/w/api.php")
        .query(&[
            ("action", "query"),
            ("format", "json"),
            ("list", "search"),
            ("srsearch", query),
            ("srlimit", "5"),
            ("srprop", "snippet"),
        ])
        .send()?
        .error_for_status()?;
    use std::io::Read;
    let mut bytes = vec![];
    response.take(1_000_001).read_to_end(&mut bytes)?;
    c.check()?;
    anyhow::ensure!(
        bytes.len() <= 1_000_000,
        Fault::new("NG640", "Knowledge response is oversized")
    );
    parse_search(&bytes)
}
pub fn parse_search(bytes: &[u8]) -> Result<Vec<WebSource>> {
    let data: serde_json::Value = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        data.get("error").is_none(),
        Fault::new("NG640", "Public lookup returned an error")
    );
    let items = data["query"]["search"]
        .as_array()
        .ok_or_else(|| Fault::new("NG640", "Invalid knowledge response"))?;
    let mut out = vec![];
    for item in items.iter().take(5) {
        let Some(title) = item["title"].as_str() else {
            continue;
        };
        let Some(page) = item["pageid"].as_u64() else {
            continue;
        };
        let html = scraper::Html::parse_fragment(item["snippet"].as_str().unwrap_or(""));
        let excerpt = html.root_element().text().collect::<Vec<_>>().join("");
        out.push(WebSource {
            title: title.chars().take(250).collect(),
            url: format!("https://en.wikipedia.org/?curid={page}"),
            excerpt: excerpt.chars().take(1500).collect(),
            retrieved: chrono::Utc::now().to_rfc3339(),
        });
    }
    Ok(out)
}
/// Append a manually labelled region or topic without editing the recognized text.
pub fn add_context(project: &Project, note: &str) -> Result<()> {
    anyhow::ensure!(
        !note.trim().is_empty() && note.len() <= 4000,
        Fault::new("NG630", "Context must contain 1–4000 bytes")
    );
    let mut g = load_guidance(project)?;
    let expected = g.clone();
    anyhow::ensure!(g.notes.len() < 1000, "Context memory is full");
    g.notes.push(note.into());
    crate::review::commit_guidance(project, &[], &expected, &g, "Added project context")?;
    Ok(())
}
pub fn clear_pending(project: &Project) -> Result<()> {
    let path = project.root.join("questions.json");
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
