//! Lazy persistent download controls, separate from recognition settings.
use super::*;
use crate::downloads::{self, Manager, NetworkPolicy, Policy, Status, UtcWindow};

#[derive(Default)]
pub(super) struct Downloads {
    pub(super) manager: Option<Manager>,
    root: Option<PathBuf>,
    open: bool,
    attempted: bool,
    observed_models_dir: Option<PathBuf>,
    page: usize,
    policy: Policy,
    error: String,
    notice: String,
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Queued => "Queued",
        Status::Downloading => "Downloading",
        Status::Paused => "Paused by you",
        Status::WaitingWindow => "Waiting for UTC window",
        Status::WaitingNetwork => "Waiting for network policy",
        Status::WaitingRate => "Waiting for bandwidth budget",
        Status::WaitingTurn => "Waiting for another download",
        Status::Cancelled => "Cancelled · partial bytes kept",
        Status::Complete => "Verified download complete",
        Status::Failed => "Failed · partial bytes kept",
    }
}
fn minute_text(value: f64) -> String {
    let minute = value.round().clamp(0., 1439.) as u16;
    format!("{:02}:{:02}", minute / 60, minute % 60)
}
fn parse_minute(value: &str) -> Option<f64> {
    let (hours, minutes) = value.trim().split_once(':')?;
    let hour = hours.parse::<u16>().ok()?;
    let minute = minutes.parse::<u16>().ok()?;
    (hour < 24 && minute < 60).then_some(f64::from(hour * 60 + minute))
}
fn policy_ui(ui: &mut egui::Ui, policy: &mut Policy) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Maximum receive rate");
        ui.add(
            egui::DragValue::new(&mut policy.bytes_per_second)
                .range(0..=10_000_000_000u64)
                .speed(1024.)
                .suffix(" B/s"),
        )
        .accessible_name("Download bandwidth in bytes per second; zero is unlimited");
        ui.small(if policy.bytes_per_second == 0 {
            "Unlimited".into()
        } else {
            format!("≈ {:.2} MiB/s", policy.bytes_per_second as f64 / 1_048_576.)
        });
    });
    ui.horizontal_wrapped(|ui| {
        let mut metered = policy.metered == NetworkPolicy::Pause;
        let mut unknown = policy.unknown == NetworkPolicy::Pause;
        if ui
            .checkbox(&mut metered, "Pause on metered networks")
            .changed()
        {
            policy.metered = if metered {
                NetworkPolicy::Pause
            } else {
                NetworkPolicy::Allow
            };
        }
        if ui
            .checkbox(&mut unknown, "Pause when network cost is unknown")
            .changed()
        {
            policy.unknown = if unknown {
                NetworkPolicy::Pause
            } else {
                NetworkPolicy::Allow
            };
        }
    });
    ui.small("Network cost can be unavailable. Unknown networks are allowed by default; select Pause to require a known connection or a per-job override.");
    ui.label("Allowed UTC download windows");
    ui.small("No windows allows any time. An end time earlier than the start crosses midnight, anchored to the selected starting day. Changes take effect after Save policy.");
    let mut remove = None;
    for (index, window) in policy.utc_windows.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (day, name) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                        .iter()
                        .enumerate()
                    {
                        let mut enabled = window.weekdays.contains(&(day as u8));
                        if ui.checkbox(&mut enabled, *name).changed() {
                            window.weekdays.retain(|value| *value != day as u8);
                            if enabled {
                                window.weekdays.push(day as u8);
                                window.weekdays.sort_unstable();
                            }
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Start UTC");
                    ui.add(
                        egui::DragValue::new(&mut window.start_minute)
                            .range(0..=1439u16)
                            .custom_formatter(|value, _| minute_text(value))
                            .custom_parser(parse_minute),
                    )
                    .accessible_name(format!(
                        "Window {} start time UTC, hours and minutes",
                        index + 1
                    ));
                    ui.label("End UTC");
                    ui.add(
                        egui::DragValue::new(&mut window.end_minute)
                            .range(0..=1439u16)
                            .custom_formatter(|value, _| minute_text(value))
                            .custom_parser(parse_minute),
                    )
                    .accessible_name(format!(
                        "Window {} end time UTC, hours and minutes",
                        index + 1
                    ));
                    if ui.small_button("Remove window").clicked() {
                        remove = Some(index);
                    }
                });
            });
        });
    }
    if let Some(index) = remove {
        policy.utc_windows.remove(index);
    }
    if ui
        .add_enabled(
            policy.utc_windows.len() < 14,
            egui::Button::new("Add UTC window"),
        )
        .clicked()
    {
        policy.utc_windows.push(UtcWindow {
            weekdays: (0..7).collect(),
            start_minute: 0,
            end_minute: 360,
        });
    }
}
impl Desktop {
    /// New per-operation cancellation flags with the same already-open manager.
    /// Local work never opens a download store or depends on its writability.
    pub(super) fn fresh_operation_control(&self) -> Control {
        Control {
            downloads: self.downloads.manager.clone(),
            ..Control::default()
        }
    }
    /// Call only for an explicit network mutation or the Downloads view.
    /// A policy-store error blocks that network action, never local recognition.
    pub(super) fn prepare_download_control(&mut self) -> Result<()> {
        let requested = std::path::absolute(self.state.settings.models_dir.join(".downloads"))?;
        crate::watch::no_links(&requested)?;
        let resolved = requested
            .canonicalize()
            .unwrap_or_else(|_| requested.clone());
        if self.downloads.root.as_ref() == Some(&resolved) && self.downloads.manager.is_some() {
            self.control.downloads = self.downloads.manager.clone();
            return Ok(());
        }
        anyhow::ensure!(
            self.worker.is_none() || self.downloads.manager.is_none(),
            Fault::new(
                "NG650",
                "Wait for the background task before changing the download store"
            )
        );
        let manager = Manager::open(&requested)?;
        let policy = manager.policy()?;
        self.downloads.root = Some(requested.canonicalize()?);
        self.downloads.manager = Some(manager.clone());
        self.downloads.policy = policy;
        self.downloads.page = 0;
        self.downloads.error.clear();
        self.control.downloads = Some(manager);
        Ok(())
    }
    pub(super) fn downloads_open(&mut self) {
        if !self.downloads.open {
            self.downloads.attempted = false;
        }
        self.downloads.open = true;
    }
    pub(super) fn downloads_ui(&mut self, ctx: &egui::Context) {
        if !self.downloads.open {
            return;
        }
        if self.downloads.observed_models_dir.as_ref() != Some(&self.state.settings.models_dir) {
            self.downloads.observed_models_dir = Some(self.state.settings.models_dir.clone());
            self.downloads.attempted = false;
        }
        if !self.downloads.attempted {
            self.downloads.attempted = true;
            if let Err(error) = self.prepare_download_control() {
                self.downloads.error = error.to_string();
            }
        }
        let mut open = self.downloads.open;
        let manager = self.downloads.manager.clone();
        let bounds = ctx.viewport_rect().shrink(8.);
        egui::Window::new("Downloads").id(egui::Id::unique("notegen-downloads-window"))
            .open(&mut open).default_size([790., 610.]).max_height(bounds.height())
            .constrain_to(bounds).vscroll(true).show(ctx, |ui| {
                ui.label("Setup, language packs, repairs and updates share this persisted queue and policy. Downloads start only after an explicit action.");
                if let Some(root) = &self.downloads.root { ui.small(format!("Store: {}", root.display())); }
                if !self.downloads.error.is_empty() {
                    Self::accessible_status(ui, "downloads-error", &self.downloads.error, true);
                    if ui.button("Retry opening download store").clicked() { self.downloads.attempted = false; }
                }
                let Some(manager) = manager.as_ref() else {
                    ui.label("Local recognition and project tools remain available. Choose a writable models folder to manage downloads.");
                    return;
                };
                egui::CollapsingHeader::new("Bandwidth, UTC windows & network policy").default_open(true).show(ui, |ui| {
                    policy_ui(ui, &mut self.downloads.policy);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Save download policy").clicked() {
                            match manager.set_policy(self.downloads.policy.clone()) {
                                Ok(()) => { self.downloads.error.clear(); self.downloads.notice = "Download policy saved. Active downloads observe changes automatically.".into(); }
                                Err(error) => self.downloads.error = error.to_string(),
                            }
                        }
                        if ui.button("Reload saved policy").clicked() {
                            match manager.policy() { Ok(policy) => self.downloads.policy = policy, Err(error) => self.downloads.error = error.to_string() }
                        }
                    });
                });
                ui.separator();
                if ui.add_enabled(self.worker.is_none(), egui::Button::new("Continue queued asset downloads")).clicked() {
                    let manager = manager.clone();
                    self.studio_task(ctx, move |c, log| {
                        let report = manager.run_pending(&c, &log)?;
                        Ok(studio::StudioMessage::Done(format!("Downloaded and verified {} assets; {} failed. Rerun the original setup/update action to activate any pending registrations.", report.completed, report.failed)))
                    });
                }
                ui.small("Continuing the queue verifies asset bytes. It does not activate a reader, runtime or app update. Rerun the original install/update action to finish registration.");
                if !self.downloads.notice.is_empty() { Self::accessible_status(ui, "downloads-notice", &self.downloads.notice, false); }
                match manager.list() {
                    Ok(jobs) => {
                        if jobs.is_empty() { ui.label("No asset downloads have been queued."); }
                        let pages = jobs.len().div_ceil(20).max(1);
                        self.downloads.page = self.downloads.page.min(pages - 1);
                        ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(self.downloads.page > 0, egui::Button::new("Previous jobs")).clicked() { self.downloads.page -= 1; }
                            ui.label(format!("Page {} of {} · {} jobs", self.downloads.page + 1, pages, jobs.len()));
                            if ui.add_enabled(self.downloads.page + 1 < pages, egui::Button::new("Next jobs")).clicked() { self.downloads.page += 1; }
                        });
                        for job in jobs.into_iter().skip(self.downloads.page * 20).take(20) {
                            ui.push_id(&job.id, |ui| {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.strong(&job.transfer.asset.file);
                                    ui.label(status_label(job.status));
                                    let total = job.transfer.asset.bytes;
                                    ui.add(egui::ProgressBar::new((job.received_bytes as f64 / total.max(1) as f64).clamp(0., 1.) as f32)
                                        .text(format!("{:.1} / {:.1} MiB", job.received_bytes as f64 / 1_048_576., total as f64 / 1_048_576.)));
                                    ui.horizontal_wrapped(|ui| {
                                        let terminal = matches!(job.status, Status::Complete | Status::Cancelled | Status::Failed);
                                        let mut result = None;
                                        if ui.add_enabled(!terminal && !job.user_paused, egui::Button::new("Pause download")).clicked() { result = Some(manager.pause(&job.id)); }
                                        let resumable = job.status != Status::Complete && if matches!(job.status, Status::Cancelled | Status::Failed) { self.worker.is_none() } else { job.user_paused };
                                        if ui.add_enabled(resumable, egui::Button::new("Resume / retry download")).clicked() { result = Some(manager.resume(&job.id)); }
                                        if ui.add_enabled(!terminal, egui::Button::new("Cancel download")).clicked() { result = Some(manager.cancel(&job.id)); }
                                        if ui.add_enabled(terminal, egui::Button::new("Forget queue record")).on_hover_text("Keeps complete and partial asset files.").clicked() { result = Some(manager.forget(&job.id)); }
                                        if let Some(result) = result {
                                            match result { Ok(()) => self.downloads.error.clear(), Err(error) => self.downloads.error = error.to_string() }
                                        }
                                    });
                                    egui::CollapsingHeader::new("This job: policy overrides & destination").show(ui, |ui| {
                                        ui.small(format!("Destination: {}", job.transfer.target.display()));
                                        ui.small(format!("Stable publisher URL: {}", job.transfer.url));
                                        let mut over = job.override_policy.clone();
                                        let a = ui.checkbox(&mut over.allow_metered, "Allow this job on a metered network").changed();
                                        let b = ui.checkbox(&mut over.allow_unknown, "Allow this job when network cost is unknown").changed();
                                        let d = ui.checkbox(&mut over.ignore_window, "Allow this job outside UTC windows").changed();
                                        if (a || b || d) && let Err(error) = manager.set_override(&job.id, over) { self.downloads.error = error.to_string(); }
                                        ui.small("Overrides persist for this asset job. The global bandwidth limit still applies.");
                                    });
                                });
                            });
                        }
                    }
                    Err(error) => self.downloads.error = error.to_string(),
                }
                egui::CollapsingHeader::new("Download limits & privacy").show(ui, |ui| { ui.small(downloads::LIMITS); });
            });
        self.downloads.open = open;
        if open {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_clock_editor_rejects_invalid_or_ambiguous_times() {
        assert_eq!(parse_minute("23:59"), Some(1439.));
        assert_eq!(parse_minute("00:00"), Some(0.));
        for invalid in ["24:00", "12:60", "-1:00", "12", "12:30:00", "NaN:00"] {
            assert_eq!(parse_minute(invalid), None);
        }
        assert_eq!(minute_text(1439.), "23:59");
    }
    #[test]
    fn startup_and_local_controls_never_create_a_download_store() {
        let temp = tempfile::tempdir().unwrap();
        let state = Workspace {
            settings: Settings {
                models_dir: temp.path().join("uncreated-models"),
                ..Default::default()
            },
            ..Default::default()
        };
        let app = Desktop::new(state);
        assert!(app.fresh_operation_control().downloads.is_none());
        assert!(!temp.path().join("uncreated-models").exists());
    }
    #[test]
    fn new_operation_flags_reuse_the_same_live_persisted_policy_manager() {
        let temp = tempfile::tempdir().unwrap();
        let state = Workspace {
            settings: Settings {
                models_dir: temp.path().to_owned(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut app = Desktop::new(state);
        app.prepare_download_control().unwrap();
        let first = app.fresh_operation_control();
        first.cancelled.store(true, Ordering::Relaxed);
        let next = app.fresh_operation_control();
        assert!(!next.cancelled.load(Ordering::Relaxed));
        let policy = Policy {
            bytes_per_second: 4096,
            ..Default::default()
        };
        first
            .downloads
            .as_ref()
            .unwrap()
            .set_policy(policy)
            .unwrap();
        assert_eq!(
            next.downloads
                .as_ref()
                .unwrap()
                .policy()
                .unwrap()
                .bytes_per_second,
            4096
        );
        app.prepare_download_control().unwrap(); // does not open a second locked manager
    }
    #[test]
    fn unavailable_download_store_does_not_disable_local_operation_controls() {
        let temp = tempfile::tempdir().unwrap();
        let models = temp.path().join("models-is-a-file");
        fs::write(&models, b"existing local document").unwrap();
        let state = Workspace {
            settings: Settings {
                models_dir: models.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut app = Desktop::new(state);
        assert!(app.prepare_download_control().is_err());
        let local = app.fresh_operation_control();
        local.check().unwrap();
        assert!(local.downloads.is_none());
        assert_eq!(fs::read(models).unwrap(), b"existing local document");
    }
}
