use notegen::credentials::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const URL: &str = "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/latest";
const TOKEN: &str = "ghp_SYNTHETIC_TEST_ONLY_0123456789abcdef";
struct State {
    value: Mutex<Option<String>>,
    calls: Mutex<Vec<&'static str>>,
    failed: AtomicBool,
}
struct Fake {
    state: Arc<State>,
    supported: bool,
}
impl Fake {
    fn result(&self, name: &'static str) -> StoreResult<()> {
        self.state.calls.lock().unwrap().push(name);
        if self.state.failed.load(Ordering::Relaxed) {
            Err(StoreError::Unavailable)
        } else {
            Ok(())
        }
    }
}
impl CredentialBackend for Fake {
    fn supported(&self) -> bool {
        self.supported
    }
    fn contains(&self) -> StoreResult<bool> {
        self.result("contains")?;
        Ok(self.state.value.lock().unwrap().is_some())
    }
    fn read(&self) -> StoreResult<Option<Secret>> {
        self.result("read")?;
        Ok(self
            .state
            .value
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| Secret::new(s.clone()).unwrap()))
    }
    fn write(&self, secret: &Secret) -> StoreResult<()> {
        self.result("write")?;
        *self.state.value.lock().unwrap() = Some(secret.expose_secret().into());
        Ok(())
    }
    fn remove(&self) -> StoreResult<()> {
        self.result("remove")?;
        *self.state.value.lock().unwrap() = None;
        Ok(())
    }
}
fn fixture(supported: bool) -> (Credentials<Fake>, Arc<State>) {
    let state = Arc::new(State {
        value: Mutex::new(None),
        calls: Mutex::new(vec![]),
        failed: AtomicBool::new(false),
    });
    (
        Credentials::with_backend(Fake {
            state: state.clone(),
            supported,
        }),
        state,
    )
}
#[test]
fn explicit_set_get_status_and_delete_use_fixed_publisher_boundary() {
    let (store, state) = fixture(true);
    assert!(!store.status().unwrap().stored);
    store.set(Secret::new(TOKEN.into()).unwrap()).unwrap();
    let status = store.status().unwrap();
    assert!(status.supported && status.stored);
    assert_eq!(status.publisher, ACCOUNT);
    let secret = store.get_for_publisher(URL).unwrap().unwrap();
    assert!(secret.expose_secret() == TOKEN);
    store.delete().unwrap();
    assert!(!store.status().unwrap().stored);
    assert!(store.get_for_publisher(URL).unwrap().is_none());
    store.delete().unwrap();
    assert!(state.calls.lock().unwrap().contains(&"write"));
}
#[test]
fn rejected_urls_never_touch_the_credential_backend() {
    let (store, state) = fixture(true);
    for bad in [
        "http://api.github.com/repos/InduwaraLasindu/NoteGen/releases/latest",
        "https://api.github.com.evil.example/repos/InduwaraLasindu/NoteGen/releases/latest",
        "https://api.github.com:444/repos/InduwaraLasindu/NoteGen/releases/latest",
        "https://user:password@api.github.com/repos/InduwaraLasindu/NoteGen/releases/latest",
        "https://api.github.com/repos/Other/Repository/releases/latest",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/latest?token=secret",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/latest#fragment",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases?per_page=20&token=secret",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases?per_page=100",
        "https://api.github.com/repos/Other/Repo/releases?per_page=20",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases?per_page=20#fragment",
        "https://github.com/InduwaraLasindu/NoteGen/releases/latest",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/not-an-id",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/",
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/../issues",
        "file:///private",
        "invalid",
    ] {
        assert!(store.get_for_publisher(bad).is_err());
    }
    assert!(state.calls.lock().unwrap().is_empty());
}

#[test]
fn preview_metadata_has_one_exact_approved_url() {
    let (store, state) = fixture(true);
    store.set(Secret::new(TOKEN.into()).unwrap()).unwrap();
    assert!(publisher_metadata_url(PUBLISHER_RELEASES_URL));
    assert!(
        store
            .get_for_publisher(PUBLISHER_RELEASES_URL)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        state
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|name| **name == "read")
            .count(),
        1
    );
    assert!(!publisher_metadata_url(
        "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/321"
    ));
}
#[test]
fn numeric_publisher_assets_are_allowed_but_suffixes_and_encoded_segments_are_not() {
    for id in ["1", "1234567890", "99999999999999999999"] {
        assert!(publisher_url(&format!(
            "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/{id}"
        )));
    }
    for id in [
        "-1",
        "+1",
        "1/download",
        "%31",
        "123456789012345678901",
        "1?query=1",
        "1#part",
    ] {
        assert!(!publisher_url(&format!(
            "https://api.github.com/repos/InduwaraLasindu/NoteGen/releases/assets/{id}"
        )));
    }
}
#[test]
fn unsupported_platform_has_no_plaintext_or_environment_fallback_and_no_backend_calls() {
    let (store, state) = fixture(false);
    let status = store.status().unwrap();
    assert!(!status.supported && !status.stored);
    assert!(store.get_for_publisher(URL).unwrap().is_none());
    assert!(store.set(Secret::new(TOKEN.into()).unwrap()).is_err());
    assert!(store.delete().is_err());
    assert!(state.calls.lock().unwrap().is_empty());
}
#[test]
fn injected_backend_failures_return_only_fixed_nonsecret_errors() {
    let (store, state) = fixture(true);
    state.failed.store(true, Ordering::Relaxed);
    let errors = vec![
        store.status().err().unwrap(),
        store.get_for_publisher(URL).err().unwrap(),
        store.set(Secret::new(TOKEN.into()).unwrap()).err().unwrap(),
        store.delete().err().unwrap(),
    ];
    for e in errors {
        let text = format!("{e:?} {e}");
        assert!(!text.contains(TOKEN));
        assert!(!text.contains("password"));
        assert!(text.contains("NG750"));
    }
}
#[test]
fn tokens_validate_lengths_and_bearer_characters_without_echoing_input() {
    for bad in [
        "short".to_string(),
        " ghp_SECRET_TOKEN_HAS_SPACES ".into(),
        "ghp_SECRET_TOKEN\nnewline".into(),
        "ගුප්තපදයසිංහල".into(),
        "x".repeat(2049),
    ] {
        let error = Secret::new(bad.clone()).err().unwrap();
        assert!(!error.to_string().contains(&bad));
    }
    for value in [
        "x".repeat(16),
        "x".repeat(2048),
        TOKEN.into(),
        "abcdef0123456789-._~+/=".into(),
    ] {
        assert!(Secret::new(value).is_ok());
    }
}
#[test]
fn serialized_status_contains_only_presence_and_fixed_storage_metadata() {
    let (store, _) = fixture(true);
    store.set(Secret::new(TOKEN.into()).unwrap()).unwrap();
    let json = serde_json::to_value(store.status().unwrap()).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 4);
    let text = json.to_string();
    assert!(!text.contains(TOKEN));
    assert!(!text.contains("ghp_"));
}
#[cfg(not(windows))]
#[test]
fn native_backend_reports_unsupported_without_attempting_an_os_store() {
    let store = native();
    assert!(!store.status().unwrap().supported);
    assert!(store.get_for_publisher(URL).unwrap().is_none());
    assert!(store.delete().is_err());
}
