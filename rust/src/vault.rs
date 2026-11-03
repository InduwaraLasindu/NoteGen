//! Opt-in authenticated workspace snapshots. age owns encryption, KDFs and nonces.
//! Original projects are never adopted for cleanup. Unlocked/native/OS temporary
//! files, pagefile and crash dumps are outside encrypted-at-rest protection.
use crate::{
    sessions::{self, Session},
    storage::{Activity, Control},
    types::*,
    watch::no_links,
};
use age::secrecy::ExposeSecret;
pub use age::secrecy::SecretString;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant, SystemTime},
};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"NoteGen-Encrypted-Workspace-1\n";
const PRIVATE_MAGIC: &[u8] = b"NoteGen-Private-Snapshot-1\n";
const HEADER_LIMIT: usize = 32 * 1024;
const AGE_HEADER_LIMIT: usize = 64 * 1024;
const WRAPPER_LIMIT: usize = 16 * 1024;
const MANIFEST_LIMIT: usize = 16 * 1024 * 1024;
const HARD_FILES: usize = 50_000;
const HARD_FILE_BYTES: u64 = 512 * 1024 * 1024;
const HARD_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const WORK_MARKER: &str = "notegen-vault-work.json";
pub const LIMITS_NOTE: &str = "Only the selected project snapshot and managed source/work copy are encrypted. Originals, external references and exported documents remain where you put them. Unlocked files, native/OS temporary files, pagefile and crash dumps are not protected by this container. Cleanup is ordinary deletion, not secure erasure. Unknown, changed or busy work files are retained and reported. Older encrypted backups remain readable with their old keys after rotation.";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub scrypt_log_n: u8,
    /// Explicit offline/export choice only; the default fails closed if private ACLs fail.
    pub allow_inherited_permissions: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_files: HARD_FILES,
            max_file_bytes: HARD_FILE_BYTES,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            scrypt_log_n: 18,
            allow_inherited_permissions: false,
        }
    }
}
impl Options {
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            (1..=HARD_FILES).contains(&self.max_files)
                && (1..=HARD_FILE_BYTES).contains(&self.max_file_bytes)
                && (1..=HARD_TOTAL_BYTES).contains(&self.max_total_bytes)
                && (16..=18).contains(&self.scrypt_log_n),
            fail("Invalid encrypted workspace resource limits")
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub schema: u32,
    pub vault_id: String,
    pub revision: u64,
    pub header_sha256: String,
    pub recovery_recipient: String,
    pub project_relative: String,
    pub original_project_root: Option<PathBuf>,
    pub directories: Vec<String>,
    pub files: Vec<FileEntry>,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub path: PathBuf,
    pub vault_id: String,
    pub revision: u64,
    pub bytes: u64,
    pub sha256: String,
    pub previous: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryReceipt {
    pub path: PathBuf,
    pub recipient: String,
    pub sha256: String,
    pub note: String,
    pub private_permissions: bool,
}
pub enum Credentials<'a> {
    Passphrase(&'a SecretString),
    RecoveryKey(&'a Path),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Retained {
    pub path: PathBuf,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockReport {
    pub receipt: Receipt,
    pub removed: Vec<PathBuf>,
    pub retained: Vec<Retained>,
    pub fully_locked: bool,
    pub cancelled: bool,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetainedCopy {
    pub root: PathBuf,
    pub vault: PathBuf,
    pub vault_id: String,
    pub base_sha256: String,
    pub retained: Vec<Retained>,
    pub note: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    schema: u32,
    vault_id: String,
    wrapped_workspace_key: String,
}
struct Access {
    header: Header,
    identity: age::x25519::Identity,
    inventory: Inventory,
    receipt: Receipt,
    recovery: bool,
}
struct SourceFile {
    source: PathBuf,
    entry: FileEntry,
    modified: SystemTime,
    held: Option<File>,
}
struct Snapshot {
    files: Vec<SourceFile>,
    directories: Vec<String>,
    observed_directories: Vec<(PathBuf, SystemTime)>,
    total_bytes: u64,
}
enum SnapshotReader<'a> {
    Held(&'a File),
    Opened(File),
}
impl SnapshotReader<'_> {
    fn file(&self) -> &File {
        match self {
            Self::Held(file) => file,
            Self::Opened(file) => file,
        }
    }
}
impl Read for SnapshotReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let mut file = self.file();
        file.read(buffer)
    }
}
fn source_reader(source: &SourceFile) -> Result<SnapshotReader<'_>> {
    no_links(&source.source)?;
    let reader = if let Some(file) = &source.held {
        SnapshotReader::Held(file)
    } else {
        SnapshotReader::Opened(File::open(&source.source).map_err(|error| {
            fail(&format!(
                "Cannot open snapshot data file (OS error {:?})",
                error.raw_os_error()
            ))
        })?)
    };
    // Windows byte-range locks are mandatory for other handles, including ones
    // opened by this process. Read the held control file through its owning
    // handle; keep the exclusion intact during hashing/encryption/verification.
    let mut file = reader.file();
    file.seek(SeekFrom::Start(0))?;
    Ok(reader)
}
fn source_hash(source: &SourceFile, c: &Control) -> Result<String> {
    let mut reader = source_reader(source)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        c.check()?;
        let count = reader.read(&mut buffer).map_err(|error| {
            fail(&format!(
                "Snapshot source hashing failed (OS error {:?})",
                error.raw_os_error()
            ))
        })?;
        if count == 0 {
            break;
        }
        total += count as u64;
        anyhow::ensure!(
            total <= source.entry.bytes,
            fail("Snapshot source grew while hashing")
        );
        hash.update(&buffer[..count]);
    }
    anyhow::ensure!(
        total == source.entry.bytes,
        fail("Snapshot source shrank while hashing")
    );
    Ok(hex::encode(hash.finalize()))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkMarker {
    schema: u32,
    kind: String,
    root: PathBuf,
    vault: PathBuf,
    vault_id: String,
    base_sha256: String,
    id: String,
}
#[derive(Default)]
struct WorkerState {
    active: usize,
    closing: bool,
}
struct Workers {
    state: Mutex<WorkerState>,
    changed: Condvar,
}
pub struct WorkerGuard {
    workers: Arc<Workers>,
    _file: Option<File>,
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let mut state = self.workers.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active = state.active.saturating_sub(1);
        self.workers.changed.notify_all();
    }
}
/// A new marked work copy, never a user's original project. Call commit_and_lock
/// before closing. Dropping a handle preserves plaintext for explicit recovery.
pub struct WorkCopy {
    session: Session,
    vault: PathBuf,
    _lease: Option<File>,
    header: Header,
    workspace_key: Option<age::x25519::Identity>,
    previous_key: Option<age::x25519::Identity>,
    inventory: Inventory,
    base_sha256: String,
    options: Options,
    owner_hashes: BTreeMap<String, String>,
    workers: Arc<Workers>,
    closed: bool,
    private_permissions: bool,
}
impl Drop for WorkCopy {
    fn drop(&mut self) {
        let mut state = self.workers.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closing = true;
        // Never recursively delete a possibly unsaved, changed or busy copy here.
    }
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG682", message).into()
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn hash_file(path: &Path, c: &Control) -> Result<String> {
    no_links(path)?;
    let mut file = File::open(path)?;
    let limit = HARD_TOTAL_BYTES + MANIFEST_LIMIT as u64 + 64 * 1024 * 1024;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        fail("Workspace file exceeds its bounded byte budget")
    );
    let mut count = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        c.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        anyhow::ensure!(
            count <= limit,
            fail("Workspace file grew beyond its byte budget")
        );
        hash.update(&buffer[..n]);
    }
    no_links(path)?;
    Ok(hex::encode(hash.finalize()))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
}
fn secret_ok(passphrase: &SecretString) -> Result<()> {
    let n = passphrase.expose_secret().len();
    anyhow::ensure!(
        (12..=1024).contains(&n),
        fail("Choose a passphrase of 12–1,024 UTF-8 bytes")
    );
    Ok(())
}
fn portable(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && path.split('/').count() <= 16
        && path.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 240
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|ch| ch.is_control() || "\\:<>\"|?*".contains(ch))
                && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                && !matches!(
                    stem.strip_prefix("COM")
                        .or_else(|| stem.strip_prefix("LPT")),
                    Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                )
        })
}
fn path_key(path: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    path.nfkc()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect()
}
fn data_path(path: &str) -> bool {
    portable(path) && (path.starts_with("work/") || path.starts_with("source/"))
}
fn original_location_valid(path: &Path) -> bool {
    // Historical metadata can originate on another OS. It is never opened or
    // adopted; only a matching local canonical prefix is used to reject targets.
    let Some(text) = path.to_str() else {
        return false;
    };
    let bytes = text.as_bytes();
    let absolute = path.is_absolute()
        || text.starts_with('/')
        || text.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'));
    absolute
        && text.len() <= 32768
        && !text.chars().any(char::is_control)
        && !text.split(['/', '\\']).any(|part| part == "..")
}
fn relative(path: &Path) -> Result<String> {
    anyhow::ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        fail("Unsafe workspace-relative path")
    );
    let value = path
        .iter()
        .map(|p| {
            p.to_str()
                .ok_or_else(|| fail("Workspace names must be valid Unicode"))
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    anyhow::ensure!(
        portable(&value),
        fail("Workspace filename is not portable or exceeds its path budget")
    );
    Ok(value)
}
fn parent(path: &Path) -> Result<PathBuf> {
    let value = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    no_links(value)?;
    let absolute = std::path::absolute(value)?;
    fs::create_dir_all(&absolute)?;
    no_links(&absolute)?;
    Ok(absolute.canonicalize()?)
}
fn target(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| fail("Choose an encrypted workspace filename"))?;
    let result = parent(path)?.join(name);
    no_links(&result)?;
    Ok(result)
}
fn lease(path: &Path) -> Result<File> {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    let lock = PathBuf::from(name);
    no_links(&lock)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() == 0,
        fail("Workspace lock must be an empty regular file")
    );
    file.try_lock()
        .map_err(|_| fail("Encrypted workspace is busy or already unlocked"))?;
    Ok(file)
}
fn preview_target(path: &Path) -> Result<PathBuf> {
    let mut ancestor = std::path::absolute(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    no_links(&ancestor)?;
    let mut missing = vec![];
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or_else(|| fail("Cannot resolve workspace destination"))?
                .to_os_string(),
        );
        anyhow::ensure!(ancestor.pop(), fail("Cannot resolve workspace destination"));
    }
    let mut resolved = ancestor.canonicalize()?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved.join(
        path.file_name()
            .ok_or_else(|| fail("Choose a workspace filename"))?,
    ))
}
fn recovery_identity(path: &Path) -> Result<age::x25519::Identity> {
    no_links(path)?;
    let file = File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 1024,
        fail("Recovery key must be a small regular key file")
    );
    let mut bytes = Zeroizing::new(vec![]);
    file.take(1025).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 1024, fail("Recovery key is too large"));
    let text = std::str::from_utf8(&bytes).map_err(|_| fail("Invalid recovery key"))?;
    text.trim()
        .parse()
        .map_err(|_| fail("Invalid recovery key"))
}
#[cfg(unix)]
fn private_permissions(path: &Path, directory: bool, _c: &Control) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    anyhow::ensure!(
        fs::metadata(path)?.permissions().mode() & 0o777 == mode,
        fail("Could not enforce private workspace permissions")
    );
    Ok(())
}
#[cfg(windows)]
fn private_permissions(path: &Path, directory: bool, c: &Control) -> Result<()> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    // Static script; paths are passed as data through a child-only environment
    // variable, never interpolated into shell code. Only this new empty file or
    // app-created directory is changed, before any secret content is written.
    // Use Windows PowerShell's .NET Framework APIs directly: an inherited pwsh
    // PSModulePath can make Set-Acl import an incompatible PowerShell 7 module.
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
try {
$p=$env:NOTEGEN_VAULT_PERMISSION_TARGET
$sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User
$rights=[System.Security.AccessControl.FileSystemRights]::FullControl
$allow=[System.Security.AccessControl.AccessControlType]::Allow
if($env:NOTEGEN_VAULT_PERMISSION_DIRECTORY -eq '1'){
 $acl=[System.Security.AccessControl.DirectorySecurity]::new()
 $inherit=[System.Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit'
 $rule=[System.Security.AccessControl.FileSystemAccessRule]::new($sid,$rights,$inherit,[System.Security.AccessControl.PropagationFlags]::None,$allow)
}else{
 $acl=[System.Security.AccessControl.FileSecurity]::new()
 $rule=[System.Security.AccessControl.FileSystemAccessRule]::new($sid,$rights,$allow)
}
$acl.SetOwner($sid)
$acl.SetAccessRuleProtection($true,$false)
$acl.AddAccessRule($rule)
if($env:NOTEGEN_VAULT_PERMISSION_DIRECTORY -eq '1'){
 [System.IO.Directory]::SetAccessControl($p,$acl)
 $check=[System.IO.Directory]::GetAccessControl($p)
}else{
 [System.IO.File]::SetAccessControl($p,$acl)
 $check=[System.IO.File]::GetAccessControl($p)
}
$rules=@($check.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier]))
if(!$check.AreAccessRulesProtected -or $rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid.Value -or $rules[0].AccessControlType -ne $allow -or ($rules[0].FileSystemRights -band $rights) -ne $rights -or $check.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -ne $sid.Value){throw 'Private permissions did not verify'}
if($env:NOTEGEN_VAULT_PERMISSION_DIRECTORY -eq '1' -and $rules[0].InheritanceFlags -ne $inherit){throw 'Private inheritance did not verify'}
[Console]::Out.WriteLine('PRIVATE_OK')
}catch{
 [Console]::Error.WriteLine('Private ACL operation failed: '+$_.Exception.GetType().FullName+'; HRESULT '+$_.Exception.HResult)
 exit 1
}
"#;
    let system = std::env::var_os("SystemRoot")
        .ok_or_else(|| fail("Windows permission helper is unavailable"))?;
    let executable = PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    anyhow::ensure!(
        executable.is_file(),
        fail("Windows permission helper is unavailable")
    );
    let encoded = STANDARD.encode(
        SCRIPT
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut child = Command::new(executable)
        .creation_flags(0x08000000)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded,
        ])
        .env("NOTEGEN_VAULT_PERMISSION_TARGET", path)
        .env(
            "NOTEGEN_VAULT_PERMISSION_DIRECTORY",
            if directory { "1" } else { "0" },
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| fail("Could not start private-permission helper"))?;
    let start = Instant::now();
    loop {
        if c.check_cancelled().is_err() || start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(fail(
                "Private-permission verification was cancelled or timed out",
            ));
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(fail("Private-permission helper stopped unexpectedly"));
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|_| fail("Could not verify private permissions"))?;
    if !output.status.success()
        || output.stdout.len() > 1024
        || String::from_utf8_lossy(&output.stdout).trim() != "PRIVATE_OK"
    {
        // The script prints only exception type/HRESULT. Never pass arbitrary
        // PowerShell diagnostics (which may contain a private path) to UI/logs.
        let error = String::from_utf8_lossy(&output.stderr);
        let diagnostic = error
            .lines()
            .find(|line| {
                line.starts_with("Private ACL operation failed:")
                    && line.len() <= 256
                    && line
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b" .:;_-".contains(&byte))
            })
            .unwrap_or("Private ACL helper did not return a verified result");
        return Err(fail(&format!(
            "Private permissions could not be enforced; no secret data was written. {diagnostic}"
        )));
    }
    Ok(())
}
#[cfg(not(any(unix, windows)))]
fn private_permissions(_path: &Path, _directory: bool, _c: &Control) -> Result<()> {
    Err(fail("Private permissions are unsupported on this platform"))
}
fn protect_new(path: &Path, directory: bool, allow_inherited: bool, c: &Control) -> Result<bool> {
    no_links(path)?;
    match private_permissions(path, directory, c) {
        Ok(()) => Ok(true),
        Err(error) => {
            c.check_cancelled()?;
            if allow_inherited {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
fn private_directories(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}
pub fn generate_recovery_key(output: &Path, c: &Control) -> Result<RecoveryReceipt> {
    generate_key(output, false, c)
}
/// Explicit offline/export fallback. The receipt states whether private permissions held.
pub fn generate_recovery_key_offline_export(output: &Path, c: &Control) -> Result<RecoveryReceipt> {
    generate_key(output, true, c)
}
fn generate_key(output: &Path, allow_inherited: bool, c: &Control) -> Result<RecoveryReceipt> {
    c.check()?;
    let output = target(output)?;
    anyhow::ensure!(!output.exists(), fail("Recovery key output already exists"));
    let mut file = tempfile::NamedTempFile::new_in(output.parent().expect("resolved parent"))?;
    let private_permissions = protect_new(file.path(), false, allow_inherited, c)?;
    let identity = age::x25519::Identity::generate();
    let secret = identity.to_string();
    file.write_all(secret.expose_secret().as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.as_file().sync_all()?;
    c.check()?;
    file.persist_noclobber(&output)
        .map_err(|_| fail("Recovery key output collided or is unavailable"))?;
    let check = recovery_identity(&output)?;
    anyhow::ensure!(
        check.to_public() == identity.to_public(),
        fail("Saved recovery key did not verify")
    );
    Ok(RecoveryReceipt {
        path: output.clone(),
        recipient: identity.to_public().to_string(),
        sha256: hash_file(&output, &Control::default())?,
        private_permissions,
        note: format!(
            "This separate high-entropy key unlocks the encrypted workspace. Keep a verified copy apart from its encrypted backups. The key file itself is not encrypted. Private owner-only permissions verified: {private_permissions}. If false, you explicitly chose inherited permissions for an offline/export destination. Lost keys/passphrases cannot be recovered by NoteGen."
        ),
    })
}
fn wrap_key(
    identity: &age::x25519::Identity,
    passphrase: &SecretString,
    log_n: u8,
) -> Result<String> {
    secret_ok(passphrase)?;
    let mut recipient = age::scrypt::Recipient::new(passphrase.clone());
    recipient.set_work_factor(log_n);
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .map_err(|_| fail("Cannot prepare passphrase envelope"))?;
    let mut bytes = vec![];
    let mut writer = encryptor
        .wrap_output(&mut bytes)
        .map_err(|_| fail("Cannot encrypt workspace key"))?;
    let key = identity.to_string();
    writer.write_all(key.expose_secret().as_bytes())?;
    writer.finish()?;
    anyhow::ensure!(
        bytes.len() <= WRAPPER_LIMIT,
        fail("Workspace key envelope is oversized")
    );
    Ok(STANDARD.encode(bytes))
}
fn age_header<R: Read>(reader: &mut R, limit: usize, wrapper: bool) -> Result<Vec<u8>> {
    let mut header = vec![];
    let mut line = vec![];
    let mut recipients = 0usize;
    let mut scrypt = 0usize;
    loop {
        let mut byte = [0u8; 1];
        reader
            .read_exact(&mut byte)
            .map_err(|_| fail("Truncated encrypted header"))?;
        header.push(byte[0]);
        line.push(byte[0]);
        anyhow::ensure!(
            header.len() <= limit && line.len() <= 4096,
            fail("Encrypted header exceeds its bounded budget")
        );
        if byte[0] == b'\n' {
            if header.len() == line.len() {
                anyhow::ensure!(
                    line == b"age-encryption.org/v1\n",
                    fail("Unsupported age encryption header")
                );
            }
            if line.starts_with(b"-> ") {
                recipients += 1;
                anyhow::ensure!(
                    recipients <= 8,
                    fail("Too many encrypted recipient stanzas")
                );
                if line.starts_with(b"-> scrypt ") {
                    scrypt += 1;
                    let text = std::str::from_utf8(&line)
                        .map_err(|_| fail("Invalid passphrase header"))?;
                    let fields = text.split_whitespace().collect::<Vec<_>>();
                    let factor = fields
                        .get(3)
                        .and_then(|s| s.parse::<u8>().ok())
                        .ok_or_else(|| fail("Invalid passphrase work factor"))?;
                    anyhow::ensure!(
                        wrapper && (16..=18).contains(&factor),
                        fail("Passphrase work factor exceeds the supported 16–18 budget")
                    );
                }
            }
            if line.starts_with(b"--- ") {
                anyhow::ensure!(
                    recipients > 0 && (!wrapper || scrypt == 1),
                    fail("Invalid encrypted recipient envelope")
                );
                break;
            }
            line.clear();
        }
    }
    Ok(header)
}
fn wrapper_bytes(header: &Header) -> Result<Vec<u8>> {
    anyhow::ensure!(
        header.schema == 1
            && valid_digest(&header.vault_id)
            && header.wrapped_workspace_key.len() <= WRAPPER_LIMIT * 2,
        fail("Invalid workspace envelope")
    );
    let bytes = STANDARD
        .decode(&header.wrapped_workspace_key)
        .map_err(|_| fail("Invalid workspace envelope encoding"))?;
    anyhow::ensure!(
        bytes.len() <= WRAPPER_LIMIT,
        fail("Oversized workspace key envelope")
    );
    age_header(&mut bytes.as_slice(), WRAPPER_LIMIT, true)?;
    Ok(bytes)
}
fn unwrap_key(header: &Header, passphrase: &SecretString) -> Result<age::x25519::Identity> {
    secret_ok(passphrase)?;
    let bytes = wrapper_bytes(header)?;
    let mut identity = age::scrypt::Identity::new(passphrase.clone());
    identity.set_max_work_factor(18);
    let decryptor = age::Decryptor::new(bytes.as_slice())
        .map_err(|_| fail("Invalid workspace key envelope"))?;
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| fail("Wrong passphrase or damaged encrypted workspace"))?;
    let mut plain = Zeroizing::new(vec![]);
    reader
        .by_ref()
        .take(257)
        .read_to_end(&mut plain)
        .map_err(|_| fail("Wrong passphrase or damaged encrypted workspace"))?;
    anyhow::ensure!(plain.len() <= 256, fail("Invalid decrypted workspace key"));
    let text = std::str::from_utf8(&plain).map_err(|_| fail("Invalid decrypted workspace key"))?;
    text.parse()
        .map_err(|_| fail("Invalid decrypted workspace key"))
}
fn read_header(file: &mut File) -> Result<(Header, Vec<u8>)> {
    let mut magic = vec![0u8; MAGIC.len()];
    file.read_exact(&mut magic)?;
    anyhow::ensure!(magic == MAGIC, fail("Not a supported encrypted workspace"));
    let mut length = [0u8; 4];
    file.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    anyhow::ensure!(
        length > 0 && length <= HEADER_LIMIT,
        fail("Workspace header exceeds its bounded budget")
    );
    let mut bytes = vec![0u8; length];
    file.read_exact(&mut bytes)?;
    let header: Header =
        serde_json::from_slice(&bytes).map_err(|_| fail("Invalid workspace header"))?;
    wrapper_bytes(&header)?;
    Ok((header, bytes))
}
fn payload_reader(
    path: &Path,
    identity: &age::x25519::Identity,
    c: &Control,
) -> Result<(Header, Vec<u8>, Box<dyn Read>)> {
    c.check()?;
    no_links(path)?;
    let mut file = File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file()
            && file.metadata()?.len()
                <= HARD_TOTAL_BYTES + MANIFEST_LIMIT as u64 + 64 * 1024 * 1024,
        fail("Encrypted workspace exceeds its file budget")
    );
    let (header, raw) = read_header(&mut file)?;
    let prefix = age_header(&mut file, AGE_HEADER_LIMIT, false)?;
    let input = std::io::Cursor::new(prefix).chain(file);
    let decryptor = age::Decryptor::new(input).map_err(|_| fail("Invalid encrypted payload"))?;
    let reader = decryptor
        .decrypt(std::iter::once(identity as &dyn age::Identity))
        .map_err(|_| fail("Wrong key or damaged encrypted workspace"))?;
    Ok((header, raw, Box::new(reader)))
}
fn private_inventory(reader: &mut dyn Read, header: &Header, raw: &[u8]) -> Result<Inventory> {
    let mut magic = vec![0u8; PRIVATE_MAGIC.len()];
    reader
        .read_exact(&mut magic)
        .map_err(|_| fail("Wrong key or damaged encrypted payload"))?;
    anyhow::ensure!(
        magic == PRIVATE_MAGIC,
        fail("Unsupported private workspace format")
    );
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    anyhow::ensure!(
        length > 0 && length <= MANIFEST_LIMIT,
        fail("Private manifest exceeds its bounded budget")
    );
    let mut bytes = Zeroizing::new(vec![0u8; length]);
    reader.read_exact(&mut bytes)?;
    let inventory: Inventory =
        serde_json::from_slice(&bytes).map_err(|_| fail("Invalid private workspace manifest"))?;
    validate_inventory(&inventory, header, raw)?;
    Ok(inventory)
}
fn read_payload(
    path: &Path,
    identity: &age::x25519::Identity,
    extract: Option<&Path>,
    c: &Control,
) -> Result<(Header, Inventory)> {
    let (header, raw, mut reader) = payload_reader(path, identity, c)?;
    let inventory = private_inventory(&mut *reader, &header, &raw)?;
    if let Some(root) = extract {
        for directory in &inventory.directories {
            c.check()?;
            let path = root.join(directory);
            no_links(&path)?;
            private_directories(&path)?;
        }
    }
    let mut buffer = Zeroizing::new(vec![0u8; 64 * 1024]);
    for entry in &inventory.files {
        c.check()?;
        let mut output = if let Some(root) = extract {
            let path = root.join(&entry.path);
            no_links(&path)?;
            anyhow::ensure!(!path.exists(), fail("Decrypted work-file collision"));
            Some((
                tempfile::NamedTempFile::new_in(path.parent().expect("declared parent"))?,
                path,
            ))
        } else {
            None
        };
        let mut remaining = entry.bytes;
        let mut hash = Sha256::new();
        while remaining > 0 {
            c.check()?;
            let count = remaining.min(buffer.len() as u64) as usize;
            reader
                .read_exact(&mut buffer[..count])
                .map_err(|_| fail("Truncated or damaged encrypted file data"))?;
            hash.update(&buffer[..count]);
            if let Some((file, _)) = &mut output {
                file.write_all(&buffer[..count])?;
            }
            remaining -= count as u64;
        }
        anyhow::ensure!(
            hex::encode(hash.finalize()) == entry.sha256,
            fail("Encrypted file content does not match its private manifest")
        );
        if let Some((file, path)) = output {
            file.as_file().sync_all()?;
            file.persist_noclobber(path)
                .map_err(|_| fail("Decrypted work-file collision"))?;
        }
    }
    let mut trailing = [0u8; 1];
    anyhow::ensure!(
        reader
            .read(&mut trailing)
            .map_err(|_| fail("Truncated or damaged encrypted payload"))?
            == 0,
        fail("Unexpected trailing private workspace data")
    );
    Ok((header, inventory))
}
fn validate_inventory(inventory: &Inventory, header: &Header, raw: &[u8]) -> Result<()> {
    anyhow::ensure!(
        inventory.schema == 1
            && inventory.vault_id == header.vault_id
            && inventory.revision > 0
            && inventory.header_sha256 == digest(raw)
            && inventory.files.len() + inventory.directories.len() <= HARD_FILES
            && inventory.note.len() <= 8192
            && data_path(&inventory.project_relative)
            && inventory.project_relative.starts_with("work/"),
        fail("Private manifest identity, version or bounds are invalid")
    );
    inventory
        .recovery_recipient
        .parse::<age::x25519::Recipient>()
        .map_err(|_| fail("Invalid recovery recipient"))?;
    anyhow::ensure!(
        inventory
            .original_project_root
            .as_ref()
            .is_none_or(|path| original_location_valid(path)),
        fail("Invalid original-project location")
    );
    let mut paths = BTreeMap::<String, String>::new();
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut total = 0u64;
    for directory in &inventory.directories {
        anyhow::ensure!(
            (directory == "work" || directory == "source" || data_path(directory))
                && directories.insert(directory),
            fail("Invalid or duplicate encrypted directory")
        );
        anyhow::ensure!(
            paths
                .insert(path_key(directory), directory.clone())
                .is_none(),
            fail("Encrypted paths collide across platforms")
        );
    }
    for directory in &inventory.directories {
        let mut parent = Path::new(directory).parent();
        while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
            let text = relative(path)?;
            anyhow::ensure!(
                directories.contains(&text),
                fail("Encrypted directory has an undeclared parent")
            );
            parent = path.parent();
        }
    }
    for entry in &inventory.files {
        anyhow::ensure!(
            data_path(&entry.path)
                && valid_digest(&entry.sha256)
                && entry.bytes <= HARD_FILE_BYTES
                && files.insert(&entry.path),
            fail("Invalid encrypted file entry")
        );
        anyhow::ensure!(
            paths
                .insert(path_key(&entry.path), entry.path.clone())
                .is_none(),
            fail("Encrypted paths collide across platforms")
        );
        total = total
            .checked_add(entry.bytes)
            .ok_or_else(|| fail("Encrypted file sizes overflow"))?;
        anyhow::ensure!(
            total <= HARD_TOTAL_BYTES,
            fail("Encrypted workspace exceeds total file budget")
        );
        let mut parent = Path::new(&entry.path).parent();
        while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
            let text = relative(path)?;
            anyhow::ensure!(
                directories.contains(&text),
                fail("Encrypted file has an undeclared parent directory")
            );
            parent = path.parent();
        }
    }
    anyhow::ensure!(
        files.contains(&format!("{}/project.json", inventory.project_relative)),
        fail("Encrypted workspace has no active project manifest")
    );
    Ok(())
}
fn add_file(
    snapshot: &mut Snapshot,
    source: &Path,
    path: String,
    options: &Options,
    c: &Control,
) -> Result<()> {
    no_links(source)?;
    let before = fs::symlink_metadata(source)?;
    anyhow::ensure!(
        before.is_file() && before.len() <= options.max_file_bytes,
        fail("Workspace entry is not a bounded regular file")
    );
    let held = if source
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".lock"))
    {
        let lock = OpenOptions::new().read(true).write(true).open(source)?;
        lock.try_lock()
            .map_err(|_| fail("A project worker is still using the source snapshot"))?;
        Some(lock)
    } else {
        None
    };
    let mut file = SourceFile {
        source: source.into(),
        entry: FileEntry {
            path,
            bytes: before.len(),
            sha256: String::new(),
        },
        modified: before.modified()?,
        held,
    };
    file.entry.sha256 = source_hash(&file, c)?;
    let after = fs::symlink_metadata(source)?;
    no_links(source)?;
    anyhow::ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        fail("Workspace source changed while inspecting it")
    );
    snapshot.files.push(file);
    anyhow::ensure!(
        snapshot.files.len() + snapshot.directories.len() <= options.max_files,
        fail("Workspace exceeds its file-count budget")
    );
    snapshot.total_bytes = snapshot
        .total_bytes
        .checked_add(after.len())
        .ok_or_else(|| fail("Workspace sizes overflow"))?;
    anyhow::ensure!(
        snapshot.total_bytes <= options.max_total_bytes,
        fail("Workspace exceeds its total plaintext byte budget")
    );
    Ok(())
}
fn add_tree(
    snapshot: &mut Snapshot,
    source: &Path,
    prefix: &str,
    options: &Options,
    c: &Control,
) -> Result<()> {
    no_links(source)?;
    anyhow::ensure!(source.is_dir(), fail("Choose a project folder"));
    let mut pending = vec![(source.to_owned(), prefix.to_string())];
    while let Some((folder, relative)) = pending.pop() {
        c.check()?;
        no_links(&folder)?;
        snapshot
            .observed_directories
            .push((folder.clone(), fs::metadata(&folder)?.modified()?));
        snapshot.directories.push(relative.clone());
        anyhow::ensure!(
            snapshot.files.len() + snapshot.directories.len() <= options.max_files,
            fail("Workspace exceeds its file-count budget")
        );
        let mut entries = fs::read_dir(folder)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            c.check()?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| fail("Workspace names must be valid Unicode"))?;
            anyhow::ensure!(
                !name.eq_ignore_ascii_case("project-transaction.json"),
                fail(
                    "A project migration is incomplete; recover its pending transaction before creating an encrypted snapshot"
                )
            );
            let relative = format!("{relative}/{name}");
            anyhow::ensure!(data_path(&relative), fail("Workspace path is not portable"));
            no_links(&entry.path())?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                pending.push((entry.path(), relative));
            } else {
                add_file(snapshot, &entry.path(), relative, options, c)?;
            }
        }
    }
    snapshot
        .files
        .sort_by(|a, b| a.entry.path.cmp(&b.entry.path));
    snapshot.directories.sort();
    snapshot.directories.dedup();
    Ok(())
}
fn inventory(
    snapshot: &Snapshot,
    header: &Header,
    revision: u64,
    recovery: &str,
    project: &str,
    original: Option<PathBuf>,
) -> Result<Inventory> {
    let raw = serde_json::to_vec(header)?;
    let result = Inventory {
        schema: 1,
        vault_id: header.vault_id.clone(),
        revision,
        header_sha256: digest(&raw),
        recovery_recipient: recovery.into(),
        project_relative: project.into(),
        original_project_root: original,
        directories: snapshot.directories.clone(),
        files: snapshot.files.iter().map(|f| f.entry.clone()).collect(),
        note: LIMITS_NOTE.into(),
    };
    validate_inventory(&result, header, &raw)?;
    Ok(result)
}
fn write_prefix(file: &mut File, header: &Header) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(header)?;
    anyhow::ensure!(
        bytes.len() <= HEADER_LIMIT,
        fail("Workspace header is oversized")
    );
    file.write_all(MAGIC)?;
    file.write_all(&(bytes.len() as u32).to_be_bytes())?;
    file.write_all(&bytes)?;
    Ok(bytes)
}
fn encrypt_snapshot(
    snapshot: &Snapshot,
    header: &Header,
    identity: &age::x25519::Identity,
    manifest: &Inventory,
    output: &Path,
    c: &Control,
) -> Result<tempfile::NamedTempFile> {
    let parent = output
        .parent()
        .ok_or_else(|| fail("Missing workspace output folder"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write_prefix(temporary.as_file_mut(), header)?;
    let recipient = identity.to_public();
    let recovery = manifest
        .recovery_recipient
        .parse::<age::x25519::Recipient>()
        .map_err(|_| fail("Invalid recovery recipient"))?;
    let encryptor = age::Encryptor::with_recipients(
        [
            &recipient as &dyn age::Recipient,
            &recovery as &dyn age::Recipient,
        ]
        .into_iter(),
    )
    .map_err(|_| fail("Cannot prepare encrypted snapshot"))?;
    let mut writer = encryptor
        .wrap_output(temporary.as_file_mut())
        .map_err(|_| fail("Cannot encrypt workspace snapshot"))?;
    let private = Zeroizing::new(serde_json::to_vec(manifest)?);
    anyhow::ensure!(
        private.len() <= MANIFEST_LIMIT,
        fail("Private manifest exceeds its bounded budget")
    );
    writer.write_all(PRIVATE_MAGIC)?;
    writer.write_all(&(private.len() as u32).to_be_bytes())?;
    writer.write_all(&private)?;
    let mut buffer = Zeroizing::new(vec![0u8; 64 * 1024]);
    for source in &snapshot.files {
        c.check()?;
        no_links(&source.source)?;
        let mut file = source_reader(source)?;
        anyhow::ensure!(
            file.file().metadata()?.len() == source.entry.bytes
                && file.file().metadata()?.modified()? == source.modified,
            fail("Workspace changed after its snapshot preview")
        );
        let mut hash = Sha256::new();
        let mut remaining = source.entry.bytes;
        while remaining > 0 {
            c.check()?;
            let count = remaining.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..count]).map_err(|error| {
                fail(&format!(
                    "Snapshot source encryption read failed (OS error {:?})",
                    error.raw_os_error()
                ))
            })?;
            writer.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        let mut extra = [0u8; 1];
        anyhow::ensure!(
            file.read(&mut extra)? == 0
                && file.file().metadata()?.modified()? == source.modified
                && hex::encode(hash.finalize()) == source.entry.sha256,
            fail("Workspace changed while encrypting it")
        );
        no_links(&source.source)?;
    }
    writer.finish()?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    c.check()?;
    let (_, checked) = read_payload(temporary.path(), identity, None, c)?;
    anyhow::ensure!(
        serde_json::to_vec(&checked)? == *private,
        fail("New encrypted snapshot did not verify")
    );
    Ok(temporary)
}
fn unchanged(snapshot: &Snapshot, c: &Control) -> Result<()> {
    for (path, modified) in &snapshot.observed_directories {
        no_links(path)?;
        let metadata = fs::symlink_metadata(path)?;
        anyhow::ensure!(
            metadata.is_dir() && metadata.modified()? == *modified,
            fail("Workspace directory changed before committing its snapshot")
        );
    }
    for file in &snapshot.files {
        c.check()?;
        no_links(&file.source)?;
        let metadata = fs::symlink_metadata(&file.source)?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.len() == file.entry.bytes
                && metadata.modified()? == file.modified
                && source_hash(file, c)? == file.entry.sha256,
            fail("Workspace changed before committing its encrypted snapshot")
        );
    }
    Ok(())
}
fn receipt(
    path: &Path,
    inventory: &Inventory,
    previous: Option<PathBuf>,
    c: &Control,
) -> Result<Receipt> {
    Ok(Receipt {
        path: path.into(),
        vault_id: inventory.vault_id.clone(),
        revision: inventory.revision,
        bytes: fs::metadata(path)?.len(),
        sha256: hash_file(path, c)?,
        previous,
    })
}
fn access(path: &Path, credentials: &Credentials<'_>, c: &Control) -> Result<Access> {
    no_links(path)?;
    let before = hash_file(path, c)?;
    let (header, _) = read_header(&mut File::open(path)?)?;
    let (identity, recovery) = match credentials {
        Credentials::Passphrase(passphrase) => (unwrap_key(&header, passphrase)?, false),
        Credentials::RecoveryKey(path) => (recovery_identity(path)?, true),
    };
    let (header, inventory) = read_payload(path, &identity, None, c)?;
    anyhow::ensure!(
        !recovery || identity.to_public().to_string() == inventory.recovery_recipient,
        fail("Recovery key does not match this workspace")
    );
    let receipt = receipt(path, &inventory, None, c)?;
    anyhow::ensure!(
        before == receipt.sha256,
        fail("Encrypted workspace changed while verifying it")
    );
    Ok(Access {
        header,
        identity,
        inventory,
        receipt,
        recovery,
    })
}
pub fn create(
    project: &Path,
    output: &Path,
    passphrase: &SecretString,
    recovery_key: &Path,
    options: &Options,
    c: &Control,
) -> Result<Receipt> {
    options.validate()?;
    secret_ok(passphrase)?;
    c.check()?;
    no_links(project)?;
    let project = project.canonicalize()?;
    anyhow::ensure!(
        project.is_dir() && project.join("project.json").is_file(),
        fail("Choose an existing NoteGen project folder")
    );
    let preview = preview_target(output)?;
    anyhow::ensure!(
        !preview.starts_with(&project),
        fail("Store the encrypted container outside its original project")
    );
    no_links(recovery_key)?;
    let recovery_path = recovery_key.canonicalize()?;
    anyhow::ensure!(
        !recovery_path.starts_with(&project),
        fail("Save the recovery key separately from the project")
    );
    let recovery = recovery_identity(&recovery_path)?;
    let output = target(output)?;
    let _lease = lease(&output)?;
    anyhow::ensure!(
        !output.exists(),
        fail("Encrypted workspace output already exists")
    );
    let identity = age::x25519::Identity::generate();
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| fail("Secure randomness is unavailable"))?;
    let header = Header {
        schema: 1,
        vault_id: hex::encode(random),
        wrapped_workspace_key: wrap_key(&identity, passphrase, options.scrypt_log_n)?,
    };
    let mut snapshot = Snapshot {
        files: vec![],
        directories: vec!["work".into()],
        observed_directories: vec![],
        total_bytes: 0,
    };
    add_tree(&mut snapshot, &project, "work/project", options, c)?;
    let key_hash = hash_file(&recovery_path, c)?;
    anyhow::ensure!(
        snapshot.files.iter().all(|f| f.entry.sha256 != key_hash),
        fail("The selected project contains a copy of its recovery key")
    );
    let manifest = inventory(
        &snapshot,
        &header,
        1,
        &recovery.to_public().to_string(),
        "work/project",
        Some(project.clone()),
    )?;
    let temporary = encrypt_snapshot(&snapshot, &header, &identity, &manifest, &output, c)?;
    unchanged(&snapshot, c)?;
    c.check()?;
    temporary
        .persist_noclobber(&output)
        .map_err(|_| fail("Encrypted workspace output collided or is unavailable"))?;
    receipt(&output, &manifest, None, &Control::default())
}
pub fn verify(path: &Path, credentials: &Credentials<'_>, c: &Control) -> Result<Inventory> {
    no_links(path)?;
    let path = path.canonicalize()?;
    let _lease = lease(&path)?;
    Ok(access(&path, credentials, c)?.inventory)
}
fn encrypted_copy(
    source: &Path,
    output: &Path,
    identity: &age::x25519::Identity,
    expected: &str,
    c: &Control,
) -> Result<()> {
    anyhow::ensure!(
        !output.exists(),
        fail("Encrypted backup output already exists")
    );
    let mut temp =
        tempfile::NamedTempFile::new_in(output.parent().expect("resolved output parent"))?;
    let mut input = File::open(source)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        c.check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        anyhow::ensure!(
            total <= HARD_TOTAL_BYTES + MANIFEST_LIMIT as u64 + 64 * 1024 * 1024,
            fail("Encrypted backup exceeds its bounded budget")
        );
        temp.write_all(&buffer[..count])?;
    }
    temp.flush()?;
    temp.as_file().sync_all()?;
    anyhow::ensure!(
        hash_file(temp.path(), c)? == expected && hash_file(source, c)? == expected,
        fail("Encrypted backup copy changed or did not verify")
    );
    read_payload(temp.path(), identity, None, c)?;
    c.check()?;
    temp.persist_noclobber(output)
        .map_err(|_| fail("Encrypted backup output collided or is unavailable"))?;
    Ok(())
}
pub fn backup(
    path: &Path,
    output: &Path,
    credentials: &Credentials<'_>,
    c: &Control,
) -> Result<Receipt> {
    no_links(path)?;
    let path = path.canonicalize()?;
    let _lease = lease(&path)?;
    let opened = access(&path, credentials, c)?;
    anyhow::ensure!(
        !opened
            .inventory
            .original_project_root
            .as_ref()
            .is_some_and(|root| preview_target(output).is_ok_and(|path| path.starts_with(root))),
        fail("Store encrypted backups outside the original project")
    );
    let output = target(output)?;
    anyhow::ensure!(
        output != path,
        fail("Choose a separate encrypted backup filename")
    );
    encrypted_copy(&path, &output, &opened.identity, &opened.receipt.sha256, c)?;
    receipt(&output, &opened.inventory, None, &Control::default())
}
/// Restore into a new encrypted container. Existing containers and originals are protected.
pub fn restore(
    backup: &Path,
    output: &Path,
    credentials: &Credentials<'_>,
    c: &Control,
) -> Result<Receipt> {
    self::backup(backup, output, credentials, c)
}
fn preserve_previous(
    path: &Path,
    identity: &age::x25519::Identity,
    expected: &str,
    c: &Control,
) -> Result<PathBuf> {
    anyhow::ensure!(
        hash_file(path, c)? == expected,
        fail("Encrypted workspace changed; reopen it before saving")
    );
    let previous = path
        .parent()
        .expect("resolved vault parent")
        .join(format!(".vault-backup-{expected}.ngvault"));
    no_links(&previous)?;
    if previous.exists() {
        anyhow::ensure!(
            hash_file(&previous, c)? == expected,
            fail("Previous encrypted backup has changed")
        );
        read_payload(&previous, identity, None, c)?;
    } else {
        encrypted_copy(path, &previous, identity, expected, c)?;
    }
    Ok(previous)
}
fn commit_candidate(
    temporary: tempfile::NamedTempFile,
    path: &Path,
    old_key: &age::x25519::Identity,
    expected: &str,
    inventory: &Inventory,
    c: &Control,
) -> Result<Receipt> {
    let previous = preserve_previous(path, old_key, expected, c)?;
    c.check()?;
    anyhow::ensure!(
        hash_file(path, c)? == expected,
        fail("Encrypted workspace changed before atomic commit")
    );
    temporary.persist(path).map_err(|_| {
        fail("Could not activate the encrypted snapshot; previous container is preserved")
    })?;
    // The commit succeeded. Finish its receipt even if cancellation arrived now.
    receipt(path, inventory, Some(previous), &Control::default())
}

