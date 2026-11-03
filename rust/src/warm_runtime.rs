//! Explicit, bounded owned-reader reuse. Confidential sessions and hardware trials
//! use the ordinary cold APIs. Logical reset is not forensic RAM zeroization.
use crate::{
    hardware,
    planning::{Model, Plan, server_args},
    runtime::{ManagedShutdown, Server},
    storage::{Activity, Control, fingerprint, hash_file},
    types::*,
};
use serde::Serialize;
use std::{
    fs,
    ops::Deref,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const MAX_TTL: Duration = Duration::from_secs(60);
#[derive(Clone, Debug, Serialize)]
pub struct WarmStatus {
    pub cached_pid: Option<u32>,
    pub busy: bool,
    pub remaining_seconds: u64,
    pub note: String,
}
pub(crate) struct ReaderLease {
    server: Server,
    directory: Option<tempfile::TempDir>,
    key: Option<String>,
}
impl ReaderLease {
    pub(crate) fn cold(server: Server) -> Self {
        Self {
            server,
            directory: None,
            key: None,
        }
    }
}
impl Deref for ReaderLease {
    type Target = Server;
    fn deref(&self) -> &Server {
        &self.server
    }
}
struct Cached {
    reader: ReaderLease,
    expires: Instant,
}
#[derive(Default)]
struct State {
    cached: Option<Cached>,
    active: Option<ManagedShutdown>,
    active_cancel: Option<Arc<AtomicBool>>,
    generation: u64,
    busy: bool,
    closed: bool,
    releasing: usize,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
pub struct WarmPool {
    shared: Arc<Shared>,
    ttl: Duration,
    timer: Option<thread::JoinHandle<()>>,
}
fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG200", message).into()
}
impl WarmPool {
    pub fn new(ttl: Duration) -> Result<Self> {
        anyhow::ensure!(
            !ttl.is_zero() && ttl <= MAX_TTL,
            fault("Warm reader lifetime must be greater than zero and at most 60 seconds")
        );
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let timer_shared = shared.clone();
        let timer = thread::Builder::new()
            .name("notegen-warm-expiry".into())
            .spawn(move || {
                let mut state = timer_shared
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                loop {
                    if state.closed {
                        break;
                    }
                    if state
                        .cached
                        .as_ref()
                        .is_some_and(|entry| entry.expires <= Instant::now())
                    {
                        let expired = state.cached.take();
                        state.releasing += 1;
                        drop(state);
                        drop(expired); // kill and wait outside the pool lock
                        state = timer_shared
                            .state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        state.releasing -= 1;
                        timer_shared.changed.notify_all();
                        continue;
                    }
                    let wait = state.cached.as_ref().map_or(MAX_TTL, |entry| {
                        entry.expires.saturating_duration_since(Instant::now())
                    });
                    state = timer_shared
                        .changed
                        .wait_timeout(state, wait)
                        .unwrap_or_else(|error| error.into_inner())
                        .0;
                }
            })?;
        Ok(Self {
            shared,
            ttl,
            timer: Some(timer),
        })
    }
    /// Release also invalidates an in-flight lease, which can never return to cache.
    pub fn release(&self) {
        let (cached, active, cancellation) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.generation = state.generation.wrapping_add(1);
            state.releasing += 1;
            (
                state.cached.take(),
                state.active.take(),
                state.active_cancel.take(),
            )
        };
        if let Some(cancellation) = cancellation {
            cancellation.store(true, Ordering::Release);
        }
        if let Some(active) = active {
            active.stop();
        }
        drop(cached);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.releasing -= 1;
        self.shared.changed.notify_all();
        while state.releasing > 0 {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
    pub fn shutdown(&self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
        self.release();
    }
    pub fn status(&self) -> WarmStatus {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        WarmStatus {
            cached_pid: state.cached.as_ref().and_then(|entry| entry.reader.managed_pid()),
            busy: state.busy || state.releasing > 0,
            remaining_seconds: state.cached.as_ref().map_or(0, |entry| entry.expires.saturating_duration_since(Instant::now()).as_secs()),
            note: "At most one owned reader; explicit release cancels an active pool job, and expiry stops an idle reader. Context is reset between jobs; native RAM, including image working buffers, may retain bytes until release. Confidential sessions and hardware trials always use fresh processes.".into(),
        }
    }
    pub(crate) fn begin(&self, c: &Control) -> Result<WarmJob<'_>> {
        loop {
            c.check()?;
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| fault("Warm reader lease failed"))?;
            anyhow::ensure!(
                !state.closed,
                Fault::new("NG900", "Warm reader pool was shut down")
            );
            if !state.busy && state.releasing == 0 {
                state.busy = true;
                state.active_cancel = Some(c.cancelled.clone());
                let candidate = state
                    .cached
                    .take()
                    .filter(|entry| entry.expires > Instant::now())
                    .map(|entry| entry.reader);
                state.active = candidate
                    .as_ref()
                    .and_then(|reader| reader.shutdown_handle());
                return Ok(WarmJob {
                    pool: self,
                    generation: state.generation,
                    candidate,
                    tainted: false,
                    used: false,
                });
            }
            let (state, _) = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .map_err(|_| fault("Warm reader lease failed"))?;
            drop(state); // pause/cancel checks must never retain the pool lock
        }
    }
    /// A complete explicit job, useful for integrations that already own their reader loop.
    #[allow(clippy::too_many_arguments)]
    pub fn with_reader<T>(
        &self,
        model: &Model,
        plan: &Plan,
        s: &Settings,
        c: &Control,
        log: &Activity,
        root: &Path,
        job: impl FnOnce(&Server) -> Result<T>,
    ) -> Result<T> {
        let mut lease = self.begin(c)?;
        let reader = lease.reader(model, plan, s, c, log, root)?;
        let result = job(&reader)?;
        lease.finish_reader(reader, c, log);
        c.check()?;
        lease.commit()?;
        Ok(result)
    }
}
impl Drop for WarmPool {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}
pub(crate) struct WarmJob<'a> {
    pool: &'a WarmPool,
    generation: u64,
    candidate: Option<ReaderLease>,
    tainted: bool,
    used: bool,
}
impl WarmJob<'_> {
    fn current(&self) -> Result<()> {
        let state = self
            .pool
            .shared
            .state
            .lock()
            .map_err(|_| fault("Warm reader lease failed"))?;
        anyhow::ensure!(
            !state.closed && state.generation == self.generation,
            Fault::new("NG900", "Warm reader lease was released")
        );
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reader(
        &mut self,
        model: &Model,
        plan: &Plan,
        s: &Settings,
        c: &Control,
        log: &Activity,
        root: &Path,
    ) -> Result<ReaderLease> {
        self.current()?;
        c.check()?;
        self.used = true;
        let key = compatibility_key(model, plan, s, c);
        let existing = self.candidate.take();
        let reader = match (existing, key) {
            (Some(reader), Ok(key))
                if reader.key.as_ref() == Some(&key)
                    && reader.healthy()
                    && memory_admitted(s, plan, c) =>
            {
                reader.bind_control(Some(c))?;
                log.emit(
                    "model",
                    format!(
                        "Reusing reset reader {} (PID {})",
                        model.name,
                        reader.managed_pid().unwrap_or_default()
                    ),
                    0,
                    0,
                    0,
                    "",
                );
                reader
            }
            (existing, key) => {
                drop(existing); // never load a second owned reader alongside the old one
                self.current()?;
                c.check()?;
                if !s.external_server && model.endpoint.is_none() {
                    let fresh = crate::planning::build(
                        &hardware::detect_for(s, c),
                        s,
                        std::slice::from_ref(model),
                    )?;
                    anyhow::ensure!(
                        fresh
                            .models
                            .iter()
                            .any(|candidate| candidate.id == model.id),
                        Fault::new("NG300", "Fresh memory admission rejected reader load")
                    );
                }
                match key {
                    Ok(key) if memory_admitted(s, plan, c) => {
                        let directory = tempfile::Builder::new()
                            .prefix("notegen-warm-runtime-")
                            .tempdir()?;
                        let server =
                            Server::start_private(model, plan, s, c, log, directory.path())?;
                        ReaderLease {
                            server,
                            directory: Some(directory),
                            key: Some(key),
                        }
                    }
                    _ => {
                        log.emit("model", "Warm reuse unavailable for this runtime or memory budget; using a fresh reader", 0, 0, 0, "");
                        ReaderLease::cold(Server::start(model, plan, s, c, log, root)?)
                    }
                }
            }
        };
        let mut state = self
            .pool
            .shared
            .state
            .lock()
            .map_err(|_| fault("Warm reader lease failed"))?;
        if state.closed || state.generation != self.generation {
            drop(state);
            drop(reader);
            return Err(Fault::new("NG900", "Warm reader lease was released").into());
        }
        state.active = reader.shutdown_handle();
        Ok(reader)
    }
    pub(crate) fn finish_reader(&mut self, reader: ReaderLease, c: &Control, log: &Activity) {
        if !reader.healthy() {
            self.tainted = true;
        }
        let logs_bounded = reader
            .directory
            .as_ref()
            .is_some_and(|directory| owned_files_bounded(directory.path()));
        if !self.tainted && reader.key.is_some() && logs_bounded && self.current().is_ok() {
            match reader
                .reset_for_reuse(c)
                .and_then(|()| reader.bind_control(None))
            {
                Ok(()) => {
                    self.candidate = Some(reader);
                    return;
                }
                Err(error) => log.emit(
                    "model",
                    format!("Reader reset could not be verified; process released: {error}"),
                    0,
                    0,
                    0,
                    "NG200",
                ),
            }
            self.tainted = true;
        }
        drop(reader);
    }
    pub(crate) fn taint(&mut self) {
        self.tainted = true;
    }
    pub(crate) fn commit(mut self) -> Result<()> {
        self.current()?;
        let mut state = self
            .pool
            .shared
            .state
            .lock()
            .map_err(|_| fault("Warm reader lease failed"))?;
        anyhow::ensure!(
            !state.closed
                && state.generation == self.generation
                && state
                    .active_cancel
                    .as_ref()
                    .is_none_or(|cancel| !cancel.load(Ordering::Acquire)),
            Fault::new("NG900", "Warm reader lease was released")
        );
        if !self.tainted && self.used {
            state.cached = self.candidate.take().map(|reader| Cached {
                reader,
                expires: Instant::now() + self.pool.ttl,
            });
        }
        state.active = None;
        state.active_cancel = None;
        self.pool.shared.changed.notify_all();
        Ok(())
    }
}

