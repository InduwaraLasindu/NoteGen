//! Explicit local project catalogs, source-linked search and recovery reports.
use crate::{
    storage::{Control, Project, atomic_json, hash_file, read_json},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const MAX_PROJECTS: usize = 1000;
fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG500", message).into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reference {
    pub id: String,
    pub label: String,
    pub role: String,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    pub schema: u32,
    pub name: String,
    pub folder: String,
    pub tags: Vec<String>,
    pub status: String,
    pub updated: String,
    pub source: Option<PathBuf>,
    pub references: Vec<Reference>,
}
impl Default for Metadata {
    fn default() -> Self {
        Self {
            schema: 1,
            name: String::new(),
            folder: String::new(),
            tags: vec![],
            status: "active".into(),
            updated: String::new(),
            source: None,
            references: vec![],
        }
    }
}
fn validate_metadata(m: &Metadata) -> Result<()> {
    anyhow::ensure!(
        m.schema == 1
            && m.name.len() <= 200
            && m.folder.len() <= 200
            && m.updated.len() <= 100
            && m.tags.len() <= 32
            && m.tags.iter().all(|t| !t.trim().is_empty() && t.len() <= 80)
            && m.tags.iter().collect::<BTreeSet<_>>().len() == m.tags.len()
            && ["active", "paused", "archived", "complete"].contains(&m.status.as_str())
            && m.references.len() <= 100,
        fault("Invalid project metadata")
    );
    let mut ids = BTreeSet::new();
    for r in &m.references {
        anyhow::ensure!(
            r.id.len() == 64
                && r.id.chars().all(|c| c.is_ascii_hexdigit())
                && ids.insert(&r.id)
                && !r.label.trim().is_empty()
                && r.label.len() <= 200
                && ["background", "diagram", "reference"].contains(&r.role.as_str())
                && r.path.is_absolute()
                && r.sha256.len() == 64
                && r.sha256.chars().all(|c| c.is_ascii_hexdigit()),
            fault("Invalid local reference metadata")
        );
    }
    if let Some(source) = &m.source {
        anyhow::ensure!(
            source.is_absolute(),
            fault("Record an absolute original source path")
        );
    }
    Ok(())
}
fn internal(p: &Project, path: &Path) -> Result<()> {
    if path.exists() {
        anyhow::ensure!(
            fs::canonicalize(path)?.starts_with(fs::canonicalize(&p.root)?),
            fault("A project file resolves outside its project folder")
        );
    }
    Ok(())
}
fn open(root: &Path) -> Result<Project> {
    let canonical = fs::canonicalize(root)?;
    let manifest = canonical.join("project.json");
    anyhow::ensure!(
        fs::canonicalize(&manifest)?.starts_with(&canonical),
        fault("Project manifest resolves outside its folder")
    );
    Project::read(canonical)
}
fn page_safe(p: &Project, n: usize) -> Result<()> {
    for path in [
        p.page_path(n),
        p.root.join(format!("page-{n:05}.edits.json")),
        p.image(n),
    ] {
        internal(p, &path)?;
    }
    Ok(())
}
pub fn metadata(p: &Project) -> Result<Metadata> {
    let path = p.root.join("project-metadata.json");
    internal(p, &path)?;
    let mut m = if path.is_file() {
        read_json(&path, 1_000_000)?
    } else {
        Metadata::default()
    };
    validate_metadata(&m)?;
    if m.name.is_empty() {
        m.name = p
            .root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled project".into());
    }
    Ok(m)
}
pub fn save_metadata(p: &Project, m: &Metadata) -> Result<()> {
    validate_metadata(m)?;
    let mut m = m.clone();
    m.updated = chrono::Utc::now().to_rfc3339();
    let path = p.root.join("project-metadata.json");
    internal(p, &path)?;
    atomic_json(&path, &m)
}
/// Attach an explicitly selected local background file. It never becomes transcription evidence.
pub fn attach_reference(
    p: &Project,
    input: &Path,
    label: &str,
    role: &str,
    c: &Control,
) -> Result<Reference> {
    anyhow::ensure!(
        !label.trim().is_empty()
            && label.len() <= 200
            && ["background", "diagram", "reference"].contains(&role),
        fault("Choose a reference label and background/diagram/reference role")
    );
    let path = fs::canonicalize(input)?;
    let info = fs::metadata(&path)?;
    anyhow::ensure!(
        info.is_file() && info.len() <= 512_000_000,
        fault("Reference must be a local file no larger than 512 MB")
    );
    let sha256 = hash_file(&path, c)?;
    let id = crate::storage::fingerprint(&(path.clone(), sha256.clone(), role))?;
    let reference = Reference {
        id,
        label: label.trim().into(),
        role: role.into(),
        path,
        sha256,
        bytes: info.len(),
    };
    let mut m = metadata(p)?;
    anyhow::ensure!(
        !m.references.iter().any(|r| r.id == reference.id),
        fault("This file and role are already linked")
    );
    m.references.push(reference.clone());
    save_metadata(p, &m)?;
    Ok(reference)
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Filter {
    pub query: String,
    pub tag: Option<String>,
    pub folder: Option<String>,
    pub status: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub root: PathBuf,
    pub metadata: Metadata,
    pub pages: usize,
    pub completed: usize,
    pub pending_review: usize,
    pub input_sha256: String,
    pub error: Option<String>,
}
fn validate_roots(roots: &[PathBuf]) -> Result<()> {
    anyhow::ensure!(
        roots.len() <= MAX_PROJECTS,
        fault("Select no more than 1,000 explicit project folders")
    );
    Ok(())
}
/// Read only the project folders supplied by the caller. No recursive disk indexing occurs.
pub fn catalog(roots: &[PathBuf], filter: &Filter) -> Result<Vec<Entry>> {
    validate_roots(roots)?;
    anyhow::ensure!(
        filter.query.len() <= 1000,
        fault("Catalog query is too long")
    );
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    for root in roots {
        let key = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if !seen.insert(key.clone()) {
            continue;
        }
        let result = (|| -> Result<Entry> {
            let p = open(&key)?;
            let m = metadata(&p)?;
            let mut completed = 0;
            let mut pending_review = 0;
            let mut warnings = vec![];
            for n in &p.manifest.pages {
                page_safe(&p, *n)?;
                match p.load(*n) {
                    Ok(page) => {
                        completed += 1;
                        if page.needs_review(p.manifest.settings.review_threshold) {
                            pending_review += 1;
                        }
                    }
                    Err(e) => warnings.push(format!("Page {n}: {e}")),
                }
            }
            Ok(Entry {
                root: p.root,
                metadata: m,
                pages: p.manifest.pages.len(),
                completed,
                pending_review,
                input_sha256: p.manifest.input_sha256,
                error: (!warnings.is_empty()).then(|| warnings.join("; ")),
            })
        })();
        let entry = match result {
            Ok(entry) => entry,
            Err(e) => Entry {
                root: key,
                metadata: Metadata {
                    name: root
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    ..Default::default()
                },
                pages: 0,
                completed: 0,
                pending_review: 0,
                input_sha256: String::new(),
                error: Some(e.to_string()),
            },
        };
        let query = filter.query.to_lowercase();
        if !query.is_empty()
            && !format!(
                "{} {} {}",
                entry.metadata.name,
                entry.metadata.folder,
                entry.metadata.tags.join(" ")
            )
            .to_lowercase()
            .contains(&query)
        {
            continue;
        }
        if filter
            .tag
            .as_ref()
            .is_some_and(|tag| !entry.metadata.tags.contains(tag))
            || filter
                .folder
                .as_ref()
                .is_some_and(|folder| &entry.metadata.folder != folder)
            || filter
                .status
                .as_ref()
                .is_some_and(|status| &entry.metadata.status != status)
        {
            continue;
        }
        out.push(entry);
    }
    out.sort_by(|a, b| {
        b.metadata
            .updated
            .cmp(&a.metadata.updated)
            .then_with(|| a.metadata.name.cmp(&b.metadata.name))
            .then_with(|| a.root.cmp(&b.root))
    });
    Ok(out)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub project: PathBuf,
    pub page: usize,
    pub block: usize,
    pub region: Option<BoxRect>,
    pub image: PathBuf,
    pub kind: String,
    pub reviewed: bool,
    pub excerpt: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchReport {
    pub hits: Vec<SearchHit>,
    pub warnings: Vec<String>,
    pub truncated: bool,
}
fn excerpt(text: &str, query: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 280 {
        return text.into();
    }
    let lower = text.to_lowercase();
    let position = lower
        .find(query)
        .map(|i| lower[..i].chars().count())
        .unwrap_or(0)
        .min(chars.len());
    let start = position.saturating_sub(70);
    let end = (start + 280).min(chars.len());
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        chars[start..end].iter().collect::<String>(),
        if end < chars.len() { "…" } else { "" }
    )
}
/// Search recognized output, including cells and figure labels, with explicit source locations.
pub fn search(roots: &[PathBuf], query: &str, limit: usize) -> Result<SearchReport> {
    validate_roots(roots)?;
    anyhow::ensure!(
        !query.trim().is_empty() && query.len() <= 1000 && (1..=10000).contains(&limit),
        fault("Enter a 1–1,000 byte search and a limit of 1–10,000")
    );
    let mut report = SearchReport {
        hits: vec![],
        warnings: vec![],
        truncated: false,
    };
    let query = query.to_lowercase();
    let mut seen = BTreeSet::new();
    for root in roots {
        let key = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if !seen.insert(key.clone()) {
            continue;
        }
        let p = match open(&key) {
            Ok(p) => p,
            Err(e) => {
                report.warnings.push(format!("{}: {e}", root.display()));
                continue;
            }
        };
        for n in &p.manifest.pages {
            let page = match page_safe(&p, *n).and_then(|_| p.load(*n)) {
                Ok(page) => page,
                Err(e) => {
                    report
                        .warnings
                        .push(format!("{} page {n}: {e}", root.display()));
                    continue;
                }
            };
            for (index, b) in page.content.blocks.iter().enumerate() {
                let text = format!("{}\n{}\n{}", b.plain(), b.description, b.interpretation);
                if !text.to_lowercase().contains(&query) {
                    continue;
                }
                if report.hits.len() == limit {
                    report.truncated = true;
                    return Ok(report);
                }
                report.hits.push(SearchHit {
                    project: p.root.clone(),
                    page: *n,
                    block: index,
                    region: b.bbox.clone(),
                    image: p.image(*n),
                    kind: b.kind.clone(),
                    reviewed: page.reviewed,
                    excerpt: excerpt(text.trim(), &query),
                });
            }
        }
    }
    Ok(report)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub input_sha256: String,
    pub projects: Vec<PathBuf>,
}
pub fn duplicates(roots: &[PathBuf]) -> Result<Vec<DuplicateGroup>> {
    let entries = catalog(roots, &Filter::default())?;
    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for entry in entries {
        if entry.input_sha256.len() == 64
            && entry.input_sha256.chars().all(|c| c.is_ascii_hexdigit())
        {
            groups
                .entry(entry.input_sha256)
                .or_default()
                .push(entry.root);
        }
    }
    Ok(groups
        .into_iter()
        .filter(|(_, p)| p.len() > 1)
        .map(|(input_sha256, projects)| DuplicateGroup {
            input_sha256,
            projects,
        })
        .collect())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryPage {
    pub page: usize,
    pub state: String,
    pub original_checkpoint: bool,
    pub corrected_checkpoint: bool,
    pub source_image: bool,
    pub reusable: bool,
    pub next_action: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceState {
    pub id: String,
    pub label: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub project: PathBuf,
    pub input_state: String,
    pub pages: Vec<RecoveryPage>,
    pub references: Vec<ReferenceState>,
    pub pending_review: usize,
    pub completed: usize,
    pub pending_transaction: bool,
}
pub fn recovery(p: &Project) -> Result<RecoveryReport> {
    let m = metadata(p)?;
    let input_state = match &m.source {
        None => "Original input path was not recorded; select the matching input to resume.".into(),
        Some(path) => {
            if path.is_file() {
                if hash_file(path, &Control::default())? == p.manifest.input_sha256 {
                    "Original input matches the project checksum.".into()
                } else {
                    "Original input changed; select the original file or create a new project."
                        .into()
                }
            } else {
                "Original input is missing; restore it before reconversion.".into()
            }
        }
    };
    let mut report = RecoveryReport {
        project: p.root.clone(),
        input_state,
        pages: vec![],
        references: vec![],
        pending_review: 0,
        completed: 0,
        pending_transaction: p.root.join("review-pending.json").is_file(),
    };
    for n in &p.manifest.pages {
        let original_checkpoint = p.page_path(*n).is_file();
        let corrected_checkpoint = p.root.join(format!("page-{n:05}.edits.json")).is_file();
        let source_image = p.image(*n).is_file();
        let loaded = page_safe(p, *n).and_then(|_| p.load(*n));
        let error = loaded.as_ref().err().map(ToString::to_string);
        let (state, reusable, next_action) = match loaded {
            Ok(page) => {
                report.completed += 1;
                if page.needs_review(p.manifest.settings.review_threshold) {
                    report.pending_review += 1;
                }
                if !source_image {
                    (
                        "missing_source",
                        true,
                        "Recognition is retained. Restore or render the source image before visual review.",
                    )
                } else if page.needs_review(p.manifest.settings.review_threshold) {
                    (
                        "pending_review",
                        true,
                        "Reuse the saved recognition and review flagged source regions.",
                    )
                } else {
                    (
                        "complete",
                        true,
                        "Reuse the saved page; export can continue without another reader pass.",
                    )
                }
            }
            Err(_) => {
                if original_checkpoint || corrected_checkpoint {
                    (
                        "invalid_checkpoint",
                        false,
                        "A checkpoint is invalid. Preserve a copy, inspect the error and reconvert this page.",
                    )
                } else if source_image {
                    (
                        "recognition_pending",
                        false,
                        "Reuse the rendered image; recognition has not completed.",
                    )
                } else {
                    (
                        "render_pending",
                        false,
                        "Restore the matching input, render and recognize this page.",
                    )
                }
            }
        };
        report.pages.push(RecoveryPage {
            page: *n,
            state: state.into(),
            original_checkpoint,
            corrected_checkpoint,
            source_image,
            reusable,
            next_action: next_action.into(),
            error,
        });
    }
    for r in &m.references {
        let state = if !r.path.is_file() {
            "missing"
        } else if fs::metadata(&r.path)?.len() != r.bytes
            || hash_file(&r.path, &Control::default())? != r.sha256
        {
            "changed"
        } else {
            "verified"
        };
        report.references.push(ReferenceState {
            id: r.id.clone(),
            label: r.label.clone(),
            state: state.into(),
        });
    }
    Ok(report)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub schema: u32,
    pub name: String,
    pub settings: Settings,
}
/// Create a portable template after showing its settings to the user. Existing files are protected.
pub fn save_template(output: &Path, name: &str, settings: &Settings) -> Result<()> {
    anyhow::ensure!(
        !name.trim().is_empty() && name.len() <= 200,
        fault("Template name must contain 1–200 bytes")
    );
    settings.validate()?;
    // Device paths, existing writer identity and public-source excerpts are local/private, not portable policy.
    let mut portable = settings.clone();
    let defaults = Settings::default();
    portable.tesseract = defaults.tesseract;
    portable.pdfium = defaults.pdfium;
    portable.server = defaults.server;
    portable.models_dir = defaults.models_dir;
    portable.writer.root = defaults.writer.root;
    portable.writer.id = None;
    portable.writer_reference = None;
    portable.assistance.sources.clear();
    let template = Template {
        schema: 1,
        name: name.trim().into(),
        settings: portable,
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, &template)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist_noclobber(output).map_err(|e| e.error)?;
    Ok(())
}
pub fn load_template(input: &Path) -> Result<Template> {
    let t: Template = read_json(input, 1_000_000)?;
    anyhow::ensure!(
        t.schema == 1 && !t.name.trim().is_empty() && t.name.len() <= 200,
        fault("Invalid template schema or name")
    );
    t.settings.validate()?;
    Ok(t)
}
