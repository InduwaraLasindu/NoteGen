//! A fixed Windows credential entry for private publisher update access.
//! Secrets never enter settings, diagnostics, serialization, Debug output or error details.
use crate::types::*;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const SERVICE: &str = "NoteGen.PrivateUpdates";
pub const ACCOUNT: &str = "github.com/InduwaraLasindu/NoteGen";
const LATEST: &str = "/repos/InduwaraLasindu/NoteGen/releases/latest";
const ASSET_PREFIX: &str = "/repos/InduwaraLasindu/NoteGen/releases/assets/";

/// Deliberately implements neither Debug, Display, Clone nor Serialize.
pub struct Secret(Zeroizing<Vec<u8>>);
impl Secret {
    pub fn new(value: String) -> Result<Self> {
        Self::from_bytes(value.into_bytes())
    }
    fn from_bytes(value: Vec<u8>) -> Result<Self> {
        let value = Zeroizing::new(value);
        anyhow::ensure!(
            (16..=2048).contains(&value.len())
                && value
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(b)),
            Fault::new(
                "NG750",
                "Paste only a 16–2048 byte bearer token without spaces or control characters"
            )
        );
        Ok(Self(value))
    }
    /// Explicit access for the known publisher request's Authorization header.
    pub fn expose_secret(&self) -> &str {
        std::str::from_utf8(&self.0).expect("validated ASCII bearer token")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unsupported,
    Unavailable,
}
pub type StoreResult<T> = std::result::Result<T, StoreError>;
/// Injectable boundary used by tests; the production backend always uses fixed identifiers.
pub trait CredentialBackend {
    fn supported(&self) -> bool;
    fn contains(&self) -> StoreResult<bool>;
    fn read(&self) -> StoreResult<Option<Secret>>;
    fn write(&self, secret: &Secret) -> StoreResult<()>;
    fn remove(&self) -> StoreResult<()>;
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialStatus {
    pub supported: bool,
    pub stored: bool,
    pub publisher: String,
    pub storage: String,
}
pub struct Credentials<B: CredentialBackend> {
    backend: B,
}
impl<B: CredentialBackend> Credentials<B> {
    pub fn with_backend(backend: B) -> Self {
        Self { backend }
    }
    pub fn status(&self) -> Result<CredentialStatus> {
        let supported = self.backend.supported();
        let stored = if supported {
            self.backend.contains().map_err(store_error)?
        } else {
            false
        };
        Ok(CredentialStatus {
            supported,
            stored,
            publisher: ACCOUNT.into(),
            storage: if supported {
                "Windows Credential Manager"
            } else {
                "Unavailable on this platform"
            }
            .into(),
        })
    }
    pub fn set(&self, secret: Secret) -> Result<()> {
        if !self.backend.supported() {
            return Err(store_error(StoreError::Unsupported));
        }
        self.backend.write(&secret).map_err(store_error)
    }
    /// Validate the full destination before any credential-store read.
    /// Public checks remain usable on unsupported systems without a plaintext fallback.
    pub fn get_for_publisher(&self, url: &str) -> Result<Option<Secret>> {
        anyhow::ensure!(
            publisher_url(url),
            Fault::new(
                "NG750",
                "Stored credentials can only be retrieved for the fixed publisher release API"
            )
        );
        if !self.backend.supported() {
            return Ok(None);
        }
        self.backend.read().map_err(store_error)
    }
    pub fn delete(&self) -> Result<()> {
        if !self.backend.supported() {
            return Err(store_error(StoreError::Unsupported));
        }
        self.backend.remove().map_err(store_error)
    }
}
fn store_error(error: StoreError) -> anyhow::Error {
    Fault::new("NG750",match error {
        StoreError::Unsupported=>"Private-update credentials require Windows Credential Manager; no file or environment fallback is used",
        StoreError::Unavailable=>"Windows Credential Manager is unavailable or denied access; the stored entry was not exposed",
    }).into()
}
pub fn publisher_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("api.github.com")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && (url.path() == LATEST
            || url.path().strip_prefix(ASSET_PREFIX).is_some_and(|id| {
                !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
            }))
}

