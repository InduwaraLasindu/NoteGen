//! Explicit, persisted asset downloads. Resuming bytes never activates a reader/runtime.
use crate::{
    models::{self, Asset},
    storage::{Activity, Control, atomic_json, fingerprint, read_json},
    types::*,
    watch::no_links,
};
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_JOBS: usize = 500;
const STORE_BYTES: usize = 4_000_000;
pub const MAX_RECEIVE_CHUNK: usize = 1_048_576;
pub const LIMITS: &str = "Explicit downloads only; no recurring external automation. Times are UTC. Unknown network cost is allowed by default. Metered detection can be unavailable; observations refresh at most every 30 seconds with a bounded two-second native query. The global rate is an average receive budget with at most one bounded 1 MiB network-chunk burst. Rate waits up to 250 ms retain the response with policy/cancellation polling every 20 ms; longer waits, pauses and closed policies close the response and resume verified partial bytes by Range. Servers ignoring Range restart from zero. Cancel preserves partial files. Restart requeues interrupted work without activating registrations. Jobs store stable publisher URLs, never tokens, signed redirect URLs, headers or network identifiers.";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkCost {
    Unmetered,
    Metered,
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    #[default]
    Allow,
    Pause,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UtcWindow {
    /// Monday=0 ... Sunday=6, anchored to the starting day.
    pub weekdays: Vec<u8>,
    pub start_minute: u16,
    pub end_minute: u16,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Zero means unlimited.
    pub bytes_per_second: u64,
    pub utc_windows: Vec<UtcWindow>,
    pub metered: NetworkPolicy,
    pub unknown: NetworkPolicy,
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.bytes_per_second <= 10_000_000_000 && self.utc_windows.len() <= 14,
            fault("Invalid download rate or window count")
        );
        for w in &self.utc_windows {
            anyhow::ensure!(
                !w.weekdays.is_empty()
                    && w.weekdays.len() <= 7
                    && w.weekdays.iter().all(|d| *d < 7)
                    && w.weekdays.iter().collect::<BTreeSet<_>>().len() == w.weekdays.len()
                    && w.start_minute < 1440
                    && w.end_minute < 1440
                    && w.start_minute != w.end_minute,
                fault("Invalid UTC download window")
            );
        }
        Ok(())
    }
    pub fn window_open(&self, now: DateTime<Utc>) -> bool {
        let day = now.weekday().num_days_from_monday() as u8;
        let minute = (now.hour() * 60 + now.minute()) as u16;
        self.utc_windows.is_empty()
            || self.utc_windows.iter().any(|w| {
                if w.start_minute < w.end_minute {
                    w.weekdays.contains(&day) && (w.start_minute..w.end_minute).contains(&minute)
                } else {
                    w.weekdays.contains(&day) && minute >= w.start_minute
                        || w.weekdays.contains(&((day + 6) % 7)) && minute < w.end_minute
                }
            })
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobOverride {
    pub allow_metered: bool,
    pub allow_unknown: bool,
    pub ignore_window: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferSpec {
    pub url: String,
    pub target: PathBuf,
    pub asset: Asset,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Downloading,
    Paused,
    WaitingWindow,
    WaitingNetwork,
    WaitingRate,
    WaitingTurn,
    Cancelled,
    Complete,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    pub transfer: TransferSpec,
    pub status: Status,
    pub user_paused: bool,
    pub override_policy: JobOverride,
    pub received_bytes: u64,
    pub error_code: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    schema: u32,
    policy: Policy,
    jobs: Vec<Job>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Ready,
    Wait(Status),
}
/// Deterministic policy evaluation; no download or lease is started by a preview.
pub fn policy_gate(
    policy: &Policy,
    over: &JobOverride,
    paused: bool,
    now: DateTime<Utc>,
    cost: NetworkCost,
) -> Result<Gate> {
    policy.validate()?;
    Ok(if paused {
        Gate::Wait(Status::Paused)
    } else if !over.ignore_window && !policy.window_open(now) {
        Gate::Wait(Status::WaitingWindow)
    } else if cost == NetworkCost::Metered
        && policy.metered == NetworkPolicy::Pause
        && !over.allow_metered
        || cost == NetworkCost::Unknown
            && policy.unknown == NetworkPolicy::Pause
            && !over.allow_unknown
    {
        Gate::Wait(Status::WaitingNetwork)
    } else {
        Gate::Ready
    })
}
#[derive(Clone, Debug, Default)]
pub struct ReceiveBudget {
    next_ms: u64,
}
impl ReceiveBudget {
    pub fn charge(&mut self, bytes: usize, now_ms: u64, bytes_per_second: u64) -> Result<()> {
        anyhow::ensure!(
            bytes <= MAX_RECEIVE_CHUNK && bytes_per_second <= 10_000_000_000,
            fault("Invalid receive-rate reservation")
        );
        if bytes_per_second > 0 {
            let delay = (bytes as u64)
                .saturating_mul(1000)
                .div_ceil(bytes_per_second);
            self.next_ms = self.next_ms.max(now_ms).saturating_add(delay);
        } else {
            self.next_ms = now_ms;
        }
        Ok(())
    }
    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        self.next_ms.saturating_sub(now_ms)
    }
}
pub trait Environment: Send + Sync {
    fn utc_now(&self) -> DateTime<Utc>;
    fn monotonic_ms(&self) -> u64;
    /// Return a cached observation without blocking the transfer policy poll.
    fn network_cost(&self) -> NetworkCost;
}
struct NetworkCache {
    observed: Option<Instant>,
    cost: NetworkCost,
    probing: bool,
}
struct SystemEnvironment {
    start: Instant,
    cached: Arc<Mutex<NetworkCache>>,
}
impl Environment for SystemEnvironment {
    fn utc_now(&self) -> DateTime<Utc> {
        Utc::now()
    }
    fn monotonic_ms(&self) -> u64 {
        self.start.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn network_cost(&self) -> NetworkCost {
        let Ok(mut cached) = self.cached.lock() else {
            return NetworkCost::Unknown;
        };
        if cached.probing
            || cached
                .observed
                .is_some_and(|when| when.elapsed() < Duration::from_secs(30))
        {
            return cached.cost;
        }
        cached.probing = true;
        let result = cached.cost;
        let cache = self.cached.clone();
        if std::thread::Builder::new()
            .name("notegen-network-cost".into())
            .spawn(move || {
                let cost = probe_network_cost();
                if let Ok(mut cache) = cache.lock() {
                    cache.cost = cost;
                    cache.observed = Some(Instant::now());
                    cache.probing = false;
                }
            })
            .is_err()
        {
            cached.probing = false;
            cached.observed = Some(Instant::now());
        }
        result
    }
}
#[cfg(not(windows))]
pub fn probe_network_cost() -> NetworkCost {
    NetworkCost::Unknown
}
#[cfg(windows)]
pub fn probe_network_cost() -> NetworkCost {
    use std::{io::Read, process::Stdio, thread};
    let code = "try {$p=[Windows.Networking.Connectivity.NetworkInformation,Windows.Networking.Connectivity,ContentType=WindowsRuntime]::GetInternetConnectionProfile(); if($null -eq $p){'Unknown'}else{$p.GetConnectionCost().NetworkCostType.ToString()}} catch {'Unknown'}";
    let Ok(mut child) = crate::hardware::command(Path::new("powershell.exe"))
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            code,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return NetworkCost::Unknown;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return NetworkCost::Unknown;
                }
                break;
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return NetworkCost::Unknown;
            }
        }
        if started.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            let _ = child.wait();
            return NetworkCost::Unknown;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let mut text = String::new();
    if let Some(out) = child.stdout.take() {
        let _ = out.take(64).read_to_string(&mut text);
    }
    match text.trim() {
        "Unrestricted" => NetworkCost::Unmetered,
        "Fixed" | "Variable" => NetworkCost::Metered,
        _ => NetworkCost::Unknown,
    }
}
struct State {
    store: Store,
    leases: BTreeMap<String, Arc<AtomicBool>>,
    active: Option<String>,
    budget: ReceiveBudget,
    last_save: Instant,
}
struct Inner {
    root: PathBuf,
    state: Mutex<State>,
    environment: Arc<dyn Environment>,
    _lock: File,
}
#[derive(Clone)]
pub struct Manager(Arc<Inner>);
pub(crate) struct Transfer {
    manager: Manager,
    id: String,
    cancelled: Arc<AtomicBool>,
    finished: bool,
}
fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG650", message).into()
}
fn cancelled() -> anyhow::Error {
    Fault::new(
        "NG900",
        "Download cancelled; verified active assets and partial bytes are preserved",
    )
    .into()
}
fn asset_valid(a: &Asset) -> bool {
    !a.file.is_empty()
        && a.file.len() <= 300
        && !a.file.chars().any(char::is_control)
        && (1..=100_000_000_000).contains(&a.bytes)
        && a.sha256.len() == 64
        && a.sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn validate_spec(spec: &TransferSpec) -> Result<()> {
    models::validate_download_url(&spec.url)?;
    let parsed = reqwest::Url::parse(&spec.url)?;
    anyhow::ensure!(
        asset_valid(&spec.asset)
            && spec.url.len() <= 2000
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && spec.target.is_absolute()
            && spec.target.to_string_lossy().len() <= 4096
            && !spec
                .target
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json")),
        fault(
            "Download job must contain a stable publisher URL, bounded asset and non-registration destination"
        )
    );
    no_links(&spec.target)?;
    Ok(())
}
fn job_id(spec: &TransferSpec) -> Result<String> {
    fingerprint(&(1, &spec.url, &spec.target, &spec.asset))
}
impl Manager {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_environment(
            root,
            Arc::new(SystemEnvironment {
                start: Instant::now(),
                cached: Arc::new(Mutex::new(NetworkCache {
                    observed: None,
                    cost: NetworkCost::Unknown,
                    probing: false,
                })),
            }),
        )
    }
    pub fn open_with_environment(root: &Path, environment: Arc<dyn Environment>) -> Result<Self> {
        no_links(root)?;
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let lockpath = root.join("downloads.lock");
        no_links(&lockpath)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lockpath)?;
        lock.try_lock()
            .map_err(|_| fault("Download manager is already open in another process"))?;
        let path = root.join("jobs.json");
        no_links(&path)?;
        let mut store: Store = if path.exists() {
            read_json(&path, STORE_BYTES as u64)?
        } else {
            Store {
                schema: 1,
                policy: Policy::default(),
                jobs: vec![],
            }
        };
        anyhow::ensure!(
            store.schema == 1 && store.jobs.len() <= MAX_JOBS,
            fault("Invalid download manager store")
        );
        store.policy.validate()?;
        let mut ids = BTreeSet::new();
        for job in &mut store.jobs {
            validate_spec(&job.transfer)?;
            anyhow::ensure!(
                job.id == job_id(&job.transfer)?
                    && ids.insert(job.id.clone())
                    && job.received_bytes <= job.transfer.asset.bytes
                    && job
                        .error_code
                        .as_ref()
                        .is_none_or(|s| ["NG650", "NG900"].contains(&s.as_str())),
                fault("Invalid persisted download job")
            );
            if !matches!(
                job.status,
                Status::Complete | Status::Cancelled | Status::Failed
            ) {
                job.status = Status::Queued;
                let partial = job.transfer.target.with_file_name(format!(
                    "{}.part",
                    job.transfer
                        .target
                        .file_name()
                        .ok_or_else(|| fault("Invalid download target filename"))?
                        .to_string_lossy()
                ));
                no_links(&partial)?;
                job.received_bytes = match fs::metadata(&partial) {
                    Ok(metadata) => {
                        anyhow::ensure!(
                            metadata.is_file(),
                            fault("Partial download is not a regular file")
                        );
                        metadata.len().min(job.transfer.asset.bytes)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(error) => return Err(error.into()),
                };
            }
            if job.user_paused && job.status != Status::Cancelled {
                job.status = Status::Paused;
            }
        }
        anyhow::ensure!(
            serde_json::to_vec_pretty(&store)?.len() <= STORE_BYTES,
            fault("Download store exceeds its metadata budget")
        );
        atomic_json(&path, &store)?;
        Ok(Self(Arc::new(Inner {
            root,
            state: Mutex::new(State {
                store,
                leases: BTreeMap::new(),
                active: None,
                budget: ReceiveBudget::default(),
                last_save: Instant::now(),
            }),
            environment,
            _lock: lock,
        })))
    }
    fn save(&self, state: &mut State) -> Result<()> {
        anyhow::ensure!(
            serde_json::to_vec_pretty(&state.store)?.len() <= STORE_BYTES,
            fault("Download store exceeds its metadata budget")
        );
        no_links(&self.0.root.join("jobs.json"))?;
        atomic_json(&self.0.root.join("jobs.json"), &state.store)?;
        state.last_save = Instant::now();
        Ok(())
    }
    pub fn policy(&self) -> Result<Policy> {
        Ok(self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?
            .store
            .policy
            .clone())
    }
    pub fn set_policy(&self, policy: Policy) -> Result<()> {
        policy.validate()?;
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        let previous = state.store.policy.clone();
        let previous_budget = state.budget.clone();
        state.store.policy = policy;
        state.budget = ReceiveBudget::default();
        if let Err(error) = self.save(&mut state) {
            state.store.policy = previous;
            state.budget = previous_budget;
            return Err(error);
        }
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<Job>> {
        Ok(self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?
            .store
            .jobs
            .clone())
    }
    pub fn enqueue(&self, mut spec: TransferSpec) -> Result<Job> {
        spec.target = std::path::absolute(&spec.target)?;
        validate_spec(&spec)?;
        let parent = spec
            .target
            .parent()
            .ok_or_else(|| fault("Choose a download filename"))?;
        fs::create_dir_all(parent)?;
        no_links(parent)?;
        spec.target = parent.canonicalize()?.join(
            spec.target
                .file_name()
                .ok_or_else(|| fault("Choose a download filename"))?,
        );
        let id = job_id(&spec)?;
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        if let Some(index) = state.store.jobs.iter().position(|j| j.id == id) {
            if state.store.jobs[index].status == Status::Complete
                && !state.store.jobs[index].user_paused
            {
                let previous = state.store.jobs[index].clone();
                state.store.jobs[index].status = Status::Queued;
                state.store.jobs[index].received_bytes = 0;
                state.store.jobs[index].error_code = None;
                if let Err(error) = self.save(&mut state) {
                    state.store.jobs[index] = previous;
                    return Err(error);
                }
            }
            return Ok(state.store.jobs[index].clone());
        }
        anyhow::ensure!(
            state.store.jobs.len() < MAX_JOBS
                && !state
                    .store
                    .jobs
                    .iter()
                    .any(|j| j.transfer.target == spec.target),
            fault("Download manager is full or destination belongs to another job")
        );
        let job = Job {
            id,
            transfer: spec,
            status: Status::Queued,
            user_paused: false,
            override_policy: JobOverride::default(),
            received_bytes: 0,
            error_code: None,
        };
        state.store.jobs.push(job.clone());
        if let Err(error) = self.save(&mut state) {
            state.store.jobs.pop();
            return Err(error);
        }
        Ok(job)
    }
    pub fn pause(&self, id: &str) -> Result<()> {
        self.edit(id, |job| {
            job.user_paused = true;
            if job.status != Status::Cancelled {
                job.status = Status::Paused;
            }
        })
    }
    pub fn resume(&self, id: &str) -> Result<()> {
        self.edit(id, |job| {
            job.user_paused = false;
            job.status = Status::Queued;
            job.error_code = None;
        })
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        self.edit(id, |job| {
            job.status = Status::Cancelled;
            job.error_code = Some("NG900".into());
        })?;
        let state = self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        if let Some(flag) = state.leases.get(id) {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
    /// Remove a completed/cancelled queue record only; never delete asset bytes.
    pub fn forget(&self, id: &str) -> Result<()> {
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        anyhow::ensure!(
            !state.leases.contains_key(id),
            fault("Stop this download before forgetting its record")
        );
        let index = state
            .store
            .jobs
            .iter()
            .position(|j| j.id == id)
            .ok_or_else(|| fault("Unknown download job"))?;
        let previous = state.store.jobs.remove(index);
        if let Err(error) = self.save(&mut state) {
            state.store.jobs.insert(index, previous);
            return Err(error);
        }
        Ok(())
    }
    pub fn set_override(&self, id: &str, over: JobOverride) -> Result<()> {
        self.edit(id, |job| job.override_policy = over)
    }
    fn edit(&self, id: &str, change: impl FnOnce(&mut Job)) -> Result<()> {
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        let job = state
            .store
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or_else(|| fault("Unknown download job"))?;
        let previous = job.clone();
        change(job);
        if let Err(error) = self.save(&mut state) {
            if let Some(job) = state.store.jobs.iter_mut().find(|j| j.id == id) {
                *job = previous;
            }
            return Err(error);
        }
        Ok(())
    }
    pub(crate) fn begin(&self, url: &str, path: &Path, asset: &Asset) -> Result<Transfer> {
        let job = self.enqueue(TransferSpec {
            url: url.into(),
            target: path.into(),
            asset: asset.clone(),
        })?;
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        anyhow::ensure!(
            !state.leases.contains_key(&job.id),
            fault("This asset already has an active download")
        );
        if job.status == Status::Cancelled {
            return Err(cancelled());
        }
        let flag = Arc::new(AtomicBool::new(false));
        state.leases.insert(job.id.clone(), flag.clone());
        Ok(Transfer {
            manager: self.clone(),
            id: job.id,
            cancelled: flag,
            finished: false,
        })
    }
    pub fn run_pending(&self, c: &Control, log: &Activity) -> Result<RunSummary> {
        let mut summary = RunSummary::default();
        let jobs = self.list()?;
        let control = Control {
            downloads: Some(self.clone()),
            ..c.clone()
        };
        for job in jobs.into_iter().filter(|j| {
            !j.user_paused
                && !matches!(
                    j.status,
                    Status::Complete | Status::Cancelled | Status::Failed
                )
        }) {
            control.check()?;
            match models::download(
                &job.transfer.url,
                &job.transfer.target,
                &job.transfer.asset,
                &control,
                log,
            ) {
                Ok(()) => summary.completed += 1,
                Err(e) => {
                    if e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900") {
                        return Err(e);
                    }
                    summary.failed += 1;
                }
            }
        }
        Ok(summary)
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct RunSummary {
    pub completed: usize,
    pub failed: usize,
}
impl Transfer {
    pub(crate) fn rate_wait_ms(&self) -> Result<u64> {
        let state = self
            .manager
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        Ok(state
            .budget
            .wait_ms(self.manager.0.environment.monotonic_ms()))
    }
    pub(crate) fn suspend(&self) -> Result<()> {
        self.check_cancelled()?;
        let mut state = self
            .manager
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        if state.active.as_ref() == Some(&self.id) {
            state.active = None;
        }
        if let Some(job) = state.store.jobs.iter_mut().find(|j| j.id == self.id)
            && job.status != Status::Paused
        {
            if job.status == Status::Cancelled {
                return Err(cancelled());
            }
            job.status = Status::Paused;
            self.manager.save(&mut state)?;
        }
        Ok(())
    }
    pub(crate) fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        Ok(())
    }
    pub(crate) fn gate(&self) -> Result<Gate> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let now = self.manager.0.environment.utc_now();
        let millis = self.manager.0.environment.monotonic_ms();
        let policy = self.manager.policy()?;
        let cost =
            if policy.metered == NetworkPolicy::Allow && policy.unknown == NetworkPolicy::Allow {
                NetworkCost::Unknown
            } else {
                self.manager.0.environment.network_cost()
            };
        let mut state = self
            .manager
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        let index = state
            .store
            .jobs
            .iter()
            .position(|j| j.id == self.id)
            .ok_or_else(|| fault("Unknown download job"))?;
        let job = &state.store.jobs[index];
        if job.status == Status::Cancelled {
            return Err(cancelled());
        }
        let policy_wait = policy_gate(
            &state.store.policy,
            &job.override_policy,
            job.user_paused,
            now,
            cost,
        )?;
        let waiting = if let Gate::Wait(status) = policy_wait {
            Some(status)
        } else if state.budget.wait_ms(millis) > 0 {
            Some(Status::WaitingRate)
        } else if state.active.as_ref().is_some_and(|id| id != &self.id) {
            Some(Status::WaitingTurn)
        } else {
            None
        };
        let status = waiting.unwrap_or(Status::Downloading);
        if waiting.is_some() && state.active.as_ref() == Some(&self.id) {
            state.active = None;
        }
        if waiting.is_none() {
            state.active = Some(self.id.clone());
        }
        if state.store.jobs[index].status != status {
            state.store.jobs[index].status = status;
            if matches!(
                status,
                Status::Paused | Status::WaitingWindow | Status::WaitingNetwork
            ) || state.last_save.elapsed() >= Duration::from_secs(1)
            {
                self.manager.save(&mut state)?;
            }
        }
        Ok(waiting.map_or(Gate::Ready, Gate::Wait))
    }
    pub(crate) fn received(&self, bytes: usize, total: u64) -> Result<()> {
        anyhow::ensure!(
            bytes <= MAX_RECEIVE_CHUNK,
            fault("Network chunk exceeds the bounded download receive budget")
        );
        let mut state = self
            .manager
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        let now = self.manager.0.environment.monotonic_ms();
        let rate = state.store.policy.bytes_per_second;
        state.budget.charge(bytes, now, rate)?;
        let job = state
            .store
            .jobs
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| fault("Unknown download job"))?;
        anyhow::ensure!(
            total <= job.transfer.asset.bytes,
            fault("Download progress exceeds pinned size")
        );
        job.received_bytes = total;
        if state.last_save.elapsed() >= Duration::from_secs(1) {
            self.manager.save(&mut state)?;
        }
        Ok(())
    }
    pub(crate) fn finish(&mut self, result: &Result<()>) -> Result<()> {
        let mut state = self
            .manager
            .0
            .state
            .lock()
            .map_err(|_| fault("Download manager lock failed"))?;
        let job = state
            .store
            .jobs
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| fault("Unknown download job"))?;
        if result.is_ok() {
            job.status = Status::Complete;
            job.received_bytes = job.transfer.asset.bytes;
            job.error_code = None;
        } else if self.cancelled.load(Ordering::Relaxed) && job.status == Status::Queued {
            // A user explicitly resumed after cancelling this older lease.
            job.error_code = None;
        } else if self.cancelled.load(Ordering::Relaxed)
            || job.status == Status::Cancelled
            || result
                .as_ref()
                .err()
                .and_then(|e| e.downcast_ref::<Fault>())
                .is_some_and(|f| f.code == "NG900")
        {
            job.status = Status::Cancelled;
            job.error_code = Some("NG900".into());
        } else {
            job.status = Status::Failed;
            job.error_code = Some(
                if result
                    .as_ref()
                    .err()
                    .and_then(|e| e.downcast_ref::<Fault>())
                    .is_some_and(|f| f.code == "NG900")
                {
                    "NG900"
                } else {
                    "NG650"
                }
                .into(),
            );
        }
        self.manager.save(&mut state)?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for Transfer {
    fn drop(&mut self) {
        if let Ok(mut state) = self.manager.0.state.lock() {
            state.leases.remove(&self.id);
            if state.active.as_ref() == Some(&self.id) {
                state.active = None;
            }
            if !self.finished {
                if let Some(job) = state.store.jobs.iter_mut().find(|j| j.id == self.id)
                    && !matches!(job.status, Status::Paused | Status::Cancelled)
                {
                    job.status = Status::Queued;
                }
                let _ = self.manager.save(&mut state);
            }
        }
    }
}
