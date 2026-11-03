//! Explicit content duplicates. Reuse is strict; comparison/forks never alter originals.
use crate::{
    pipeline::{self, PlannedIdentity},
    storage::{Control, Project, fingerprint},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const BYTE_LIMIT: u64 = 4_000_000_000;
fn fail(message: impl Into<String>) -> anyhow::Error {
    Fault::new("NG500", message).into()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    relative: String,
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub root: PathBuf,
    pub compatible: bool,
    pub reasons: Vec<String>,
    pub pages: usize,
    pub review_pages: usize,
    pub snapshot: String,
}
#[derive(Clone, Serialize)]
pub struct Preview {
    pub input_sha256: String,
    pub fingerprint: String,
    pub candidates: Vec<Candidate>,
    pub warnings: Vec<String>,
    pub token: String,
    pub note: String,
    #[serde(skip)]
    source: PathBuf,
    #[serde(skip)]
    settings: Settings,
    #[serde(skip)]
    files: Vec<Vec<File>>,
}
#[derive(Clone, Copy, Debug)]
pub enum Choice {
    Reuse,
    SeparateRevision,
}
#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    pub same_input: bool,
    pub same_recognition: bool,
    pub active_order_changed: bool,
    pub content_withheld: bool,
    pub differences: Vec<crate::review::Difference>,
    pub omitted_differences: usize,
    pub note: String,
}
fn source_file(path: &Path, c: &Control) -> Result<(u64, String)> {
    use sha2::Digest;
    crate::watch::no_links(path)?;
    let file = fs::File::open(path)?;
    let meta = file.metadata()?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= BYTE_LIMIT,
        fail("Duplicate source is not a bounded regular file")
    );
    let mut file = file.take(meta.len() + 1);
    let mut hash = sha2::Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        c.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        anyhow::ensure!(
            total <= meta.len(),
            fail("Duplicate source grew while hashing")
        );
        hash.update(&buffer[..n]);
    }
    anyhow::ensure!(
        total == meta.len(),
        fail("Duplicate source changed while hashing")
    );
    Ok((total, hex::encode(hash.finalize())))
}
fn snapshot(root: &Path, c: &Control) -> Result<Vec<File>> {
    snapshot_budget(root, c, BYTE_LIMIT)
}
fn snapshot_budget(root: &Path, c: &Control, limit: u64) -> Result<Vec<File>> {
    crate::migrations::ensure_readable(root)?;
    anyhow::ensure!(
        !root.join("revision-copy.pending").exists(),
        fail("Separate revision is incomplete; preserve it and choose a new destination")
    );
    let root = root.canonicalize()?;
    crate::watch::no_links(&root)?;
    let mut stack = vec![(root.clone(), 0usize)];
    let mut output = vec![];
    let mut total = 0u64;
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        anyhow::ensure!(
            depth <= 32,
            fail("Private project exceeds revision copy depth")
        );
        for entry in fs::read_dir(&dir)? {
            c.check()?;
            visited += 1;
            anyhow::ensure!(
                visited <= 10000,
                fail("Private revision exceeds 10,000 directory entries")
            );
            let entry = entry?;
            let path = entry.path();
            crate::watch::no_links(&path)?;
            let relative = path
                .strip_prefix(&root)?
                .to_str()
                .ok_or_else(|| fail("Project contains a non-Unicode filename"))?
                .replace('\\', "/");
            anyhow::ensure!(
                relative.len() <= 4096,
                fail("Private revision path exceeds its 4 KiB portable budget")
            );
            let metadata = entry.metadata()?;
            if depth == 0
                && metadata.len() == 0
                && ["review.lock", "migration.lock", "project-metadata.lock"]
                    .contains(&relative.as_str())
            {
                continue;
            }
            if metadata.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            anyhow::ensure!(
                metadata.is_file() && output.len() < 10000,
                fail("Private project exceeds file count or contains a special file")
            );
            total = total.saturating_add(metadata.len());
            anyhow::ensure!(
                total <= limit,
                fail("Private revision or inspection exceeds its 4 GB byte budget")
            );
            let (bytes, sha256) = source_file(&path, c)?;
            output.push(File {
                relative,
                bytes,
                sha256,
            });
        }
    }
    output.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(output)
}
fn compatible(
    project: &Project,
    expected: &PlannedIdentity,
    c: &Control,
) -> Result<(Vec<String>, usize)> {
    let mut reasons = vec![];
    if project.manifest.pages != expected.pages {
        reasons.push("Active page order or selection differs from the requested source.".into());
    }
    if project.manifest.fingerprint != expected.fingerprint {
        reasons.push("Recognition settings, reader/runtime bytes or effective resource plan differ (or legacy identity cannot prove compatibility).".into());
    }
    if expected.external_readers {
        reasons.push("An HTTP reader alias cannot prove the same model/runtime version; comparison and a separate revision remain available.".into());
    }
    if !expected.ocr_executable_verified {
        reasons.push("The selected OCR executable identity is unavailable.".into());
    }
    if project.root.join("input-invalidated.json").exists() {
        reasons.push("Source was invalidated during an earlier conversion.".into());
    }
    if let Some(layout) = crate::project_layout::catalog(project)? {
        if project.manifest.pages != layout.states[0].order {
            reasons.push(
                "The active page organization differs from the original input; reuse is withheld."
                    .into(),
            );
        }
    }
    let mut review = 0;
    for n in &project.manifest.pages {
        let page = project.load(*n)?;
        crate::geometry::verify_metadata_source(project, &page, c)?;
        if page.content.blocks.is_empty()
            || project
                .root
                .join(format!("page-{n:05}.pending-recognition.json"))
                .exists()
        {
            reasons.push("A prepared or empty page still requires explicit recognition.".into());
            break;
        }
        if page.needs_review(project.manifest.settings.review_threshold) {
            review += 1;
        }
    }
    Ok((reasons, review))
}
pub fn preview(
    source: &Path,
    settings: &Settings,
    roots: &[PathBuf],
    c: &Control,
) -> Result<Preview> {
    anyhow::ensure!(
        roots.len() <= 1000,
        fail("Choose at most 1,000 explicit project roots")
    );
    let expected = pipeline::planned_identity(source, settings, c)?;
    let mut candidates = vec![];
    let mut files = vec![];
    let mut warnings = vec![];
    let mut seen = BTreeSet::new();
    let mut budget = 0u64;
    for root in roots {
        c.check()?;
        if candidates.len() == 32 {
            warnings.push("Only the first 32 matching projects were inspected; narrow the supplied roots to inspect others.".into());
            break;
        }
        let root = match root.canonicalize() {
            Ok(root) => root,
            Err(error) => {
                warnings.push(format!("Cannot inspect {}: {error}", root.display()));
                continue;
            }
        };
        if !seen.insert(root.clone()) {
            continue;
        }
        let result: Result<Option<(Candidate, Vec<File>)>> = (|| {
            let project = Project::read(root.clone())?;
            if project.manifest.input_sha256 != expected.input_sha256 {
                return Ok(None);
            }
            let (reasons, review_pages) = compatible(&project, &expected, c)?;
            let checked = snapshot_budget(&root, c, BYTE_LIMIT - budget)?;
            let size = checked.iter().map(|file| file.bytes).sum::<u64>();
            budget += size;
            Ok(Some((
                Candidate {
                    root: root.clone(),
                    compatible: reasons.is_empty(),
                    reasons,
                    pages: project.manifest.pages.len(),
                    review_pages,
                    snapshot: fingerprint(&checked)?,
                },
                checked,
            )))
        })();
        match result {
            Ok(Some((candidate, checked))) => {
                candidates.push(candidate);
                files.push(checked)
            }
            Ok(None) => {}
            Err(error) => {
                if error
                    .downcast_ref::<Fault>()
                    .is_some_and(|f| f.code == "NG900")
                {
                    return Err(error);
                }
                warnings.push(format!("{}: {error}", root.display()));
            }
        }
    }
    let token = fingerprint(&(&expected.input_sha256, &expected.fingerprint, &candidates))?;
    Ok(Preview{input_sha256:expected.input_sha256,fingerprint:expected.fingerprint,candidates,warnings,token,source:source.canonicalize()?,settings:settings.clone(),files,note:"Reuse opens the exact existing private project, including its confirmed corrections and review requirements. It never substitutes a differently configured reader. Comparison is read-only. A separate revision copies private originals, evidence, history and exact locks to a new folder; it is not a sharing export.".into()})
}
pub fn choose(
    preview: &Preview,
    token: &str,
    index: usize,
    choice: Choice,
    target: Option<&Path>,
    c: &Control,
) -> Result<Project> {
    anyhow::ensure!(
        token == preview.token
            && token
                == fingerprint(&(
                    &preview.input_sha256,
                    &preview.fingerprint,
                    &preview.candidates
                ))?,
        fail("Confirm a fresh exact duplicate preview")
    );
    let candidate = preview
        .candidates
        .get(index)
        .ok_or_else(|| fail("Unknown duplicate selection"))?;
    let current = pipeline::planned_identity(&preview.source, &preview.settings, c)?;
    anyhow::ensure!(
        current.input_sha256 == preview.input_sha256
            && current.fingerprint == preview.fingerprint
            && snapshot(&candidate.root, c)? == preview.files[index],
        fail("Input, recognition identity or private duplicate changed after preview")
    );
    let project = Project::read(candidate.root.clone())?;
    match choice {
        Choice::Reuse => {
            let (reasons, _) = compatible(&project, &current, c)?;
            anyhow::ensure!(
                candidate.compatible && reasons.is_empty(),
                fail(
                    "Duplicate is incompatible; compare it or explicitly create a separate revision"
                )
            );
            Ok(project)
        }
        Choice::SeparateRevision => fork(
            &project,
            target.ok_or_else(|| fail("Choose a new private revision folder"))?,
            &preview.files[index],
            c,
        ),
    }
}
fn copy_file(source: &Path, target: &Path, expected: &File, c: &Control) -> Result<()> {
    use sha2::Digest;
    let mut input = fs::File::open(source)?.take(expected.bytes + 1);
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    let mut hash = sha2::Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        c.check()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        anyhow::ensure!(
            total <= expected.bytes,
            fail("Revision source grew during copy")
        );
        output.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
    }
    output.sync_all()?;
    anyhow::ensure!(
        total == expected.bytes && hex::encode(hash.finalize()) == expected.sha256,
        fail("Revision source changed during copy")
    );
    Ok(())
}
fn fork(project: &Project, target: &Path, files: &[File], c: &Control) -> Result<Project> {
    c.check()?;
    let parent = crate::exports::output_parent(project, target)?;
    fs::create_dir_all(&parent)?;
    let parent = crate::exports::output_parent(project, &parent.join("revision"))?;
    crate::watch::no_links(&parent)?;
    let name = target
        .file_name()
        .ok_or_else(|| fail("Choose a revision folder name"))?;
    let target = parent.join(name);
    crate::watch::no_links(&target)?;
    fs::create_dir(&target)
        .map_err(|_| fail("Revision destination already exists or cannot be created"))?;
    let result: Result<Project> = (|| {
        crate::storage::atomic_json(
            &target.join("revision-copy.pending"),
            &serde_json::json!({"schema":1,"source_snapshot":fingerprint(files)?,"note":"Incomplete private revision; source project remains unchanged. A valid manifest is published last."}),
        )?;
        let manifest = files
            .iter()
            .find(|file| file.relative == "project.json")
            .ok_or_else(|| fail("Duplicate manifest missing"))?;
        let manifest_bytes = crate::migrations::snapshot(&project.root.join("project.json"), c)?
            .ok_or_else(|| fail("Duplicate manifest disappeared"))?;
        let actual = crate::migrations::snapshot_identity(&manifest_bytes);
        anyhow::ensure!(
            actual.bytes == manifest.bytes && actual.sha256 == manifest.sha256,
            fail("Duplicate manifest changed")
        );
        for file in files.iter().filter(|file| file.relative != "project.json") {
            let relative = Path::new(&file.relative);
            anyhow::ensure!(
                !relative.is_absolute()
                    && relative
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_))),
                fail("Unsafe revision file path")
            );
            let path = target.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            crate::watch::no_links(&path)?;
            copy_file(&project.root.join(relative), &path, file, c)?;
        }
        anyhow::ensure!(
            snapshot(&project.root, c)? == files,
            fail("Private project changed before revision publication")
        );
        c.check()?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(target.join("project.json"))?;
        output.write_all(&manifest_bytes)?;
        output.sync_all()?;
        fs::remove_file(target.join("revision-copy.pending"))?;
        let result = Project::read(target.clone())?;
        crate::storage::atomic_json(
            &target.join("revision-origin.json"),
            &serde_json::json!({"schema":1,"parent_snapshot":fingerprint(files)?,"source_input_sha256":project.manifest.input_sha256,"note":"Explicit private separate revision; source identities, readings and exact locks retained."}),
        )?;
        Ok(result)
    })();
    result.map_err(|error|error.context(format!("Private source unchanged; partial revision retained at {}. If incomplete, choose a fresh folder.",target.display())))
}
pub fn compare(first: &Project, second: &Project, c: &Control) -> Result<Comparison> {
    let mut report=Comparison{same_input:first.manifest.input_sha256==second.manifest.input_sha256,same_recognition:first.manifest.fingerprint==second.manifest.fingerprint,active_order_changed:first.manifest.pages!=second.manifest.pages,content_withheld:false,differences:vec![],omitted_differences:0,note:"Read-only comparison of saved active source slots. Covered content is withheld. Different page/source organization is reported, never guessed into an alignment; compare source images explicitly.".into()};
    if crate::redactions::has_masks(first, c)? || crate::redactions::has_masks(second, c)? {
        report.content_withheld = true;
        report.note="Confirmed masks exist in at least one project. Comparison is metadata-only for both projects, including an unmasked counterpart: text, reader evidence and history differences were not computed or emitted. Compare the effective masked views explicitly.".into();
        return Ok(report);
    }
    let mut bytes = 0u64;
    report.omitted_differences = second
        .manifest
        .pages
        .iter()
        .filter(|n| !first.manifest.pages.contains(n))
        .count();
    for n in &first.manifest.pages {
        c.check()?;
        if !second.manifest.pages.contains(n) {
            report.omitted_differences += 1;
            continue;
        }
        let a = crate::redactions::analysis_page(first, &first.load(*n)?, c)?;
        let b = crate::redactions::analysis_page(second, &second.load(*n)?, c)?;
        let source_a = crate::redactions::source_identity(first, &a, c)?;
        let source_b = crate::redactions::source_identity(second, &b, c)?;
        if source_a.sha256 != source_b.sha256 {
            report.omitted_differences += 1;
            continue;
        }
        for difference in crate::review::compare(&a, &b)? {
            let size = serde_json::to_vec(&difference)?.len() as u64;
            if report.differences.len() == 1000 || bytes.saturating_add(size) > 8_000_000 {
                report.omitted_differences += 1;
                continue;
            }
            bytes += size;
            report.differences.push(difference);
        }
    }
    Ok(report)
}
