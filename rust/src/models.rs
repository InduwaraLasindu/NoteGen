//! User-installed GGUF/local vision readers and revision-pinned, verified updates.
use crate::{
    planning::Model,
    storage::{Activity, Control, atomic_json, fingerprint, hash_file, read_json},
    types::*,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hub {
    pub repository: String,
    pub weights: String,
    pub projector: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Update {
    pub id: String,
    pub repository: String,
    pub previous_revision: String,
    pub revision: String,
    pub weights: Asset,
    pub projector: Asset,
}
pub fn validate_hub(h: &Hub) -> Result<()> {
    let parts = h.repository.split('/').collect::<Vec<_>>();
    anyhow::ensure!(
        parts.len() == 2
            && parts.iter().all(|s| !s.is_empty()
                && s.len() <= 120
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_ .".contains(&c) && c != b' ')),
        Fault::new("NG650", "Use a Hugging Face owner/repository")
    );
    for name in [&h.weights, &h.projector] {
        anyhow::ensure!(
            name.ends_with(".gguf")
                && name.len() <= 300
                && name.split('/').all(|s| !s.is_empty()
                    && s != "."
                    && s != ".."
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_ .".contains(&c) && c != b' ')),
            Fault::new("NG650", "Use exact repository-relative GGUF filenames")
        );
    }
    Ok(())
}
pub fn validate_model(m: &Model) -> Result<()> {
    anyhow::ensure!(
        crate::writers::valid_id(&m.id)
            && crate::writers::valid_id(&m.family)
            && !m.name.trim().is_empty()
            && m.name.len() <= 160
            && ["ocr", "vision"].contains(&m.role.as_str())
            && ["structured", "lighton", "glm"].contains(&m.prompt.as_str())
            && m.rank <= 1000
            && m.resident_mib <= 200000
            && m.weights_mib <= 200000
            && m.alias.len() <= 200
            && m.revision.len() <= 100,
        Fault::new(
            "NG650",
            "Invalid reader ID, family, role, prompt protocol or memory estimate"
        )
    );
    if let Some(endpoint) = &m.endpoint {
        local_endpoint(endpoint)?;
        anyhow::ensure!(
            !m.alias.trim().is_empty(),
            Fault::new("NG650", "Enter the exact local server model alias")
        );
    }
    if let Some(h) = &m.hub {
        validate_hub(h)?;
    }
    Ok(())
}
pub fn save_custom(dir: &Path, mut m: Model, c: &Control) -> Result<Model> {
    validate_model(&m)?;
    c.check()?;
    if m.endpoint.is_none() {
        m.alias.clear();
        crate::runtime::verify_gguf(&m.weights)?;
        crate::runtime::verify_gguf(&m.projector)?;
        m.weights = m.weights.canonicalize()?;
        m.projector = m.projector.canonicalize()?;
        m.weights_mib =
            (m.weights.metadata()?.len() + m.projector.metadata()?.len()).div_ceil(1048576);
        m.resident_mib = m.resident_mib.max(m.weights_mib + 512);
        m.sha256 = fingerprint(&(hash_file(&m.weights, c)?, hash_file(&m.projector, c)?))?;
    } else {
        m.weights = PathBuf::new();
        m.projector = PathBuf::new();
        m.sha256 = fingerprint(&(&m.endpoint, &m.alias, &m.revision))?;
        m.weights_mib = 0;
    }
    fs::create_dir_all(dir)?;
    let active = dir.join(format!("{}.notegen.json", m.id));
    if active.exists() {
        let old: Model = read_json(&active, 100_000)?;
        atomic_json(&dir.join(format!("{}.previous.json", m.id)), &old)?;
    }
    atomic_json(&active, &m)?;
    Ok(m)
}
pub fn attach_catalog_metadata(m: &mut Model) {
    let Ok(items) = serde_json::from_str::<Vec<Value>>(include_str!("../downloads.json")) else {
        return;
    };
    if let Some(item) = items
        .iter()
        .find(|v| v["id"].as_str() == Some(m.id.as_str()))
    {
        let url = item["weights"]["url"].as_str().unwrap_or("");
        if let Some(path) = url.strip_prefix("https://huggingface.co/") {
            let mut parts = path.split('/');
            let repository = format!(
                "{}/{}",
                parts.next().unwrap_or(""),
                parts.next().unwrap_or("")
            );
            m.hub = Some(Hub {
                repository,
                weights: item["weights"]["file"].as_str().unwrap_or("").into(),
                projector: item["projector"]["file"].as_str().unwrap_or("").into(),
            });
            if m.revision.is_empty()
                && let (Some(w), Some(p)) = (
                    item["weights"]["sha256"].as_str(),
                    item["projector"]["sha256"].as_str(),
                )
                && fingerprint(&(w, p)).is_ok_and(|v| v == m.sha256)
            {
                m.revision = item["revision"].as_str().unwrap_or("").into();
            }
        }
    }
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(30))
        .user_agent(concat!(
            "NoteGen/",
            env!("CARGO_PKG_VERSION"),
            " model manager"
        ))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let u = attempt.url();
            let h = u.host_str().unwrap_or("");
            if attempt.previous().len() >= 8
                || u.scheme() != "https"
                || !(h == "huggingface.co"
                    || h.ends_with(".huggingface.co")
                    || h.ends_with(".hf.co")
                    || h.ends_with(".amazonaws.com"))
            {
                attempt.error("Model download redirected outside publisher storage")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}
fn update_token_with(
    url: &str,
    read_native: impl FnOnce(&str) -> Result<Option<crate::credentials::Secret>>,
    read_legacy: impl FnOnce() -> Result<Option<crate::credentials::Secret>>,
) -> Result<Option<crate::credentials::Secret>> {
    anyhow::ensure!(
        crate::credentials::publisher_url(url),
        Fault::new(
            "NG750",
            "Update credentials are restricted to the fixed publisher release API"
        )
    );
    match read_native(url) {
        Ok(Some(secret)) => Ok(Some(secret)),
        Ok(None) => read_legacy(),
        Err(native_error) => match read_legacy()? {
            Some(secret) => Ok(Some(secret)),
            None => Err(native_error),
        },
    }
}
/// Legacy environment access is limited to the same publisher boundary as the native store.
pub(crate) fn publisher_update_token(url: &str) -> Result<Option<crate::credentials::Secret>> {
    update_token_with(
        url,
        |url| crate::credentials::native().get_for_publisher(url),
        || match std::env::var("NOTEGEN_GITHUB_TOKEN") {
            Ok(token) => crate::credentials::Secret::new(token).map(Some),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(Fault::new(
                "NG750",
                "The legacy update credential has an invalid format",
            )
            .into()),
        },
    )
}
pub(crate) fn has_publisher_update_credential() -> Result<bool> {
    let legacy_present = std::env::var_os("NOTEGEN_GITHUB_TOKEN").is_some();
    match crate::credentials::native().status() {
        Ok(status) => Ok(status.stored || legacy_present),
        Err(_) if legacy_present => Ok(true),
        Err(error) => Err(error),
    }
}
pub fn parse_update(m: &Model, data: &Value) -> Result<Update> {
    let h = m
        .hub
        .as_ref()
        .ok_or_else(|| Fault::new("NG650", "No update source configured for this reader"))?;
    validate_hub(h)?;
    let revision = data["sha"].as_str().unwrap_or("");
    anyhow::ensure!(
        revision.len() == 40 && revision.bytes().all(|c| c.is_ascii_hexdigit()),
        Fault::new("NG650", "Publisher did not return an immutable revision")
    );
    let siblings = data["siblings"]
        .as_array()
        .ok_or_else(|| Fault::new("NG650", "Publisher file metadata is missing"))?;
    let asset = |name: &str| -> Result<Asset> {
        let entry = siblings
            .iter()
            .find(|v| v["rfilename"].as_str() == Some(name))
            .ok_or_else(|| Fault::new("NG650", format!("Publisher revision is missing {name}")))?;
        let sha = entry["lfs"]["sha256"].as_str().unwrap_or("");
        let bytes = entry["lfs"]["size"]
            .as_u64()
            .or_else(|| entry["size"].as_u64())
            .unwrap_or(0);
        anyhow::ensure!(
            sha.len() == 64
                && sha.bytes().all(|c| c.is_ascii_hexdigit())
                && (4..=100_000_000_000).contains(&bytes),
            Fault::new(
                "NG650",
                "Publisher must provide SHA256 and size for both GGUF files"
            )
        );
        Ok(Asset {
            file: name.into(),
            bytes,
            sha256: sha.to_lowercase(),
        })
    };
    Ok(Update {
        id: m.id.clone(),
        repository: h.repository.clone(),
        previous_revision: m.revision.clone(),
        revision: revision.into(),
        weights: asset(&h.weights)?,
        projector: asset(&h.projector)?,
    })
}
pub fn check(m: &Model, c: &Control) -> Result<Update> {
    c.check()?;
    let h = m
        .hub
        .as_ref()
        .ok_or_else(|| Fault::new("NG650", "Add a publisher repository to enable updates"))?;
    validate_hub(h)?;
    let response = client()?
        .get(format!(
            "https://huggingface.co/api/models/{}/revision/main",
            h.repository
        ))
        .query(&[("blobs", "true")])
        .send()?
        .error_for_status()?;
    let mut raw = vec![];
    response.take(8_000_001).read_to_end(&mut raw)?;
    c.check()?;
    anyhow::ensure!(
        raw.len() <= 8_000_000,
        Fault::new("NG650", "Publisher metadata is oversized")
    );
    parse_update(m, &serde_json::from_slice(&raw)?)
}
pub fn content_range(value: &str, start: u64, total: u64) -> bool {
    let Some(value) = value.strip_prefix("bytes ") else {
        return false;
    };
    let Some((range, size)) = value.split_once('/') else {
        return false;
    };
    let Some((first, last)) = range.split_once('-') else {
        return false;
    };
    first.parse::<u64>().ok() == Some(start)
        && size.parse::<u64>().ok() == Some(total)
        && last.parse::<u64>().is_ok_and(|v| v >= start && v < total)
}
async fn download_async(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    a: &Asset,
    c: &Control,
    log: &Activity,
) -> Result<()> {
    c.check()?;
    if path.is_file() && path.metadata()?.len() == a.bytes && hash_file(path, c)? == a.sha256 {
        log.emit(
            "download",
            format!("{} · verified cached file", a.file),
            0,
            a.bytes as usize,
            a.bytes as usize,
            "",
        );
        return Ok(());
    }
    let part = path.with_file_name(format!(
        "{}.part",
        path.file_name().unwrap().to_string_lossy()
    ));
    fs::create_dir_all(path.parent().unwrap())?;
    let offset = part.metadata().map_or(0, |m| m.len());
    if offset > a.bytes {
        fs::remove_file(&part)?;
    }
    let offset = part.metadata().map_or(0, |m| m.len());
    if offset == a.bytes {
        if hash_file(&part, c)? == a.sha256 {
            if a.file.ends_with(".gguf") {
                crate::runtime::verify_gguf(&part)?;
            }
            fs::rename(part, path)?;
            return Ok(());
        }
        fs::remove_file(&part)?;
    }
    let offset = part.metadata().map_or(0, |m| m.len());
    c.check()?;
    let mut request = client.get(url);
    if crate::credentials::publisher_url(url) {
        request = request.header(reqwest::header::ACCEPT, "application/octet-stream");
        if let Some(token) = publisher_update_token(url)? {
            request = request.bearer_auth(token.expose_secret());
        }
    }
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let sending = request.send();
    tokio::pin!(sending);
    let mut response = loop {
        tokio::select! {r=&mut sending=>break r?.error_for_status()?,_=tokio::time::sleep(Duration::from_millis(100))=>c.check()?}
    };
    let append = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if append {
        anyhow::ensure!(
            content_range(
                response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                offset,
                a.bytes
            ),
            Fault::new("NG650", "Invalid resumed model range")
        );
    }
    anyhow::ensure!(
        append || response.status() == reqwest::StatusCode::OK,
        Fault::new("NG650", "Unexpected model download status")
    );
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(!append)
        .append(append)
        .open(&part)?;
    let mut done = if append { offset } else { 0 };
    let mut last = std::time::Instant::now() - Duration::from_secs(2);
    loop {
        c.check()?;
        let receiving = response.chunk();
        tokio::pin!(receiving);
        let chunk = loop {
            tokio::select! {r=&mut receiving=>break r?,_=tokio::time::sleep(Duration::from_millis(100))=>c.check()?}
        };
        let Some(chunk) = chunk else {
            break;
        };
        done += chunk.len() as u64;
        anyhow::ensure!(
            done <= a.bytes,
            Fault::new("NG650", "Downloaded model exceeds publisher size")
        );
        file.write_all(&chunk)?;
        if last.elapsed() >= Duration::from_millis(250) {
            log.emit(
                "download",
                format!(
                    "{} · {:.1} / {:.1} MiB",
                    a.file,
                    done as f64 / 1048576.,
                    a.bytes as f64 / 1048576.
                ),
                0,
                done as usize,
                a.bytes as usize,
                "",
            );
            last = std::time::Instant::now();
        }
    }
    log.emit(
        "download",
        format!("{} · download complete; verifying SHA256", a.file),
        0,
        done as usize,
        a.bytes as usize,
        "",
    );
    file.sync_all()?;
    drop(file);
    anyhow::ensure!(
        done == a.bytes,
        Fault::new(
            "NG650",
            "Incomplete model download; the partial file can be resumed"
        )
    );
    if hash_file(&part, c)? != a.sha256 {
        fs::remove_file(part)?;
        return Err(Fault::new(
            "NG650",
            "Model checksum mismatch; active reader was preserved",
        )
        .into());
    }
    if a.file.ends_with(".gguf") {
        crate::runtime::verify_gguf(&part)?;
    }
    fs::rename(part, path)?;
    Ok(())
}
pub(crate) fn download(
    url: &str,
    path: &Path,
    a: &Asset,
    c: &Control,
    log: &Activity,
) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .https_only(true)
            .user_agent(concat!("NoteGen/",env!("CARGO_PKG_VERSION")," asset downloader"))
            .read_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(7200))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let u = attempt.url();
                let h = u.host_str().unwrap_or("");
                if attempt.previous().len() >= 8
                    || u.scheme() != "https"
                    || !(h == "huggingface.co"
                        || h == "github.com"
                        || h == "release-assets.githubusercontent.com"
                        || h == "objects.githubusercontent.com"
                        || h.ends_with(".huggingface.co")
                        || h.ends_with(".hf.co")
                        || h.ends_with(".amazonaws.com"))
                {
                    attempt.error("Download left publisher storage")
                } else {
                    attempt.follow()
                }
            }))
            .build()?;
        let u = reqwest::Url::parse(url)?;
        anyhow::ensure!(
            u.scheme() == "https" && (matches!(u.host_str(), Some("huggingface.co" | "github.com"))
                || url.strip_prefix("https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/").is_some_and(|id| !id.is_empty() && id.bytes().all(|b|b.is_ascii_digit()))),
            Fault::new("NG650", "Invalid publisher download URL")
        );
        download_async(&client, url, path, a, c, log).await
    })
}
pub fn install(dir: &Path, m: &Model, u: &Update, c: &Control, log: &Activity) -> Result<Model> {
    validate_model(m)?;
    // Verify metadata identity again before touching disk; update objects can be loaded from files.
    let h = m
        .hub
        .as_ref()
        .ok_or_else(|| Fault::new("NG650", "Reader has no publisher source"))?;
    validate_hub(h)?;
    anyhow::ensure!(
        u.id == m.id
            && u.repository == h.repository
            && u.previous_revision == m.revision
            && u.weights.file == h.weights
            && u.projector.file == h.projector
            && u.revision.len() == 40
            && u.revision.bytes().all(|b| b.is_ascii_hexdigit())
            && [&u.weights, &u.projector].iter().all(|a| a.bytes >= 4
                && a.bytes <= 100_000_000_000
                && a.sha256.len() == 64
                && a.sha256.bytes().all(|b| b.is_ascii_hexdigit())),
        Fault::new("NG650", "Update metadata changed; check again")
    );
    let path = dir.join("versions").join(&m.id).join(&u.revision);
    fs::create_dir_all(&path)?;
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let absolute = std::path::absolute(&path)?;
    if let Some(disk) = disks
        .iter()
        .filter(|d| absolute.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().components().count())
    {
        let required = u
            .weights
            .bytes
            .saturating_add(u.projector.bytes)
            .saturating_add(128 * 1024 * 1024);
        anyhow::ensure!(
            disk.available_space() > required,
            Fault::new(
                "NG650",
                "Not enough free disk space for a safe model update"
            )
        );
    }
    let weights = path.join("weights.gguf");
    let projector = path.join("projector.gguf");
    for (a, p) in [(&u.weights, &weights), (&u.projector, &projector)] {
        download(
            &format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                u.repository, u.revision, a.file
            ),
            p,
            a,
            c,
            log,
        )?;
    }
    c.check()?;
    let mut next = m.clone();
    next.weights = weights;
    next.projector = projector;
    next.revision = u.revision.clone();
    let result = save_custom(dir, next, c)?;
    log.emit(
        "models",
        format!(
            "Activated {} revision {}. Previous reader retained for rollback.",
            m.name,
            &u.revision[..12]
        ),
        0,
        1,
        1,
        "",
    );
    Ok(result)
}
pub fn rollback(dir: &Path, id: &str) -> Result<Model> {
    anyhow::ensure!(
        crate::writers::valid_id(id),
        Fault::new("NG650", "Invalid reader ID")
    );
    let path = dir.join(format!("{id}.previous.json"));
    let previous: Model = read_json(&path, 100_000)?;
    anyhow::ensure!(
        previous.id == id,
        Fault::new("NG650", "Previous reader ID mismatch")
    );
    save_custom(dir, previous, &Control::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};
    const PUBLISHER_URL: &str =
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/latest";
    fn synthetic_secret(label: &str) -> crate::credentials::Secret {
        crate::credentials::Secret::new(format!("ghp_SYNTHETIC_ONLY_{label}_0123456789")).unwrap()
    }
    #[test]
    fn native_update_credential_wins_without_reading_legacy_environment() {
        let token = update_token_with(
            PUBLISHER_URL,
            |_| Ok(Some(synthetic_secret("native"))),
            || panic!("Legacy reader must not run when native credential exists"),
        )
        .unwrap()
        .unwrap();
        assert!(token.expose_secret().contains("native"));
    }
    #[test]
    fn missing_or_failed_native_store_can_use_explicit_legacy_credential() {
        for native in [
            Ok(None),
            Err(Fault::new("NG750", "Synthetic native store unavailable").into()),
        ] {
            let token = update_token_with(
                PUBLISHER_URL,
                |_| native,
                || Ok(Some(synthetic_secret("legacy"))),
            )
            .unwrap()
            .unwrap();
            assert!(token.expose_secret().contains("legacy"));
        }
        assert!(
            update_token_with(PUBLISHER_URL, |_| Ok(None), || Ok(None))
                .unwrap()
                .is_none()
        );
        assert!(
            update_token_with(
                PUBLISHER_URL,
                |_| Err(Fault::new("NG750", "Synthetic failure").into()),
                || Ok(None)
            )
            .is_err()
        );
    }
    #[test]
    fn other_publishers_cannot_trigger_native_or_legacy_credential_reads() {
        for url in [
            "https://huggingface.co/owner/model",
            "https://api.github.com/repos/Other/Repo/releases/latest",
            "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/not-a-number",
        ] {
            assert!(
                update_token_with(
                    url,
                    |_| panic!("Unexpected native credential read"),
                    || panic!("Unexpected legacy credential read")
                )
                .is_err()
            );
        }
    }
    #[test]
    fn authorization_is_removed_when_redirect_changes_origin() {
        fn capture(mut socket: std::net::TcpStream) -> String {
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = vec![];
            let mut buffer = [0u8; 1024];
            while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 16000);
            }
            String::from_utf8(bytes).unwrap()
        }
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let second = TcpListener::bind("127.0.0.1:0").unwrap();
        let first_url = format!("http://{}", first.local_addr().unwrap());
        let next = format!("http://{}", second.local_addr().unwrap());
        let from = thread::spawn(move || {
            let (mut socket, _) = first.accept().unwrap();
            let request = capture(socket.try_clone().unwrap());
            write!(socket,"HTTP/1.1 302 Found\r\nLocation: {next}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            request
        });
        let to = thread::spawn(move || {
            let (mut socket, _) = second.accept().unwrap();
            let request = capture(socket.try_clone().unwrap());
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            )
            .unwrap();
            request
        });
        let response = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(first_url)
            .bearer_auth("SYNTHETIC_REDIRECT_TEST_ONLY")
            .send()
            .unwrap();
        assert!(response.status().is_success());
        let initial = from.join().unwrap().to_ascii_lowercase();
        let redirected = to.join().unwrap().to_ascii_lowercase();
        assert!(initial.contains("authorization: bearer synthetic_redirect_test_only"));
        assert!(!redirected.contains("authorization:"));
        assert!(!redirected.contains("synthetic_redirect_test_only"));
    }
    fn mock(body: Vec<u8>, range: Option<&str>) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let range = range.map(str::to_string);
        let t = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut raw = vec![];
            let mut b = [0; 1];
            while socket.read(&mut b).unwrap_or(0) > 0 {
                raw.push(b[0]);
                if raw.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let status = if range.is_some() {
                "206 Partial Content"
            } else {
                "200 OK"
            };
            let header = range
                .map(|s| format!("Content-Range: {s}\r\n"))
                .unwrap_or_default();
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{header}Connection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            let _ = socket.write_all(&body);
            String::from_utf8_lossy(&raw).into_owned()
        });
        (url, t)
    }
    #[test]
    fn downloads_resume_verify_and_never_activate_bad_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"GGUFtest-verified-model";
        let a = Asset {
            file: "reader.gguf".into(),
            bytes: bytes.len() as u64,
            sha256: hex::encode(sha2::Sha256::digest(bytes)),
        };
        let path = dir.path().join("reader.gguf");
        fs::write(path.with_extension("gguf.part"), &bytes[..8]).unwrap();
        let (url, t) = mock(
            bytes[8..].to_vec(),
            Some(&format!("bytes 8-{}/{}", bytes.len() - 1, bytes.len())),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            download_async(
                &reqwest::Client::builder().no_proxy().build().unwrap(),
                &url,
                &path,
                &a,
                &Control::default(),
                &Activity::default(),
            )
            .await
            .unwrap()
        });
        assert!(t.join().unwrap().to_lowercase().contains("range: bytes=8-"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(&path).unwrap();
        let (url, t) = mock(vec![b'X'; bytes.len()], None);
        let error = rt.block_on(async {
            download_async(
                &reqwest::Client::builder().no_proxy().build().unwrap(),
                &url,
                &path,
                &a,
                &Control::default(),
                &Activity::default(),
            )
            .await
            .unwrap_err()
        });
        assert!(error.to_string().contains("checksum"));
        assert!(!path.exists());
        t.join().unwrap();
    }
    #[test]
    fn oversized_parts_restart_and_ignored_range_responses_replace_partial_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"GGUFvalid-download-bytes";
        let a = Asset {
            file: "reader.gguf".into(),
            bytes: bytes.len() as u64,
            sha256: hex::encode(sha2::Sha256::digest(bytes)),
        };
        let path = dir.path().join("reader.gguf");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for partial in [vec![b'X'; 100], bytes[..8].to_vec()] {
            fs::write(path.with_extension("gguf.part"), partial).unwrap();
            let (url, t) = mock(bytes.to_vec(), None);
            rt.block_on(download_async(
                &reqwest::Client::builder().no_proxy().build().unwrap(),
                &url,
                &path,
                &a,
                &Control::default(),
                &Activity::default(),
            ))
            .unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes);
            fs::remove_file(&path).unwrap();
            t.join().unwrap();
        }
        let c = Control::default();
        c.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            rt.block_on(download_async(
                &reqwest::Client::new(),
                "http://127.0.0.1:1",
                &path,
                &a,
                &c,
                &Activity::default()
            ))
            .is_err()
        );
        assert!(!path.exists());
    }
    use sha2::Digest;
}