/// Re-encrypt the complete authenticated snapshot with a fresh workspace key.
/// No plaintext working directory is created. Old backups keep their old keys.
pub fn rotate(
    path: &Path,
    credentials: &Credentials<'_>,
    new_passphrase: &SecretString,
    new_recovery_key: Option<&Path>,
    options: &Options,
    c: &Control,
) -> Result<Receipt> {
    options.validate()?;
    secret_ok(new_passphrase)?;
    no_links(path)?;
    let path = path.canonicalize()?;
    let _lease = lease(&path)?;
    let opened = access(&path, credentials, c)?;
    let identity = age::x25519::Identity::generate();
    let header = Header {
        schema: 1,
        vault_id: opened.header.vault_id.clone(),
        wrapped_workspace_key: wrap_key(&identity, new_passphrase, options.scrypt_log_n)?,
    };
    let mut manifest = opened.inventory.clone();
    manifest.revision = manifest
        .revision
        .checked_add(1)
        .ok_or_else(|| fail("Encrypted revision counter is exhausted"))?;
    manifest.header_sha256 = digest(&serde_json::to_vec(&header)?);
    if let Some(key) = new_recovery_key {
        no_links(key)?;
        let key = key.canonicalize()?;
        anyhow::ensure!(
            !manifest
                .original_project_root
                .as_ref()
                .is_some_and(|root| key.starts_with(root)),
            fail("Save the replacement recovery key separately from the original project")
        );
        manifest.recovery_recipient = recovery_identity(&key)?.to_public().to_string();
    }
    validate_inventory(&manifest, &header, &serde_json::to_vec(&header)?)?;
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().expect("resolved parent"))?;
    write_prefix(temporary.as_file_mut(), &header)?;
    let recipient = identity.to_public();
    let recovery = manifest
        .recovery_recipient
        .parse::<age::x25519::Recipient>()
        .map_err(|_| fail("Invalid recovery recipient"))?;
    let encryptor = age::Encryptor::with_recipients(
        [
            &recipient as &dyn age::Recipient,
            &recovery as &dyn age::Recipient,
        ]
        .into_iter(),
    )?;
    let mut writer = encryptor.wrap_output(temporary.as_file_mut())?;
    let private = Zeroizing::new(serde_json::to_vec(&manifest)?);
    anyhow::ensure!(
        private.len() <= MANIFEST_LIMIT,
        fail("Private manifest exceeds its bounded budget")
    );
    writer.write_all(PRIVATE_MAGIC)?;
    writer.write_all(&(private.len() as u32).to_be_bytes())?;
    writer.write_all(&private)?;
    let (old_header, raw, mut reader) = payload_reader(&path, &opened.identity, c)?;
    let old_manifest = private_inventory(&mut *reader, &old_header, &raw)?;
    anyhow::ensure!(
        serde_json::to_vec(&old_manifest)? == serde_json::to_vec(&opened.inventory)?,
        fail("Workspace changed while rotating keys")
    );
    let mut buffer = Zeroizing::new(vec![0u8; 64 * 1024]);
    for entry in &manifest.files {
        let mut left = entry.bytes;
        let mut hash = Sha256::new();
        while left > 0 {
            c.check()?;
            let count = left.min(buffer.len() as u64) as usize;
            reader.read_exact(&mut buffer[..count])?;
            hash.update(&buffer[..count]);
            writer.write_all(&buffer[..count])?;
            left -= count as u64;
        }
        anyhow::ensure!(
            hex::encode(hash.finalize()) == entry.sha256,
            fail("File changed or failed authentication during rotation")
        );
    }
    let mut extra = [0u8; 1];
    anyhow::ensure!(
        reader.read(&mut extra)? == 0,
        fail("Unexpected data during key rotation")
    );
    drop(reader);
    writer.finish()?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    let (_, checked) = read_payload(temporary.path(), &identity, None, c)?;
    anyhow::ensure!(
        serde_json::to_vec(&checked)? == *private,
        fail("Rotated container did not verify")
    );
    commit_candidate(
        temporary,
        &path,
        &opened.identity,
        &opened.receipt.sha256,
        &manifest,
        c,
    )
}

