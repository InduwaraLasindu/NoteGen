//! In-app request inventory. No URL paths, queries, credentials or payloads are recorded.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Reader,
    ReaderStatus,
    Download,
    UpdateCheck,
    PublicLookup,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Active {
    pub id: u64,
    pub purpose: Purpose,
    pub destination: String,
    pub seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub active: Vec<Active>,
    pub additional_active: usize,
    pub note: &'static str,
}
struct Request {
    purpose: Purpose,
    destination: String,
    started: Instant,
}
#[derive(Default)]
struct Registry {
    next: u64,
    active: BTreeMap<u64, Request>,
    additional: usize,
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
fn destination(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            if !matches!(url.scheme(), "http" | "https") {
                return None;
            }
            let host = url.host_str()?;
            let host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.into()
            };
            Some(format!(
                "{}://{}:{}",
                url.scheme(),
                host,
                url.port_or_known_default()?
            ))
        })
        .unwrap_or_else(|| "Unparsed endpoint".into())
}
pub struct Guard {
    id: Option<u64>,
}
impl Guard {
    /// Update to the response's actual origin after a permitted publisher redirect.
    pub fn response_url(&self, url: &reqwest::Url) {
        let mut state = registry().lock().unwrap_or_else(|error| error.into_inner());
        if let Some(request) = self.id.and_then(|id| state.active.get_mut(&id)) {
            request.destination = destination(url.as_str());
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let mut state = registry().lock().unwrap_or_else(|error| error.into_inner());
        if let Some(id) = self.id {
            state.active.remove(&id);
        } else {
            state.additional = state.additional.saturating_sub(1);
        }
    }
}
pub fn begin(url: &str, purpose: Purpose) -> Guard {
    let mut state = registry().lock().unwrap_or_else(|error| error.into_inner());
    if state.active.len() >= 128 {
        state.additional = state.additional.saturating_add(1);
        return Guard { id: None };
    }
    state.next = state.next.wrapping_add(1);
    let id = state.next;
    if state.active.contains_key(&id) {
        state.additional = state.additional.saturating_add(1);
        return Guard { id: None };
    }
    state.active.insert(
        id,
        Request {
            purpose,
            destination: destination(url),
            started: Instant::now(),
        },
    );
    Guard { id: Some(id) }
}
pub fn snapshot() -> Snapshot {
    let state = registry().lock().unwrap_or_else(|error| error.into_inner());
    Snapshot {
        active: state
            .active
            .iter()
            .map(|(id, request)| Active {
                id: *id,
                purpose: request.purpose,
                destination: request.destination.clone(),
                seconds: request.started.elapsed().as_secs_f64(),
            })
            .collect(),
        additional_active: state.additional,
        note: "Active NoteGen HTTP requests, including response reads. This does not inspect other applications or OS sockets. Destinations show origins only; paths, queries, credentials and document content are omitted.",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_omits_private_url_parts_and_follows_actual_response_origin() {
        let guard = begin(
            "https://user:private@example.com:8443/secret/file?token=private#private",
            Purpose::Download,
        );
        let id = guard.id.unwrap();
        let active = snapshot()
            .active
            .into_iter()
            .find(|entry| entry.id == id)
            .unwrap();
        assert_eq!(active.destination, "https://example.com:8443");
        assert!(!serde_json::to_string(&active).unwrap().contains("private"));
        guard.response_url(
            &reqwest::Url::parse("https://cdn.example.com/object?signature=private").unwrap(),
        );
        assert_eq!(
            snapshot()
                .active
                .into_iter()
                .find(|entry| entry.id == id)
                .unwrap()
                .destination,
            "https://cdn.example.com:443"
        );
        drop(guard);
        assert!(!snapshot().active.iter().any(|entry| entry.id == id));
    }
    #[test]
    fn cancelled_and_unwinding_operations_release_inventory_entries() {
        let (id, guard) = {
            let guard = begin("http://127.0.0.1:18745/read", Purpose::Reader);
            (guard.id.unwrap(), guard)
        };
        assert!(snapshot().active.iter().any(|entry| entry.id == id));
        drop(guard);
        assert!(!snapshot().active.iter().any(|entry| entry.id == id));
        let mut id = 0;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let guard = begin("https://example.com/private", Purpose::PublicLookup);
            id = guard.id.unwrap();
            panic!("injected request failure");
        }));
        assert!(!snapshot().active.iter().any(|entry| entry.id == id));
    }
}
