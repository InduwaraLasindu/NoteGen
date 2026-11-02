//! Publisher metadata checks and transactional local dependency updates.
use crate::{models, runtime, setup, storage::*, types::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io::Read, path::PathBuf, time::Duration};

const RELEASES: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=20";
const RELEASE_LIST_LIMIT: usize = 20;
const RELEASE_NOTES_LIMIT: usize = 6000;
const RELEASE_NOTES_BYTES: usize = RELEASE_NOTES_LIMIT * 4 + 100;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeRelease {
    pub build: String,
    pub assets: Vec<setup::Download>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelChange {
    pub model: crate::planning::Model,
    pub update: models::Update,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppRelease {
    pub version: String,
    pub asset: setup::Download,
    #[serde(default)]
    pub asset_id: u64,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub release_notes: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub checked_at: String,
    #[serde(default)]
    pub channel: UpdateChannel,
    pub models: Vec<ModelChange>,
    pub runtime: Option<RuntimeRelease>,
    pub app: Option<AppRelease>,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckStamp {
    pub checked_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub channel: UpdateChannel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckMode {
    Automatic,
    Manual,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckDecision {
    Due,
    Disabled,
    Deferred(chrono::DateTime<chrono::Utc>),
    Recent(chrono::DateTime<chrono::Utc>),
}
/// Decide whether to begin a metadata check, without network or filesystem access.
/// An explicit manual check bypasses automatic-check preferences and deferral.
pub fn check_decision(
    preferences: &ExperienceOptions,
    last: Option<&CheckStamp>,
    now: chrono::DateTime<chrono::Utc>,
    mode: CheckMode,
) -> CheckDecision {
    if mode == CheckMode::Manual {
        return CheckDecision::Due;
    }
    if !preferences.check_updates {
        return CheckDecision::Disabled;
    }
    if let Some(until) = preferences.defer_updates_until.filter(|until| *until > now) {
        return CheckDecision::Deferred(until);
    }
    if let Some(last) = last.filter(|last| {
        last.channel == preferences.update_channel
            && last.checked_at <= now
            && now.signed_duration_since(last.checked_at) < chrono::Duration::hours(24)
    }) {
        return CheckDecision::Recent(last.checked_at + chrono::Duration::hours(24));
    }
    CheckDecision::Due
}
pub fn app_metadata_url(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => crate::credentials::PUBLISHER_LATEST_URL,
        UpdateChannel::Preview => crate::credentials::PUBLISHER_RELEASES_URL,
    }
}
impl Report {
    pub fn count(&self) -> usize {
        self.models.len() + usize::from(self.runtime.is_some()) + usize::from(self.app.is_some())
    }
    pub fn download_bytes(&self) -> u64 {
        self.models
            .iter()
            .map(|m| m.update.weights.bytes + m.update.projector.bytes)
            .sum::<u64>()
            + self
                .runtime
                .as_ref()
                .map_or(0, |r| r.assets.iter().map(|a| a.bytes).sum())
    }
}
fn metadata(url: &str, c: &Control) -> Result<Value> {
    c.check()?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("NoteGen/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut bytes = vec![];
    let mut request = client.get(url);
    if crate::credentials::publisher_metadata_url(url)
        && let Some(token) = models::publisher_update_token(url)?
    {
        request = request.bearer_auth(token.expose_secret());
    }
    request
        .send()?
        .error_for_status()?
        .take(4_000_001)
        .read_to_end(&mut bytes)?;
    c.check()?;
    anyhow::ensure!(
        bytes.len() <= 4_000_000,
        "Publisher metadata exceeds the limit"
    );
    Ok(serde_json::from_slice(&bytes)?)
}
fn digest(v: &Value) -> Result<String> {
    let h = v["digest"]
        .as_str()
        .unwrap_or("")
        .strip_prefix("sha256:")
        .unwrap_or("");
    anyhow::ensure!(
        h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()),
        "Publisher asset has no SHA-256 digest"
    );
    Ok(h.to_ascii_lowercase())
}
pub fn validate_runtime_release(build: &str, assets: &[setup::Download]) -> Result<()> {
    anyhow::ensure!(
        build.strip_prefix('b').is_some_and(|n| !n.is_empty()
            && n.len() <= 12
            && n.bytes().all(|b| b.is_ascii_digit())),
        "Invalid runtime build"
    );
    anyhow::ensure!(
        !assets.is_empty() && assets.len() <= 6,
        "Invalid runtime asset count"
    );
    let prefix = format!("https://github.com/ggml-org/llama.cpp/releases/download/{build}/");
    let mut files = std::collections::BTreeSet::new();
    for a in assets {
        let permitted = ["cpu", "vulkan", "cuda-12.4", "cuda-13.4"]
            .iter()
            .any(|backend| {
                a.file == format!("llama-{build}-bin-win-{backend}-x64.zip")
                    || backend.starts_with("cuda")
                        && a.file == format!("cudart-llama-bin-win-{backend}-x64.zip")
            });
        anyhow::ensure!(
            permitted
                && files.insert(&a.file)
                && a.url == format!("{prefix}{}", a.file)
                && a.bytes > 1024
                && a.bytes <= 2_000_000_000
                && a.sha256.len() == 64
                && a.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid runtime asset metadata"
        );
    }
    Ok(())
}
pub fn parse_runtime(data: &Value) -> Result<RuntimeRelease> {
    for r in data
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid runtime release list"))?
    {
        if r["draft"].as_bool().unwrap_or(true) {
            continue;
        }
        let build = r["tag_name"].as_str().unwrap_or("");
        if !build.starts_with('b') {
            continue;
        }
        let Some(raw) = r["assets"].as_array() else {
            continue;
        };
        let mut assets = vec![];
        for a in raw {
            let file = a["name"].as_str().unwrap_or("");
            if !file.ends_with("x64.zip") || !file.contains("win-") {
                continue;
            }
            let candidate = setup::Download {
                file: file.into(),
                bytes: a["size"].as_u64().unwrap_or(0),
                sha256: digest(a)?,
                url: a["browser_download_url"].as_str().unwrap_or("").into(),
            };
            // Ignore other architectures and unrecognized release bundles.
            if validate_runtime_release(build, std::slice::from_ref(&candidate)).is_ok() {
                assets.push(candidate);
            }
        }
        if assets.iter().any(|a| a.file.contains("win-cpu-x64"))
            && assets.iter().any(|a| a.file.contains("win-vulkan-x64"))
        {
            validate_runtime_release(build, &assets)?;
            return Ok(RuntimeRelease {
                build: build.into(),
                assets,
            });
        }
    }
    anyhow::bail!("No verified Windows x64 runtime release found")
}
fn version(s: &str) -> Result<semver::Version> {
    anyhow::ensure!(s.len() <= 128, "App version exceeds the limit");
    let parsed = semver::Version::parse(s)
        .map_err(|_| anyhow::anyhow!("App release has an invalid semantic version"))?;
    anyhow::ensure!(
        parsed.build.is_empty() && parsed.to_string() == s,
        "App release version must be canonical and have no build metadata"
    );
    Ok(parsed)
}
fn plain_note_character(character: char) -> bool {
    (!character.is_control() || matches!(character, '\n' | '\t'))
        && !matches!(character, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
}
fn release_notes(value: &Value) -> Result<String> {
    let text = match value {
        Value::Null => "",
        Value::String(text) => text,
        _ => anyhow::bail!("Publisher release notes must be plain text"),
    };
    let mut result = String::new();
    for (count, character) in text
        .chars()
        .filter(|c| plain_note_character(*c))
        .enumerate()
    {
        if count == RELEASE_NOTES_LIMIT {
            result.push_str("\n[Release notes shortened for display.]");
            break;
        }
        result.push(character);
    }
    Ok(result)
}
/// Revalidate saved update reports before any installer download or execution.
pub fn validate_app_release(release: &AppRelease) -> Result<()> {
    let parsed = version(&release.version)?;
    let tag = format!("v{}", release.version);
    let expected_file = format!("NoteGen-Setup-{}.exe", release.version);
    anyhow::ensure!(
        (parsed.pre.is_empty() || release.prerelease)
            && release.asset.file == expected_file
            && release.asset.url
                == format!(
                    "https://github.com/InduwaraLasindu/NoteGen/releases/download/{tag}/{expected_file}"
                )
            && release.asset.bytes > 1024
            && release.asset.bytes < 1_000_000_000
            && release.asset.sha256.len() == 64
            && release.asset.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            && release.release_notes.len() <= RELEASE_NOTES_BYTES
            && release.release_notes.chars().all(plain_note_character),
        "Invalid installer update metadata"
    );
    Ok(())
}
pub fn app_compatibility(release: &AppRelease) -> &'static str {
    if release.prerelease {
        "Preview Windows x64 installer. Preview builds may change settings or project formats. Keep a backup before installing. No project version is pinned automatically."
    } else {
        "Windows x64 installer. Bundled PDFium, Tesseract and runtime dependencies update with the app. No project version is pinned automatically."
    }
}
/// Select the highest newer semantic version from at most 20 publisher releases.
/// Stable excludes both flagged previews and semantic prerelease versions.
/// Preview is an explicit opt-in; it can also select a newer stable release.
pub fn select_app_release(
    data: &Value,
    channel: UpdateChannel,
    installed_version: &str,
) -> Result<Option<AppRelease>> {
    let installed = version(installed_version)?;
    let releases = match data {
        Value::Array(releases) => releases.as_slice(),
        Value::Object(_) => std::slice::from_ref(data),
        _ => anyhow::bail!("Invalid app release metadata"),
    };
    anyhow::ensure!(
        releases.len() <= RELEASE_LIST_LIMIT,
        "App release list exceeds the limit"
    );
    let mut selected: Option<(semver::Version, AppRelease)> = None;
    let mut seen = std::collections::BTreeSet::new();
    for data in releases {
        match data["draft"].as_bool() {
            Some(true) => continue,
            Some(false) => (),
            None => anyhow::bail!("App release has no valid draft status"),
        }
        let tag = data["tag_name"].as_str().unwrap_or("");
        let parsed = version(
            tag.strip_prefix('v')
                .ok_or_else(|| anyhow::anyhow!("App release tag must start with v"))?,
        )?;
        let flagged_preview = data["prerelease"]
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("App release has no valid preview status"))?;
        let prerelease = flagged_preview || !parsed.pre.is_empty();
        if parsed <= installed || channel == UpdateChannel::Stable && prerelease {
            continue;
        }
        anyhow::ensure!(seen.insert(parsed.clone()), "Duplicate app release version");
        let file = format!("NoteGen-Setup-{parsed}.exe");
        let assets = data["assets"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("App release has no assets"))?;
        anyhow::ensure!(
            assets.len() <= 128,
            "App release asset list exceeds the limit"
        );
        let mut matching = assets
            .iter()
            .filter(|asset| asset["name"].as_str() == Some(&file));
        let asset = matching
            .next()
            .ok_or_else(|| anyhow::anyhow!("App release has no installer"))?;
        anyhow::ensure!(matching.next().is_none(), "Duplicate app release installer");
        let release = AppRelease {
            version: parsed.to_string(),
            asset: setup::Download {
                file,
                bytes: asset["size"].as_u64().unwrap_or(0),
                sha256: digest(asset)?,
                url: asset["browser_download_url"].as_str().unwrap_or("").into(),
            },
            asset_id: asset["id"]
                .as_u64()
                .filter(|id| *id > 0)
                .ok_or_else(|| anyhow::anyhow!("App installer has no valid asset identifier"))?,
            prerelease,
            release_notes: release_notes(&data["body"])?,
        };
        validate_app_release(&release)?;
        if selected
            .as_ref()
            .is_none_or(|(highest, _)| parsed > *highest)
        {
            selected = Some((parsed, release));
        }
    }
    Ok(selected.map(|(_, release)| release))
}
pub fn parse_app(data: &Value) -> Result<Option<AppRelease>> {
    select_app_release(data, UpdateChannel::Stable, env!("CARGO_PKG_VERSION"))
}
pub fn check(s: &Settings, c: &Control, log: &Activity) -> Result<Report> {
    let mut report = Report {
        checked_at: chrono::Utc::now().to_rfc3339(),
        channel: s.experience.update_channel,
        ..Report::default()
    };
    let (installed, warnings) = runtime::inventory_available(&s.models_dir)?;
    report.errors.extend(warnings);
    for mut m in installed {
        c.check()?;
        models::attach_catalog_metadata(&mut m);
        if m.endpoint.is_some() || m.hub.is_none() {
            report
                .notes
                .push(format!("{}: updates managed by its provider", m.name));
            continue;
        }
        log.emit(
            "updates",
            format!("Checking {} publisher revision", m.name),
            0,
            0,
            0,
            "",
        );
        match models::check(&m, c) {
            Ok(u) if u.revision != m.revision => report.models.push(ModelChange {
                model: m,
                update: u,
            }),
            Ok(_) => report.notes.push(format!("{} is current", m.name)),
            Err(e) => {
                c.check()?;
                report.errors.push(format!("{}: {e:#}", m.name));
            }
        }
    }
    if cfg!(windows) {
        match metadata(RELEASES, c).and_then(|v| parse_runtime(&v)) {
            Ok(r) => {
                let active = setup::saved_runtime(&setup::runtime_root(s))
                    .map(|r| r.build)
                    .unwrap_or_else(|| "b12032".into());
                if r.build[1..].parse::<u64>().unwrap_or(0)
                    > active.trim_start_matches('b').parse::<u64>().unwrap_or(0)
                {
                    report.runtime = Some(r);
                } else {
                    report.notes.push(format!("AI runtime {active} is current"));
                }
            }
            Err(e) => {
                c.check()?;
                report.errors.push(format!("AI runtime: {e:#}"));
            }
        }
    } else {
        report
            .notes
            .push("Native runtime updates are managed by your system on this platform".into());
    }
    match metadata(app_metadata_url(s.experience.update_channel), c).and_then(|v| {
        select_app_release(&v, s.experience.update_channel, env!("CARGO_PKG_VERSION"))
    }) {
        Ok(app) => report.app = app,
        Err(e) => {
            c.check()?;
            report.errors.push(format!("NoteGen app: {e:#}"));
        }
    }
    report.notes.push("PDFium, Tesseract, language data and Visual C++ dependencies are serviced together through verified NoteGen installer updates.".into());
    Ok(report)
}
pub fn apply(s: &Settings, report: &Report, c: &Control, log: &Activity) -> Result<Settings> {
    use crate::progress::{Phase, Work};
    let total = report.download_bytes().div_ceil(1024).max(1) as usize;
    let actions = report.models.len() + usize::from(report.runtime.is_some());
    log.work(
        Work::new(Phase::Download, 0, total, 2, false),
        "Preparing verified component updates",
    );
    log.work(
        Work::new(Phase::Activate, 0, actions, 1500, false),
        "Reserving component verification work",
    );
    let progress = std::sync::Mutex::new(std::collections::BTreeMap::<String, usize>::new());
    let outer = log.clone();
    let events = Activity {
        path: None,
        callback: Some(std::sync::Arc::new(move |e| {
            if e.stage == "download" {
                let name = e
                    .message
                    .split_whitespace()
                    .next()
                    .unwrap_or("asset")
                    .to_string();
                let mut bytes = progress.lock().unwrap();
                bytes.insert(name, e.done);
                let done = bytes.values().copied().sum::<usize>() / 1024;
                outer.work(
                    Work::new(Phase::Download, done, total, 2, true),
                    e.message.clone(),
                );
            }
            outer.publish(e);
        })),
    };
    let mut next = s.clone();
    if let Some(r) = &report.runtime {
        let active = setup::install_runtime_release(s, &r.build, &r.assets, c, &events)?;
        next.server = active.server;
        next.backend = active.backend;
        // Persist each successful activation, even if a later model is cancelled.
        atomic_json(&PathBuf::from("ai-settings.json"), &next)?;
    }
    for (index, change) in report.models.iter().enumerate() {
        c.check()?;
        models::install(&s.models_dir, &change.model, &change.update, c, &events)?;
        log.work(
            Work::new(Phase::Activate, index + 1, actions, 1500, false),
            format!("{} updated and verified", change.model.name),
        );
    }
    log.work(
        Work::new(Phase::Download, total, total, 2, false),
        "All selected component files verified",
    );
    log.work(
        Work::new(Phase::Activate, actions, actions, 1500, false),
        "AI updates complete",
    );
    Ok(next)
}
pub fn rollback_runtime(s: &Settings, c: &Control) -> Result<Settings> {
    let root = setup::runtime_root(s);
    let previous: setup::RuntimeRecord =
        read_json(&root.join("previous-ai-runtime.json"), 100_000)?;
    let path = previous.server.canonicalize()?;
    anyhow::ensure!(
        path.starts_with(root.canonicalize()?.join("ai-backends"))
            && path.file_name().is_some_and(|n| n == "llama-server.exe"),
        "Previous runtime is outside the managed folder"
    );
    let (ok, _, _) = crate::scan::run(
        &path,
        &["--list-devices".into()],
        Duration::from_secs(30),
        c,
    )?;
    anyhow::ensure!(ok, "Previous runtime no longer starts");
    if let Some(active) = setup::saved_runtime(&root) {
        atomic_json(&root.join("previous-ai-runtime.json"), &active)?;
    }
    atomic_json(&root.join("active-ai-runtime.json"), &previous)?;
    let mut next = s.clone();
    next.server = previous.server;
    next.backend = previous.backend;
    atomic_json(&PathBuf::from("ai-settings.json"), &next)?;
    Ok(next)
}
pub fn download_app(release: &AppRelease, c: &Control, log: &Activity) -> Result<PathBuf> {
    // Revalidate reports loaded from disk before executing their contents.
    validate_app_release(release)?;
    let root = PathBuf::from("updates");
    fs::create_dir_all(&root)?;
    let path = root.join(&release.asset.file);
    let url = if release.asset_id > 0 && models::has_publisher_update_credential()? {
        format!(
            "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/{}",
            release.asset_id
        )
    } else {
        release.asset.url.clone()
    };
    models::download(
        &url,
        &path,
        &models::Asset {
            file: release.asset.file.clone(),
            bytes: release.asset.bytes,
            sha256: release.asset.sha256.clone(),
        },
        c,
        log,
    )?;
    Ok(path.canonicalize()?)
}