#[derive(Default)]
struct NativeStore {
    #[cfg(all(test, windows))]
    test_service: Option<String>,
}
pub fn native() -> Credentials<impl CredentialBackend> {
    Credentials::with_backend(NativeStore::default())
}

#[cfg(windows)]
static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(windows)]
impl NativeStore {
    fn entry(&self) -> StoreResult<keyring::Entry> {
        #[cfg(test)]
        let service = self.test_service.as_deref().unwrap_or(SERVICE);
        #[cfg(not(test))]
        let service = SERVICE;
        // Construct the native backend explicitly; never inherit a process-wide mock builder.
        let credential = keyring::windows::WinCredential::new_with_target(None, service, ACCOUNT)
            .map_err(|_| StoreError::Unavailable)?;
        Ok(keyring::Entry::new_with_credential(Box::new(credential)))
    }
}
#[cfg(windows)]
impl CredentialBackend for NativeStore {
    fn supported(&self) -> bool {
        true
    }
    fn contains(&self) -> StoreResult<bool> {
        let _guard = STORE_LOCK.lock().map_err(|_| StoreError::Unavailable)?;
        match self.entry()?.get_attributes() {
            Ok(_) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err(StoreError::Unavailable),
        }
    }
    fn read(&self) -> StoreResult<Option<Secret>> {
        let _guard = STORE_LOCK.lock().map_err(|_| StoreError::Unavailable)?;
        match self.entry()?.get_secret() {
            Ok(bytes) => Secret::from_bytes(bytes)
                .map(Some)
                .map_err(|_| StoreError::Unavailable),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(StoreError::Unavailable),
        }
    }
    fn write(&self, secret: &Secret) -> StoreResult<()> {
        let _guard = STORE_LOCK.lock().map_err(|_| StoreError::Unavailable)?;
        self.entry()?
            .set_secret(secret.expose_secret().as_bytes())
            .map_err(|_| StoreError::Unavailable)
    }
    fn remove(&self) -> StoreResult<()> {
        let _guard = STORE_LOCK.lock().map_err(|_| StoreError::Unavailable)?;
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(StoreError::Unavailable),
        }
    }
}
#[cfg(not(windows))]
impl CredentialBackend for NativeStore {
    fn supported(&self) -> bool {
        false
    }
    fn contains(&self) -> StoreResult<bool> {
        Err(StoreError::Unsupported)
    }
    fn read(&self) -> StoreResult<Option<Secret>> {
        Err(StoreError::Unsupported)
    }
    fn write(&self, _: &Secret) -> StoreResult<()> {
        Err(StoreError::Unsupported)
    }
    fn remove(&self) -> StoreResult<()> {
        Err(StoreError::Unsupported)
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    #[ignore = "Explicit opt-in: writes and removes only a uniquely named synthetic Windows credential, never the production entry"]
    fn isolated_native_store_round_trip_and_cleanup() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let store = NativeStore {
            test_service: Some(format!("NoteGen.TestOnly.{}.{unique}", std::process::id())),
        };
        struct Cleanup<'a>(&'a NativeStore);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.remove();
            }
        }
        let _cleanup = Cleanup(&store);
        assert!(!store.contains().unwrap());
        let token = Secret::new("ghp_SYNTHETIC_TEST_ONLY_0123456789abcdef".into()).unwrap();
        store.write(&token).unwrap();
        assert!(store.contains().unwrap());
        let fetched = store.read().unwrap().unwrap();
        assert!(fetched.expose_secret() == token.expose_secret());
        store.remove().unwrap();
        assert!(!store.contains().unwrap());
        assert!(store.read().unwrap().is_none());
    }
}