/// Authenticate fully before creating a plaintext work copy. Recovery credentials
/// require a replacement passphrase; it becomes active only after a durable commit.
pub fn unlock(
    path: &Path,
    credentials: &Credentials<'_>,
    work_parent: &Path,
    replacement_passphrase: Option<&SecretString>,
    options: &Options,
    c: &Control,
) -> Result<WorkCopy> {
    options.validate()?;
    c.check()?;
    no_links(path)?;
    let vault = path.canonicalize()?;
    let lease = lease(&vault)?;
    let opened = access(&vault, credentials, c)?;
    anyhow::ensure!(
        !opened.recovery || replacement_passphrase.is_some(),
        fail("Choose a new passphrase when recovering a work copy")
    );
    anyhow::ensure!(
        opened.inventory.files.len() + opened.inventory.directories.len() <= options.max_files
            && opened
                .inventory
                .files
                .iter()
                .all(|entry| entry.bytes <= options.max_file_bytes)
            && opened
                .inventory
                .files
                .iter()
                .map(|entry| entry.bytes)
                .sum::<u64>()
                <= options.max_total_bytes,
        fail("Choose larger work-copy limits for this verified snapshot")
    );
    no_links(work_parent)?;
    let work_parent = work_parent.canonicalize()?;
    anyhow::ensure!(
        !opened
            .inventory
            .original_project_root
            .as_ref()
            .is_some_and(|original| work_parent.starts_with(original)),
        fail("Choose a managed work parent outside the original project")
    );
    let (header, workspace_key) = if let Some(passphrase) = replacement_passphrase {
        let identity = age::x25519::Identity::generate();
        let header = Header {
            schema: 1,
            vault_id: opened.header.vault_id.clone(),
            wrapped_workspace_key: wrap_key(&identity, passphrase, options.scrypt_log_n)?,
        };
        (header, identity)
    } else {
        (opened.header.clone(), opened.identity.clone())
    };
    // First pass above checked every AEAD chunk, file digest and strict EOF.
    // Only now create the marked, owner-private plaintext staging workspace.
    let session = Session::create(&work_parent, c)?;
    let private_permissions =
        protect_new(&session.root, true, options.allow_inherited_permissions, c)?;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| fail("Secure randomness is unavailable"))?;
    let marker = WorkMarker {
        schema: 1,
        kind: "notegen_encrypted_workspace_copy".into(),
        root: session.root.clone(),
        vault: vault.clone(),
        vault_id: opened.header.vault_id.clone(),
        base_sha256: opened.receipt.sha256.clone(),
        id: hex::encode(random),
    };
    let mut file = private_file(&session.root.join(WORK_MARKER))?;
    serde_json::to_writer(&mut file, &marker)?;
    file.sync_all()?;
    drop(file);
    let mut owner_hashes = BTreeMap::new();
    for name in [
        "notegen-session.json",
        "retention.json",
        "session.lock",
        WORK_MARKER,
    ] {
        owner_hashes.insert(name.into(), hash_file(&session.root.join(name), c)?);
    }
    let result = (|| -> Result<()> {
        let (_, manifest) = read_payload(&vault, &opened.identity, Some(&session.root), c)?;
        anyhow::ensure!(
            serde_json::to_vec(&manifest)? == serde_json::to_vec(&opened.inventory)?
                && hash_file(&vault, c)? == opened.receipt.sha256,
            fail("Encrypted workspace changed during opening")
        );
        Ok(())
    })();
    if let Err(error) = result {
        let mut retained = vec![];
        for entry in &opened.inventory.files {
            let path = session.root.join(&entry.path);
            if path.exists()
                && no_links(&path).is_ok()
                && hash_file(&path, &Control::default()).is_ok_and(|hash| hash == entry.sha256)
            {
                if fs::remove_file(&path).is_err() {
                    retained.push(path);
                }
            } else if path.exists() {
                retained.push(path);
            }
        }
        if retained.is_empty() {
            return Err(fail(&format!(
                "Could not publish the work copy: {error}. Empty app ownership metadata remains at {}.",
                session.root.display()
            )));
        }
        return Err(fail(&format!(
            "Could not publish the work copy: {error}. A private staging copy has retained files at {}; inspect it before deleting anything.",
            session.root.display()
        )));
    }
    Ok(WorkCopy {
        session,
        vault,
        _lease: Some(lease),
        header,
        workspace_key: Some(workspace_key),
        previous_key: Some(opened.identity),
        inventory: opened.inventory,
        base_sha256: opened.receipt.sha256,
        options: options.clone(),
        owner_hashes,
        workers: Arc::new(Workers {
            state: Mutex::new(WorkerState::default()),
            changed: Condvar::new(),
        }),
        closed: false,
        private_permissions,
    })
}
impl WorkCopy {
    pub fn project_path(&self) -> PathBuf {
        self.session.root.join(&self.inventory.project_relative)
    }
    pub fn work_root(&self) -> &Path {
        &self.session.root
    }
    pub fn private_permissions(&self) -> bool {
        self.private_permissions
    }
    pub fn note(&self) -> &'static str {
        LIMITS_NOTE
    }
    fn worker(&self) -> Result<WorkerGuard> {
        anyhow::ensure!(!self.closed, fail("This encrypted work copy is closed"));
        let mut state = self.workers.state.lock().unwrap_or_else(|e| e.into_inner());
        anyhow::ensure!(
            !state.closing && state.active == 0,
            fail("This work copy is closing or busy; wait for its result")
        );
        let file = {
            let path = self.session.root.join("session.lock");
            no_links(&path)?;
            let file = OpenOptions::new().read(true).write(true).open(path)?;
            file.try_lock()
                .map_err(|_| fail("This work copy is busy in another operation"))?;
            Some(file)
        };
        state.active += 1;
        Ok(WorkerGuard {
            workers: self.workers.clone(),
            _file: file,
        })
    }
    /// Hold for every direct project operation, including saves and native readers.
    pub fn begin_worker(&self) -> Result<WorkerGuard> {
        self.worker()
    }
    /// Each conversion gets a fresh child Session under the private work tree.
    /// Its own lock is distinct from the parent worker guard, so old absolute-
    /// bound retention records remain historical data and are never reactivated.
    pub fn run_session(
        &mut self,
        input: &Path,
        settings: &Settings,
        c: &Control,
        log: &Activity,
    ) -> Result<sessions::RunReport> {
        let _worker = self.worker()?;
        let parent = self.session.root.join("work");
        private_directories(&parent)?;
        let child = Session::create(&parent, c)?;
        let report = sessions::run(&child, input, settings, c, log)?;
        if report.success
            && let Some(project) = &report.project
        {
            no_links(project)?;
            let project = project.canonicalize()?;
            let path = project
                .strip_prefix(&self.session.root)
                .map_err(|_| fail("Session project escaped the managed work root"))?;
            let relative = relative(path)?;
            anyhow::ensure!(
                relative.starts_with("work/") && project.join("project.json").is_file(),
                fail("Session returned an invalid managed project")
            );
            self.inventory.project_relative = relative;
        }
        Ok(report)
    }
    fn owner_valid(&self, c: &Control) -> Result<()> {
        no_links(&self.session.root)?;
        anyhow::ensure!(
            self.session.root.canonicalize()? == self.session.root,
            fail("Managed work-copy location changed")
        );
        Session::read(&self.session.root)?;
        for name in ["notegen-session.json", WORK_MARKER] {
            anyhow::ensure!(
                hash_file(&self.session.root.join(name), c)? == self.owner_hashes[name],
                fail("Managed work-copy ownership marker changed")
            );
        }
        Ok(())
    }
    fn wait_workers(&self, timeout: Duration, c: &Control) -> Result<File> {
        anyhow::ensure!(
            timeout <= Duration::from_secs(300),
            fail("Worker-wait budget exceeds five minutes")
        );
        let start = Instant::now();
        let mut state = self.workers.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closing = true;
        while state.active > 0 {
            c.check_cancelled()?;
            anyhow::ensure!(
                start.elapsed() < timeout,
                fail("Work copy is still busy; plaintext remains available")
            );
            state = self
                .workers
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(state);
        let path = self.session.root.join("session.lock");
        no_links(&path)?;
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        loop {
            c.check_cancelled()?;
            if file.try_lock().is_ok() {
                return Ok(file);
            }
            anyhow::ensure!(
                start.elapsed() < timeout,
                fail("A session worker is still busy; plaintext remains available")
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn snapshot(&self, c: &Control) -> Result<Snapshot> {
        let mut snapshot = Snapshot {
            files: vec![],
            directories: vec![],
            observed_directories: vec![],
            total_bytes: 0,
        };
        for name in ["source", "work"] {
            let path = self.session.root.join(name);
            if path.exists() {
                add_tree(&mut snapshot, &path, name, &self.options, c)?;
            }
        }
        let state = self.session.root.join("retention.json");
        if state.exists() {
            let sha = hash_file(&state, c)?;
            if sha != self.owner_hashes["retention.json"] {
                let path = format!("work/.vault-session-history/{sha}.json");
                // Preserve old canonical-bound state as encrypted historical data,
                // never activate it as the new session's live retention manifest.
                if !snapshot.files.iter().any(|file| file.entry.path == path) {
                    add_file(&mut snapshot, &state, path, &self.options, c)?;
                    snapshot
                        .directories
                        .extend(["work".into(), "work/.vault-session-history".into()]);
                }
            }
        }
        snapshot
            .files
            .sort_by(|a, b| a.entry.path.cmp(&b.entry.path));
        snapshot.directories.sort();
        snapshot.directories.dedup();
        Ok(snapshot)
    }
    pub fn commit_and_lock(&mut self, timeout: Duration, c: &Control) -> Result<LockReport> {
        anyhow::ensure!(
            !self.closed,
            fail("This encrypted work copy is already closed")
        );
        let result = (|| -> Result<LockReport> {
            let worker_lock = self.wait_workers(timeout, c)?;
            self.owner_valid(c)?;
            let mut snapshot = self.snapshot(c)?;
            let revision = self
                .inventory
                .revision
                .checked_add(1)
                .ok_or_else(|| fail("Encrypted revision counter is exhausted"))?;
            let manifest = inventory(
                &snapshot,
                &self.header,
                revision,
                &self.inventory.recovery_recipient,
                &self.inventory.project_relative,
                self.inventory.original_project_root.clone(),
            )?;
            let key = self
                .workspace_key
                .as_ref()
                .ok_or_else(|| fail("Work-copy key is unavailable"))?;
            let temporary =
                encrypt_snapshot(&snapshot, &self.header, key, &manifest, &self.vault, c)?;
            unchanged(&snapshot, c)?;
            self.owner_valid(c)?;
            let receipt = commit_candidate(
                temporary,
                &self.vault,
                self.previous_key
                    .as_ref()
                    .ok_or_else(|| fail("Previous key is unavailable"))?,
                &self.base_sha256,
                &manifest,
                c,
            )?;
            // Native/project worker handles are all joined; release source locks
            // before Windows file/directory cleanup, keeping the session lock held.
            for file in &mut snapshot.files {
                file.held.take();
            }
            let report = cleanup_copy(
                &self.session,
                &self.owner_hashes,
                &snapshot,
                worker_lock,
                receipt,
                c,
            );
            self.closed = true;
            self.workspace_key.take();
            self.previous_key.take();
            self._lease.take();
            Ok(report)
        })();
        if result.is_err() {
            let mut state = self.workers.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closing = false;
        }
        result
    }
}
fn remaining(root: &Path) -> Vec<Retained> {
    let mut pending = vec![root.to_owned()];
    let mut output = vec![];
    while let Some(folder) = pending.pop() {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(_) => {
                output.push(Retained {
                    path: folder,
                    reason: "Could not inspect retained work data".into(),
                });
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    output.push(Retained {
                        path: folder.clone(),
                        reason: "A retained entry could not be inspected".into(),
                    });
                    continue;
                }
            };
            let path = entry.path();
            output.push(Retained {
                path: path.clone(),
                reason: "Retained unknown, changed, busy or unavailable plaintext/work metadata"
                    .into(),
            });
            if output.len() >= HARD_FILES {
                output.push(Retained {
                    path: root.into(),
                    reason:
                        "Further retained entries exceed report budget; inspect this marked folder"
                            .into(),
                });
                return output;
            }
            if no_links(&path).is_ok() && entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(path);
            }
        }
    }
    output
}
/// Inspect a marked abandoned/partially cleaned copy without adopting or deleting it.
/// Re-open the last verified container normally; preserve any newer plaintext edits
/// separately before deciding manually what to remove from this reported root.
pub fn inspect_retained(root: &Path, c: &Control) -> Result<RetainedCopy> {
    c.check()?;
    no_links(root)?;
    let root = root.canonicalize()?;
    let path = root.join(WORK_MARKER);
    no_links(&path)?;
    let file = File::open(&path)?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 128 * 1024,
        fail("Invalid retained-work ownership marker")
    );
    let mut bytes = vec![];
    file.take(128 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 128 * 1024,
        fail("Oversized retained-work marker")
    );
    let marker: WorkMarker = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        marker.schema == 1
            && marker.kind == "notegen_encrypted_workspace_copy"
            && marker.root == root
            && marker.vault.is_absolute()
            && valid_digest(&marker.id)
            && valid_digest(&marker.vault_id)
            && valid_digest(&marker.base_sha256),
        fail("Retained-work marker does not match this location")
    );
    Ok(RetainedCopy {
        root: root.clone(),
        vault: marker.vault,
        vault_id: marker.vault_id,
        base_sha256: marker.base_sha256,
        retained: remaining(&root),
        note: format!(
            "This is retained plaintext, not a locked workspace. The last committed container is separate. Preserve newer edits before manual cleanup. {LIMITS_NOTE}"
        ),
    })
}
fn cleanup_copy(
    session: &Session,
    owners: &BTreeMap<String, String>,
    snapshot: &Snapshot,
    worker_lock: File,
    receipt: Receipt,
    c: &Control,
) -> LockReport {
    let mut removed = vec![];
    let mut cancelled = false;
    let check = Control::default();
    let root = &session.root;
    let owner_ok = no_links(root).is_ok()
        && ["notegen-session.json", WORK_MARKER].iter().all(|name| {
            hash_file(&root.join(name), &check).is_ok_and(|hash| hash == owners[*name])
        });
    if owner_ok {
        // Prevent a new ordinary session operation before releasing its lock.
        let _ = fs::remove_file(root.join("notegen-session.json"));
        for file in &snapshot.files {
            if c.check_cancelled().is_err() {
                cancelled = true;
                break;
            }
            let path = &file.source;
            if !path.starts_with(root) || no_links(path).is_err() {
                continue;
            }
            if fs::symlink_metadata(path).is_ok_and(|m| {
                m.is_file()
                    && m.len() == file.entry.bytes
                    && m.modified().is_ok_and(|time| time == file.modified)
            }) && hash_file(path, &check).is_ok_and(|hash| hash == file.entry.sha256)
                && fs::remove_file(path).is_ok()
            {
                removed.push(path.clone());
            }
        }
        let mut directories = snapshot.directories.clone();
        directories.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
        for directory in directories {
            let path = root.join(directory);
            if no_links(&path).is_ok() {
                let _ = fs::remove_dir(path);
            }
        }
        let path = root.join("retention.json");
        if no_links(&path).is_ok()
            && hash_file(&path, &check).is_ok_and(|hash| hash == owners["retention.json"])
        {
            let _ = fs::remove_file(path);
        }
    }
    drop(worker_lock);
    if owner_ok {
        let path = root.join("session.lock");
        if no_links(&path).is_ok()
            && hash_file(&path, &check).is_ok_and(|hash| hash == owners["session.lock"])
        {
            let _ = fs::remove_file(path);
        }
        let left = remaining(root);
        if left
            .iter()
            .all(|entry| entry.path == root.join(WORK_MARKER))
        {
            let path = root.join(WORK_MARKER);
            if hash_file(&path, &check).is_ok_and(|hash| hash == owners[WORK_MARKER]) {
                let _ = fs::remove_file(path);
            }
            let _ = fs::remove_dir(root);
        }
    }
    let mut retained = if root.exists() {
        remaining(root)
    } else {
        vec![]
    };
    if root.exists() && retained.is_empty() {
        retained.push(Retained {
            path: root.clone(),
            reason: "Empty marked work directory could not be removed".into(),
        });
    }
    LockReport {
        fully_locked: retained.is_empty(),
        receipt,
        removed,
        retained,
        cancelled,
        note: LIMITS_NOTE.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reads_and_retains_control_file_bytes_without_releasing_its_exclusion() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("project.lock");
        let bytes = b"retained nonempty control-file bytes\r\n";
        fs::write(&path, bytes).unwrap();
        let mut snapshot = Snapshot {
            files: vec![],
            directories: vec![],
            observed_directories: vec![],
            total_bytes: 0,
        };
        add_file(
            &mut snapshot,
            &path,
            "work/project.lock".into(),
            &Options::default(),
            &Control::default(),
        )
        .unwrap();
        let competitor = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(
            competitor.try_lock().is_err(),
            "snapshot must keep its exclusive worker guard"
        );
        for _ in 0..2 {
            assert_eq!(
                source_hash(&snapshot.files[0], &Control::default()).unwrap(),
                digest(bytes)
            );
            let mut reader = source_reader(&snapshot.files[0]).unwrap();
            let mut actual = vec![];
            reader.read_to_end(&mut actual).unwrap();
            assert_eq!(actual, bytes);
        }
        unchanged(&snapshot, &Control::default()).unwrap();
        assert!(
            competitor.try_lock().is_err(),
            "verification must not briefly release the guard"
        );
        drop(snapshot);
        competitor.try_lock().unwrap();
        competitor.unlock().unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn encrypted_snapshot_refuses_pending_rebased_migration_journals_without_touching_sources() {
        let temp = tempfile::tempdir().unwrap();
        let journal = temp.path().join("project-transaction.json");
        fs::write(
            temp.path().join("project.json"),
            b"original project metadata",
        )
        .unwrap();
        fs::write(&journal, b"pending canonical-bound migration").unwrap();
        let mut snapshot = Snapshot {
            files: vec![],
            directories: vec![],
            observed_directories: vec![],
            total_bytes: 0,
        };
        assert!(
            add_tree(
                &mut snapshot,
                temp.path(),
                "work/project",
                &Options::default(),
                &Control::default()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(journal).unwrap(),
            b"pending canonical-bound migration"
        );
        assert_eq!(
            fs::read(temp.path().join("project.json")).unwrap(),
            b"original project metadata"
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_private_acl_handles_open_quoted_paths_and_inherits_to_new_files() {
        use std::{os::windows::process::CommandExt, process::Command};
        let temp = tempfile::Builder::new()
            .prefix("vault ACL ' & σ ")
            .tempdir()
            .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let held = tempfile::NamedTempFile::new_in(&root).unwrap();
        private_permissions(held.path(), false, &Control::default()).unwrap();
        private_permissions(&root, true, &Control::default()).unwrap();
        let child = root.join("child ' & σ.txt");
        File::create(&child).unwrap();
        const VERIFY: &str = r#"$ErrorActionPreference='Stop'
try{
 $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User
 $acl=[System.IO.File]::GetAccessControl($env:NOTEGEN_VAULT_PERMISSION_TARGET)
 $rules=@($acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier]))
 if($rules.Count -ne 1 -or !$rules[0].IsInherited -or $rules[0].IdentityReference.Value -ne $sid.Value -or $rules[0].AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow -or $rules[0].FileSystemRights -ne [System.Security.AccessControl.FileSystemRights]::FullControl){exit 2}
 [Console]::Out.WriteLine('PRIVATE_CHILD_OK')
}catch{exit 3}
"#;
        let encoded = STANDARD.encode(
            VERIFY
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child = Command::new(executable)
            .creation_flags(0x08000000)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &encoded,
            ])
            .env("NOTEGEN_VAULT_PERMISSION_TARGET", &child)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if start.elapsed() > Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Child ACL query timed out");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "Child ACL verification failed: {:?}",
            output.status.code()
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "PRIVATE_CHILD_OK"
        );
    }

    // The attacker can construct an otherwise authentic age payload (for example
    // from an imported container). Its private names and lengths remain untrusted.
    fn hostile_container(
        path: &Path,
        header: &Header,
        key: &age::x25519::Identity,
        manifest: &Inventory,
    ) {
        let mut file = File::create(path).unwrap();
        write_prefix(&mut file, header).unwrap();
        let recipient = key.to_public();
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .unwrap();
        let mut writer = encryptor.wrap_output(file).unwrap();
        let private = serde_json::to_vec(manifest).unwrap();
        writer.write_all(PRIVATE_MAGIC).unwrap();
        writer
            .write_all(&(private.len() as u32).to_be_bytes())
            .unwrap();
        writer.write_all(&private).unwrap();
        // Valid empty files; malformed sizes must be rejected before reading.
        writer.finish().unwrap().sync_all().unwrap();
    }

    #[test]
    fn authentic_untrusted_manifests_cannot_extract_traversal_aliases_or_unbounded_files() {
        let temp = tempfile::tempdir().unwrap();
        let key_path = temp.path().join("key.agekey");
        generate_recovery_key(&key_path, &Control::default()).unwrap();
        let key = recovery_identity(&key_path).unwrap();
        let pass = SecretString::from("a bounded fixture passphrase".to_string());
        let header = Header {
            schema: 1,
            vault_id: "a".repeat(64),
            wrapped_workspace_key: wrap_key(&key, &pass, 16).unwrap(),
        };
        let base = Inventory {
            schema: 1,
            vault_id: header.vault_id.clone(),
            revision: 1,
            header_sha256: digest(&serde_json::to_vec(&header).unwrap()),
            recovery_recipient: key.to_public().to_string(),
            project_relative: "work/project".into(),
            original_project_root: None,
            directories: vec!["work".into(), "work/project".into()],
            files: vec![FileEntry {
                path: "work/project/project.json".into(),
                bytes: 0,
                sha256: digest(b""),
            }],
            note: String::new(),
        };
        let mut cases = vec![];
        for name in [
            "../escaped",
            "/escaped",
            "work/project/../escaped",
            "work/C:escape",
            "work/CON.txt",
            "work/project/trailing. ",
        ] {
            let mut manifest = base.clone();
            manifest.files.push(FileEntry {
                path: name.into(),
                bytes: 0,
                sha256: digest(b""),
            });
            cases.push(manifest);
        }
        for (left, right) in [
            ("σ.txt", "ς.txt"),
            ("Straße.txt", "STRASSE.txt"),
            ("Ａ.txt", "a.txt"),
        ] {
            let mut manifest = base.clone();
            for name in [left, right] {
                manifest.files.push(FileEntry {
                    path: format!("work/project/{name}"),
                    bytes: 0,
                    sha256: digest(b""),
                });
            }
            cases.push(manifest);
        }
        let mut file_directory_alias = base.clone();
        file_directory_alias
            .directories
            .push("work/project/PROJECT.JSON".into());
        cases.push(file_directory_alias);
        let mut undeclared = base.clone();
        undeclared.files.push(FileEntry {
            path: "work/missing/hidden.txt".into(),
            bytes: 0,
            sha256: digest(b""),
        });
        cases.push(undeclared);
        let mut huge = base.clone();
        huge.files[0].bytes = HARD_FILE_BYTES + 1;
        cases.push(huge);
        let mut missing = base.clone();
        missing.files.clear();
        cases.push(missing);
        let parent = temp.path().join("plaintext");
        fs::create_dir(&parent).unwrap();
        for (index, manifest) in cases.iter().enumerate() {
            let path = temp.path().join(format!("hostile-{index}.ngvault"));
            hostile_container(&path, &header, &key, manifest);
            assert!(
                unlock(
                    &path,
                    &Credentials::RecoveryKey(&key_path),
                    &parent,
                    Some(&pass),
                    &Options {
                        scrypt_log_n: 16,
                        ..Default::default()
                    },
                    &Control::default()
                )
                .is_err(),
                "hostile case {index}"
            );
            assert!(
                fs::read_dir(&parent).unwrap().next().is_none(),
                "no staging copy for case {index}"
            );
            assert!(!temp.path().join("escaped").exists());
        }
    }

    #[test]
    fn malicious_scrypt_cost_and_header_lengths_are_rejected_before_kdf_or_allocation() {
        for factor in ["255", "19", "15", "18446744073709551615"] {
            let bytes = format!(
                "age-encryption.org/v1\n-> scrypt AAAAAAAAAAAAAAAAAAAAAA {factor}\nAAAA\n--- AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n"
            );
            assert!(age_header(&mut bytes.as_bytes(), WRAPPER_LIMIT, true).is_err());
        }
        let oversized_line = format!("age-encryption.org/v1\n{}\n", "x".repeat(4097));
        assert!(age_header(&mut oversized_line.as_bytes(), AGE_HEADER_LIMIT, false).is_err());
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large-header.ngvault");
        let mut file = File::create(&path).unwrap();
        file.write_all(MAGIC).unwrap();
        file.write_all(&u32::MAX.to_be_bytes()).unwrap();
        drop(file);
        assert!(read_header(&mut File::open(path).unwrap()).is_err());
    }
}