impl Drop for WarmJob<'_> {
    fn drop(&mut self) {
        drop(self.candidate.take());
        let mut state = self
            .pool
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active = None;
        state.active_cancel = None;
        state.busy = false;
        self.pool.shared.changed.notify_all();
    }
}
fn memory_admitted(s: &Settings, plan: &Plan, c: &Control) -> bool {
    memory_fits(&hardware::detect_for(s, c), s, plan)
}
fn owned_files_bounded(root: &Path) -> bool {
    fs::read_dir(root).is_ok_and(|mut entries| {
        entries.all(|entry| {
            entry.is_ok_and(|entry| {
                if crate::watch::no_links(&entry.path()).is_err() {
                    return false;
                }
                fs::symlink_metadata(entry.path()).is_ok_and(|meta| {
                    if meta.is_dir() {
                        entry.file_name() == "slot-reset"
                            && !meta.file_type().is_symlink()
                            && fs::read_dir(entry.path())
                                .is_ok_and(|mut files| files.next().is_none())
                    } else {
                        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 4_000_000
                    }
                })
            })
        })
    })
}
fn memory_fits(hw: &hardware::Hardware, s: &Settings, plan: &Plan) -> bool {
    // Deliberately no credit for resident/mapped weights: available RAM can already
    // include reclaimable file pages. Admission is conservative rather than double-counting.
    let budget = |free: u64, reserve: u64, fraction: f32| {
        ((free as f64 * fraction as f64) as u64).min(free.saturating_sub(reserve))
    };
    budget(hw.free_ram_mib, s.reserve_ram_mib, s.ram_fraction) >= 1024
        && (plan.backend == Backend::Cpu
            || hw.gpus.iter().any(|gpu| {
                Some(gpu.index) == plan.gpu_index
                    && budget(gpu.free_mib, s.reserve_vram_mib, s.vram_fraction) >= 512
            }))
}
fn compatibility_key(model: &Model, plan: &Plan, s: &Settings, c: &Control) -> Result<String> {
    anyhow::ensure!(
        !s.external_server && model.endpoint.is_none(),
        fault("External readers cannot enter the warm pool")
    );
    let (success, stdout, stderr) =
        crate::scan::run(&s.server, &["--version".into()], Duration::from_secs(10), c)?;
    anyhow::ensure!(
        success && format!("{stdout}{stderr}").contains("build 12032, commit dde879bc4"),
        fault("Warm reset is verified only for pinned b12032")
    );
    let server = s.server.canonicalize()?;
    let mut files = vec![server.clone()];
    for entry in fs::read_dir(
        server
            .parent()
            .ok_or_else(|| fault("Runtime directory missing"))?,
    )? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        {
            files.push(path);
        }
    }
    anyhow::ensure!(
        files.len() <= 128,
        fault("Runtime has too many dependency files")
    );
    files.sort();
    let mut runtime = vec![];
    for file in files {
        c.check()?;
        runtime.push((file.canonicalize()?, hash_file(&file, c)?));
    }
    let weights = (model.weights.canonicalize()?, hash_file(&model.weights, c)?);
    let projector = (
        model.projector.canonicalize()?,
        hash_file(&model.projector, c)?,
    );
    let mut environment = std::env::vars_os()
        .map(|(key, value)| (key.into_encoded_bytes(), value.into_encoded_bytes()))
        .collect::<Vec<_>>();
    environment.sort();
    fingerprint(&(
        1,
        runtime,
        weights,
        projector,
        &model.id,
        &model.prompt,
        &model.revision,
        server_args(model, plan, s, 0),
        s.require_acceleration,
        environment,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn owned_runtime_allows_only_empty_slot_reset_storage_and_bounded_logs() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("slot-reset")).unwrap();
        fs::write(
            root.path().join("server.log"),
            b"bounded synthetic diagnostic",
        )
        .unwrap();
        assert!(owned_files_bounded(root.path()));
        fs::write(
            root.path().join("slot-reset/kv.bin"),
            b"unexpected saved prompt cache",
        )
        .unwrap();
        assert!(
            !owned_files_bounded(root.path()),
            "saved slot bytes must prevent warm cache admission"
        );
        fs::remove_file(root.path().join("slot-reset/kv.bin")).unwrap();
        assert!(owned_files_bounded(root.path()));
        fs::create_dir(root.path().join("unknown-child")).unwrap();
        assert!(!owned_files_bounded(root.path()));
    }
    #[test]
    fn warm_lifetime_is_explicit_and_bounded_and_shutdown_is_final() {
        assert!(WarmPool::new(Duration::ZERO).is_err());
        assert!(WarmPool::new(Duration::from_secs(61)).is_err());
        let pool = WarmPool::new(Duration::from_secs(1)).unwrap();
        assert!(pool.status().cached_pid.is_none());
        pool.shutdown();
        assert!(pool.begin(&Control::default()).is_err());
    }
    #[test]
    fn release_cancels_active_job_and_cannot_cancel_a_later_job() {
        let pool = WarmPool::new(Duration::from_secs(1)).unwrap();
        let first = Control::default();
        let job = pool.begin(&first).unwrap();
        pool.release();
        assert!(first.cancelled.load(Ordering::Acquire));
        assert_eq!(
            job.commit()
                .unwrap_err()
                .downcast_ref::<Fault>()
                .unwrap()
                .code,
            "NG900"
        );
        let next = Control::default();
        let job = pool.begin(&next).unwrap();
        first.cancelled.store(true, Ordering::Release);
        assert!(!next.cancelled.load(Ordering::Acquire));
        job.commit().unwrap();
        assert!(!pool.status().busy);
    }
    #[test]
    fn jobs_serialize_and_a_waiter_can_cancel_without_entering_the_pool() {
        let pool = WarmPool::new(Duration::from_secs(1)).unwrap();
        let c = Control::default();
        let first = pool.begin(&c).unwrap();
        let waiting = Control::default();
        let cancel = waiting.cancelled.clone();
        thread::scope(|scope| {
            let (entered, results) = mpsc::channel();
            let pool_ref = &pool;
            scope.spawn(move || {
                let result = pool_ref.begin(&waiting);
                entered
                    .send(
                        result.err().and_then(|error| {
                            error.downcast_ref::<Fault>().map(|fault| fault.code)
                        }),
                    )
                    .unwrap();
            });
            assert!(matches!(
                results.recv_timeout(Duration::from_millis(80)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            cancel.store(true, Ordering::Release);
            assert_eq!(
                results.recv_timeout(Duration::from_secs(5)).unwrap(),
                Some("NG900")
            );
            assert!(pool.status().busy);
        });
        drop(first);
        assert!(!pool.status().busy);
    }
    #[test]
    fn external_endpoints_are_rejected_before_any_runtime_or_model_access() {
        let mut model = crate::planning::catalog().remove(0);
        model.endpoint = Some("http://127.0.0.1:1".into());
        let s = Settings::default();
        let plan = crate::planning::build(
            &hardware::Hardware {
                free_ram_mib: 32768,
                ..Default::default()
            },
            &s,
            &[],
        )
        .unwrap();
        let error = compatibility_key(&model, &plan, &s, &Control::default()).unwrap_err();
        assert!(error.to_string().contains("External readers"));
    }
    #[test]
    fn reuse_respects_fresh_ram_fraction_reservations_and_selected_gpu() {
        let mut s = Settings {
            reserve_ram_mib: 1024,
            ram_fraction: 0.5,
            ..Default::default()
        };
        let mut hw = hardware::Hardware {
            free_ram_mib: 8192,
            ..Default::default()
        };
        let mut plan = crate::planning::build(&hw, &s, &[]).unwrap();
        hw.free_ram_mib = 2048;
        assert!(memory_fits(&hw, &s, &plan));
        hw.free_ram_mib = 2047;
        assert!(!memory_fits(&hw, &s, &plan));
        hw.free_ram_mib = 8192;
        s.ram_fraction = 0.1;
        assert!(!memory_fits(&hw, &s, &plan));
        s.ram_fraction = 0.5;
        s.reserve_vram_mib = 400;
        s.vram_fraction = 0.9;
        plan.backend = Backend::Cuda;
        plan.gpu_index = Some(3);
        hw.gpus = vec![
            hardware::Gpu {
                index: 0,
                free_mib: 16000,
                ..Default::default()
            },
            hardware::Gpu {
                index: 3,
                free_mib: 912,
                ..Default::default()
            },
        ];
        assert!(memory_fits(&hw, &s, &plan));
        hw.gpus[1].free_mib = 911;
        assert!(
            !memory_fits(&hw, &s, &plan),
            "Another device's memory cannot admit this reader"
        );
    }
}
