//! Immutable, verified content-addressed model storage. No asset is deleted here.
use crate::{
    models::{self, Asset},
    planning::Model,
    storage::{Activity, Control, fingerprint, read_json},
    types::*,
    watch::no_links,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const MAX_BYTES: u64 = 100_000_000_000;
const REF_BYTES: usize = 100_000;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Gguf,
    RuntimeFile,
    Archive,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ObjectId {
    pub sha256: String,
    pub bytes: u64,
}
impl ObjectId {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            hash_valid(&self.sha256) && (1..=MAX_BYTES).contains(&self.bytes),
            fault("Invalid immutable asset identity")
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionReference {
    pub schema: u32,
    pub id: String,
    /// Descriptor paths are deliberately empty; object identities resolve them.
    pub model: Model,
    pub weights: ObjectId,
    pub projector: ObjectId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredModel {
    pub model: Model,
    pub reference: VersionReference,
}
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}
pub fn default_root() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    let base = base.filter(|p| p.is_absolute()).ok_or_else(|| {
        fault("Per-user data folder is unavailable; select an explicit shared model store")
    })?;
    Ok(base.join("NoteGen/model-store"))
}
fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG650", message).into()
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn kind_valid(kind: Kind, prefix: &[u8]) -> bool {
    kind != Kind::Gguf || prefix.starts_with(b"GGUF")
}
pub(crate) fn inspect(path: &Path, limit: u64, kind: Kind, c: &Control) -> Result<ObjectId> {
    c.check()?;
    no_links(path)?;
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() > 0 && metadata.len() <= limit,
        fault("Asset is not a bounded regular file")
    );
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    let mut prefix = vec![];
    loop {
        c.check()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or_else(|| fault("Asset size overflow"))?;
        anyhow::ensure!(bytes <= limit, fault("Asset grew beyond its size limit"));
        if prefix.len() < 4 {
            prefix.extend_from_slice(&buffer[..count.min(4 - prefix.len())]);
        }
        hash.update(&buffer[..count]);
    }
    anyhow::ensure!(
        bytes == metadata.len() && kind_valid(kind, &prefix),
        fault("Asset changed or lacks the required GGUF signature")
    );
    no_links(path)?;
    Ok(ObjectId {
        sha256: hex::encode(hash.finalize()),
        bytes,
    })
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        no_links(root)?;
        anyhow::ensure!(
            !root.as_os_str().is_empty(),
            fault("Choose a shared model store folder")
        );
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        for folder in ["objects", "references", "locks", "incoming"] {
            let path = root.join(folder);
            no_links(&path)?;
            fs::create_dir_all(path)?;
        }
        Ok(Self { root })
    }
    pub fn read(root: &Path) -> Result<Self> {
        no_links(root)?;
        anyhow::ensure!(
            root.is_dir(),
            fault("Shared model store is unavailable; restore its exact objects first")
        );
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    pub fn object_path(&self, id: &ObjectId) -> Result<PathBuf> {
        id.validate()?;
        Ok(self
            .root
            .join("objects")
            .join(&id.sha256[..2])
            .join(format!("{}.blob", id.sha256)))
    }
    fn object_lock(&self, id: &ObjectId, c: &Control) -> Result<File> {
        id.validate()?;
        no_links(&self.root)?;
        let path = self.root.join("locks").join(format!("{}.lock", id.sha256));
        no_links(&path)?;
        fs::create_dir_all(path.parent().unwrap())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        loop {
            c.check()?;
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(50)),
                Err(error) => return Err(fault(&format!("Cannot lock immutable asset: {error}"))),
            }
        }
    }
    pub fn resolve(&self, id: &ObjectId, kind: Kind, c: &Control) -> Result<Option<PathBuf>> {
        let path = self.object_path(id)?;
        no_links(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                anyhow::ensure!(
                    fs::metadata(&path)?.permissions().readonly()
                        && inspect(&path, id.bytes, kind, c)? == *id,
                    fault("Immutable object changed; preserve it and restore to a clean store")
                );
                Ok(Some(path))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    /// Copy external bytes: a hard link would allow the original to mutate the CAS.
    pub fn ingest(&self, source: &Path, id: &ObjectId, kind: Kind, c: &Control) -> Result<PathBuf> {
        let _lock = self.object_lock(id, c)?;
        self.ingest_locked(source, id, kind, c)
    }
    fn ingest_locked(
        &self,
        source: &Path,
        id: &ObjectId,
        kind: Kind,
        c: &Control,
    ) -> Result<PathBuf> {
        if let Some(path) = self.resolve(id, kind, c)? {
            return Ok(path);
        }
        no_links(source)?;
        let mut input = File::open(source)?;
        anyhow::ensure!(
            input.metadata()?.is_file(),
            fault("Immutable asset source must be regular")
        );
        let target = self.object_path(id)?;
        let parent = target.parent().unwrap();
        no_links(parent)?;
        fs::create_dir_all(parent)?;
        let mut output = tempfile::NamedTempFile::new_in(parent)?;
        let mut hash = Sha256::new();
        let mut received = 0u64;
        let mut buffer = [0u8; 65536];
        let mut prefix = vec![];
        loop {
            c.check()?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            received = received
                .checked_add(count as u64)
                .ok_or_else(|| fault("Immutable asset size overflow"))?;
            anyhow::ensure!(
                received <= id.bytes,
                fault("Asset source exceeds the expected identity")
            );
            if prefix.len() < 4 {
                prefix.extend_from_slice(&buffer[..count.min(4 - prefix.len())]);
            }
            output.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
        }
        anyhow::ensure!(
            received == id.bytes
                && hex::encode(hash.finalize()) == id.sha256
                && kind_valid(kind, &prefix),
            fault("Immutable asset size/SHA256/GGUF verification failed")
        );
        output.as_file().sync_all()?;
        no_links(&target)?;
        c.check()?;
        let file = output.persist_noclobber(&target).map_err(|e| e.error)?;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(true);
        file.set_permissions(permissions)?;
        self.resolve(id, kind, c)?
            .ok_or_else(|| fault("Immutable asset publication is unavailable"))
    }
    pub fn import_file(&self, path: &Path, kind: Kind, c: &Control) -> Result<(ObjectId, PathBuf)> {
        let id = inspect(path, MAX_BYTES, kind, c)?;
        let stored = self.ingest(path, &id, kind, c)?;
        Ok((id, stored))
    }
    pub fn download_object(
        &self,
        url: &str,
        asset: &Asset,
        kind: Kind,
        c: &Control,
        log: &Activity,
    ) -> Result<(ObjectId, PathBuf)> {
        let id = ObjectId {
            sha256: asset.sha256.clone(),
            bytes: asset.bytes,
        };
        id.validate()?;
        let _lock = self.object_lock(&id, c)?;
        if let Some(path) = self.resolve(&id, kind, c)? {
            return Ok((id, path));
        }
        let incoming = self
            .root
            .join("incoming")
            .join(format!("{}.download", id.sha256));
        no_links(&incoming)?;
        // The central manager owns .part/range/rate/policy behavior. Incoming
        // files remain available after cancellation; activation is separate.
        models::download(url, &incoming, asset, c, log)?;
        let path = self.ingest_locked(&incoming, &id, kind, c)?;
        anyhow::ensure!(
            inspect(&incoming, id.bytes, kind, c)? == id,
            fault("Download cache changed after immutable publication")
        );
        no_links(&incoming)?;
        c.check()?;
        fs::remove_file(&incoming)?;
        Ok((id, path))
    }
    pub fn adopt_model(&self, model: &Model, c: &Control) -> Result<StoredModel> {
        models::validate_model(model)?;
        anyhow::ensure!(
            model.endpoint.is_none(),
            fault("HTTP readers cannot be pinned as immutable local GGUF objects")
        );
        let (weights, weights_path) = self.import_file(&model.weights, Kind::Gguf, c)?;
        let (projector, projector_path) = self.import_file(&model.projector, Kind::Gguf, c)?;
        let digest = fingerprint(&(&weights.sha256, &projector.sha256))?;
        anyhow::ensure!(
            model.sha256.is_empty() || model.sha256 == digest,
            fault("Reader object pair changed before adoption")
        );
        let mut stored = model.clone();
        stored.weights = weights_path;
        stored.projector = projector_path;
        stored.sha256 = digest;
        stored.weights_mib = (weights.bytes + projector.bytes).div_ceil(1048576);
        stored.resident_mib = stored.resident_mib.max(stored.weights_mib + 512);
        let reference = self.write_reference(&stored, weights, projector, c)?;
        Ok(StoredModel {
            model: stored,
            reference,
        })
    }
    pub fn write_reference(
        &self,
        model: &Model,
        weights: ObjectId,
        projector: ObjectId,
        c: &Control,
    ) -> Result<VersionReference> {
        models::validate_model(model)?;
        weights.validate()?;
        projector.validate()?;
        anyhow::ensure!(
            model.endpoint.is_none()
                && model.sha256 == fingerprint(&(&weights.sha256, &projector.sha256))?,
            fault("Version reference does not match its immutable object pair")
        );
        anyhow::ensure!(
            self.resolve(&weights, Kind::Gguf, c)?.is_some()
                && self.resolve(&projector, Kind::Gguf, c)?.is_some(),
            fault("Version reference requires both verified objects")
        );
        let mut descriptor = model.clone();
        descriptor.weights = PathBuf::new();
        descriptor.projector = PathBuf::new();
        let id = fingerprint(&(1, &descriptor, &weights, &projector))?;
        let reference = VersionReference {
            schema: 1,
            id,
            model: descriptor,
            weights,
            projector,
        };
        let bytes = serde_json::to_vec_pretty(&reference)?;
        anyhow::ensure!(
            bytes.len() <= REF_BYTES,
            fault("Version reference exceeds its metadata budget")
        );
        let folder = self.root.join("references").join(&reference.model.id);
        no_links(&folder)?;
        fs::create_dir_all(&folder)?;
        let path = folder.join(format!("{}.json", reference.id));
        no_links(&path)?;
        if path.exists() {
            let existing = self.read_reference(&reference.model.id, &reference.id, c)?;
            anyhow::ensure!(
                fingerprint(&existing)? == fingerprint(&reference)?,
                fault("Immutable version reference changed")
            );
            return Ok(existing);
        }
        let mut file = tempfile::NamedTempFile::new_in(&folder)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        c.check()?;
        match file.persist_noclobber(&path) {
            Ok(_) => Ok(reference),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.read_reference(&reference.model.id, &reference.id, c)?;
                anyhow::ensure!(
                    fingerprint(&existing)? == fingerprint(&reference)?,
                    fault("Concurrent immutable version reference changed")
                );
                Ok(existing)
            }
            Err(error) => Err(error.error.into()),
        }
    }
    pub fn read_reference(
        &self,
        model_id: &str,
        id: &str,
        c: &Control,
    ) -> Result<VersionReference> {
        c.check()?;
        anyhow::ensure!(
            crate::writers::valid_id(model_id) && hash_valid(id),
            fault("Invalid immutable version reference identity")
        );
        let path = self
            .root
            .join("references")
            .join(model_id)
            .join(format!("{id}.json"));
        no_links(&path)?;
        let reference: VersionReference = read_json(&path, REF_BYTES as u64)?;
        models::validate_model(&reference.model)?;
        reference.weights.validate()?;
        reference.projector.validate()?;
        anyhow::ensure!(
            reference.schema == 1
                && reference.id == id
                && reference.model.id == model_id
                && reference.model.endpoint.is_none()
                && reference.model.weights.as_os_str().is_empty()
                && reference.model.projector.as_os_str().is_empty()
                && fingerprint(&(
                    1,
                    &reference.model,
                    &reference.weights,
                    &reference.projector
                ))? == id
                && reference.model.sha256
                    == fingerprint(&(&reference.weights.sha256, &reference.projector.sha256))?,
            fault("Version reference metadata is inconsistent")
        );
        Ok(reference)
    }
    pub fn resolve_reference(&self, reference: &VersionReference, c: &Control) -> Result<Model> {
        let current = self.read_reference(&reference.model.id, &reference.id, c)?;
        anyhow::ensure!(
            fingerprint(&current)? == fingerprint(reference)?,
            fault("Selected immutable reference changed")
        );
        let mut model = current.model;
        model.weights = self
            .resolve(&current.weights, Kind::Gguf, c)?
            .ok_or_else(|| fault("Pinned weights are missing; restore the exact SHA256 object"))?;
        model.projector = self
            .resolve(&current.projector, Kind::Gguf, c)?
            .ok_or_else(|| fault("Pinned projector is missing; restore the exact SHA256 object"))?;
        Ok(model)
    }
    pub fn versions(&self, model_id: &str, c: &Control) -> Result<Vec<VersionReference>> {
        anyhow::ensure!(
            crate::writers::valid_id(model_id),
            fault("Invalid model reference ID")
        );
        let folder = self.root.join("references").join(model_id);
        no_links(&folder)?;
        if !folder.exists() {
            return Ok(vec![]);
        }
        let mut refs = vec![];
        let mut seen = BTreeSet::new();
        for entry in fs::read_dir(folder)? {
            c.check()?;
            let path = entry?.path();
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| fault("Invalid immutable reference filename"))?;
            anyhow::ensure!(
                refs.len() < 1000
                    && path.extension().is_some_and(|s| s == "json")
                    && hash_valid(stem)
                    && seen.insert(stem.to_owned()),
                fault("Unexpected or excessive immutable version references")
            );
            refs.push(self.read_reference(model_id, stem, c)?);
        }
        refs.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(refs)
    }
}
