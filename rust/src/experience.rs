//! Plain-language settings, measured tuning, updates and observable AI work.
use super::*;
use crate::{maintenance, settings_help, telemetry, tuning};

#[derive(Default)]
pub(super) struct Experience {
    pub open: bool,
    pub tab: usize,
    pub checking: bool,
    checked_session: bool,
    pub updates: Option<maintenance::Report>,
    pub tuning: Option<tuning::Report>,
    previous_threads: Option<usize>,
    pub error: String,
    search: String,
    focus_settings: bool,
    live: telemetry::Preview,
    texture: Option<egui::TextureHandle>,
}

impl Experience {
    fn clear_preview(&mut self) {
        self.live = telemetry::Preview::clear();
        self.texture = None;
    }
    pub fn preview(&mut self, p: telemetry::Preview, ctx: &egui::Context) {
        let change = self.live.apply(p);
        if change == telemetry::Change::Cleared {
            self.clear_preview();
        } else if change == telemetry::Change::SourceChanged
            || change == telemetry::Change::SourceRetained && self.texture.is_none()
        {
            let path = self
                .live
                .image
                .as_ref()
                .expect("source update has an image");
            self.texture = crate::writers::open_image(path, 40_000_000).ok().map(|im| {
                let limit = ctx.input(|i| i.max_texture_side).clamp(1, 1200) as u32;
                let rgba = im.thumbnail(limit, limit).to_rgba8();
                ctx.load_texture(
                    "live-source",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
        }
    }
}
fn help(ui: &mut egui::Ui, key: &str) {
    let color = if ui.visuals().dark_mode {
        Color32::from_gray(175)
    } else {
        Color32::from_gray(75)
    };
    ui.label(
        RichText::new(settings_help::description(key))
            .size(12.)
            .color(color),
    );
}
impl Desktop {
    pub fn settings_open(&mut self, tab: usize) {
        self.experience.open = true;
        self.experience.tab = tab.min(3);
        self.experience.focus_settings = true;
    }
    /// A stable live region for discrete task results, never streamed reader text.
    /// This updates the accessibility tree without interrupting keyboard focus.
    pub(super) fn accessible_status(
        ui: &mut egui::Ui,
        id: &'static str,
        text: &str,
        is_error: bool,
    ) -> egui::Response {
        let scope = ui.make_persistent_id(id);
        ui.scope_builder(egui::UiBuilder::new().scope_id(scope), |ui| {
            let response = if is_error {
                ui.colored_label(Color32::LIGHT_RED, text)
            } else {
                ui.label(text)
            };
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_role(egui::accesskit::Role::Status);
                node.set_label(text);
                node.set_live(egui::accesskit::Live::Polite);
                node.set_live_atomic();
            });
            response
        })
        .inner
    }
    fn request_update_check(&mut self, ctx: &egui::Context, mode: maintenance::CheckMode) {
        if self.experience.checking {
            return;
        }
        let now = chrono::Utc::now();
        if maintenance::check_decision(&self.state.settings.experience, None, now, mode)
            != maintenance::CheckDecision::Due
        {
            return;
        }
        self.experience.checking = true;
        self.experience.checked_session = true;
        self.experience.error.clear();
        let _ = atomic_json(
            Path::new("user-data/last-update-check.json"),
            &maintenance::CheckStamp {
                checked_at: now,
                channel: self.state.settings.experience.update_channel,
            },
        );
        let s = self.state.settings.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                maintenance::check(&s, &Control::default(), &Activity::default())
            }))
            .map_err(|_| "Publisher check stopped unexpectedly".to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(Message::Maintenance(Box::new(result)));
            ctx.request_repaint();
        });
    }
    pub(super) fn check_updates_when_idle(&mut self, ctx: &egui::Context) {
        if !self.experience.checked_session
            && !self.experience.checking
            && self.clock.elapsed().as_secs() >= 10
            && self.hardware.is_some()
            && self.worker.is_none()
        {
            let now = chrono::Utc::now();
            if maintenance::check_decision(
                &self.state.settings.experience,
                None,
                now,
                maintenance::CheckMode::Automatic,
            ) != maintenance::CheckDecision::Due
            {
                return;
            }
            let path = Path::new("user-data/last-update-check.json");
            let stamp = read_json::<maintenance::CheckStamp>(path, 1000)
                .or_else(|_| {
                    // Older desktops stored a single UTC timestamp for the stable channel.
                    read_json::<chrono::DateTime<chrono::Utc>>(path, 1000).map(|checked_at| {
                        maintenance::CheckStamp {
                            checked_at,
                            channel: UpdateChannel::Stable,
                        }
                    })
                })
                .ok();
            if maintenance::check_decision(
                &self.state.settings.experience,
                stamp.as_ref(),
                now,
                maintenance::CheckMode::Automatic,
            ) == maintenance::CheckDecision::Due
            {
                self.request_update_check(ctx, maintenance::CheckMode::Automatic);
            } else {
                self.experience.checked_session = true;
            }
        }
    }
    pub(super) fn live_preview_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(report) = &self.experience.updates
            && report.count() > 0
            && ui
                .link(format!(
                    "{} updates available · open Settings",
                    report.count()
                ))
                .clicked()
        {
            self.settings_open(1);
        }
        if !self.state.settings.experience.live_preview {
            self.experience.clear_preview();
            return;
        }
        if self.active.is_none() && self.experience.live.image.is_none() {
            return;
        }
        let label = if self.experience.live.reader.is_empty() {
            "Live AI view".into()
        } else {
            format!(
                "Live AI view · page {} · {}",
                self.experience.live.page, self.experience.live.reader
            )
        };
        egui::CollapsingHeader::new(label)
            .id_salt("live-ai-view")
            .default_open(true)
            .show(ui, |ui| {
                if self.experience.live.clear {
                    ui.weak("Live view cleared. It resumes at the next source page or reader.");
                    return;
                }
                ui.small("Source region and generated draft. Drafts may change before review.");
                if self.experience.texture.is_none() {
                    ui.weak("A source page appears here when an AI reader starts.");
                }
                ui.columns(2, |columns| {
                    if let Some(texture) = &self.experience.texture {
                        let fit = (columns[0].available_width() / texture.size_vec2().x)
                            .min(240. / texture.size_vec2().y)
                            .min(1.);
                        let response = columns[0].add(
                            egui::Image::new((texture.id(), texture.size_vec2() * fit)).alt_text(
                                format!("Source page {} for {}", self.experience.live.page, self.experience.live.reader),
                            ),
                        );
                        if let Some(b) = &self.experience.live.region {
                            columns[0].ctx().accesskit_node_builder(response.id, |node| {
                                node.set_description(format!(
                                    "Highlighted region: left {:.0}%, top {:.0}%, right {:.0}%, bottom {:.0}% of source page.",
                                    b.left * 100., b.top * 100., b.right * 100., b.bottom * 100.,
                                ));
                            });
                            let r = response.rect;
                            let region = egui::Rect::from_min_max(
                                egui::pos2(
                                    r.left() + b.left * r.width(),
                                    r.top() + b.top * r.height(),
                                ),
                                egui::pos2(
                                    r.left() + b.right * r.width(),
                                    r.top() + b.bottom * r.height(),
                                ),
                            );
                            columns[0].painter().rect_stroke(
                                region,
                                2.,
                                egui::Stroke::new(2., self.accent()),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("live-answer")
                        .max_height(240.)
                        .stick_to_bottom(true)
                        .show(&mut columns[1], |ui| {
                            let draft = self
                                .experience
                                .live
                                .draft
                                .as_deref()
                                .filter(|t| !t.is_empty())
                                .unwrap_or("Waiting for the reader's first answer text…");
                            let response = ui.label(RichText::new(draft).monospace().size(12.));
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_description("Generated draft, may change before review.");
                                node.set_live(egui::accesskit::Live::Off);
                            });
                        });
                });
                if ui.small_button("Clear live view").clicked() {
                    self.experience.clear_preview();
                }
            });
    }
    pub(super) fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.experience.open {
            return;
        }
        let mut open = true;
        let focus_settings = std::mem::take(&mut self.experience.focus_settings);
        egui::Window::new("Settings").open(&mut open).default_size([760.,620.]).resizable(true).show(ctx,|ui| {
            ui.horizontal_wrapped(|ui| {
                for (index,title) in ["Everyday settings","Updates & installation","Hardware test","Setting descriptions"].iter().enumerate() {
                    let response = ui.selectable_value(&mut self.experience.tab,index,*title);
                    if focus_settings && index == self.experience.tab { response.request_focus(); }
                }
            }); ui.separator();
            egui::ScrollArea::vertical().max_height(620.).show(ui,|ui| {
                match self.experience.tab {
                    0 => {
                        ui.add_enabled_ui(self.worker.is_none(),|ui| {
                            ui.heading("Recognition & Word output");
                            egui::ComboBox::from_label(crate::localization::text(self.state.settings.ui.locale,"interface_language")).selected_text(self.state.settings.ui.locale.label()).show_ui(ui,|ui|{for locale in crate::localization::locales(){ui.selectable_value(&mut self.state.settings.ui.locale,locale,locale.label());}});
                            ui.small(crate::localization::coverage_note());
                            egui::ComboBox::from_label("Quality").selected_text(format!("{:?}",self.state.settings.preset)).show_ui(ui,|ui| { for p in [Preset::Fast,Preset::Balanced,Preset::Accuracy,Preset::Maximum] { ui.selectable_value(&mut self.state.settings.preset,p,format!("{p:?}")); } }); help(ui,"preset");
                            ui.add_space(8.); let label = ui.label("OCR languages"); ui.text_edit_singleline(&mut self.state.settings.languages).labelled_by(label.id); help(ui,"languages");
                            ui.checkbox(&mut self.state.settings.deskew,"Straighten tilted scans"); help(ui,"deskew");
                            ui.checkbox(&mut self.state.settings.auto_rotate,"Detect page orientation"); help(ui,"auto_rotate");
                            ui.checkbox(&mut self.state.settings.assistance.ask_questions,"Ask me about unclear writing"); help(ui,"assistance.ask_questions");
                            ui.checkbox(&mut self.state.settings.preserve_figures,"Keep diagrams in Word"); help(ui,"preserve_figures");
                            ui.checkbox(&mut self.state.settings.keep_page_breaks,"Keep original page breaks"); help(ui,"keep_page_breaks");
                            ui.add_space(12.); ui.heading("Workspace");
                            ui.checkbox(&mut self.state.settings.experience.live_preview,"Show live AI source and draft"); help(ui,"experience.live_preview");
                            ui.checkbox(&mut self.state.settings.experience.warm_reader,"Keep one reader ready between compatible scans");
                            ui.add(egui::Slider::new(&mut self.state.settings.experience.warm_reader_ttl_secs,1..=60).text("Reader expiry in seconds"));
                            ui.small("Optional · off by default. Reuses one reset reader for up to 60 seconds. Native image buffers may remain in RAM until release. Retention sessions and hardware trials use fresh processes. Different readers or settings may require a reload.");
                            ui.checkbox(&mut self.state.settings.ui.reduced_motion,"Reduce animation"); help(ui,"ui.reduced_motion");
                            ui.checkbox(&mut self.state.settings.ui.dark,"Dark appearance");
                            ui.checkbox(&mut self.state.settings.ui.compact,"Compact spacing");
                            ui.add(egui::Slider::new(&mut self.state.settings.ui.scale,0.75..=2.).text("Interface scale")); help(ui,"ui.scale");
                            if ui.button("Advanced settings for power users").clicked() { self.draft=self.state.settings.clone(); self.raw=serde_json::to_string_pretty(&self.draft).unwrap_or_default(); self.advanced=true; }
                        });
                        self.warm_reader_status_ui(ui);
                    }
                    1 => self.updates_ui(ui,ctx),
                    2 => self.tuning_ui(ui,ctx),
                    _ => {
                        ui.heading("Configuration reference"); ui.label("Search by setting name or description. All options are editable in Advanced, including raw JSON.");
                        ui.add(egui::TextEdit::singleline(&mut self.experience.search).hint_text("Search settings…")).accessible_name("Search setting names and descriptions");
                        let query=self.experience.search.to_lowercase();
                        for (key,description) in settings_help::GUIDE { if query.is_empty() || key.contains(&query) || description.to_lowercase().contains(&query) { ui.add_space(7.); ui.strong(*key); ui.label(*description); } }
                    }
                }
                if !self.experience.error.is_empty() { Self::accessible_status(ui,"settings-error",&self.experience.error,true); }
            });
        });
        self.experience.open = open;
        if let Err(error) = self.sync_warm_reader_preferences() {
            self.experience.error = error.to_string();
        }
    }
    fn updates_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Local AI and dependencies");
        ui.label("One setup installs a compatible runtime and the reader models that fit this PC. Models can require several gigabytes. Downloads resume and are checksum verified.");
        if ui
            .add_enabled(
                self.worker.is_none() && self.hardware.is_some(),
                egui::Button::new("Install all required AI for this PC"),
            )
            .clicked()
        {
            let mut s = self.state.settings.clone();
            s.preset = Preset::Maximum;
            let preset = self.state.settings.preset;
            self.studio_task(ctx, move |c, log| {
                let mut next = crate::setup::install(&s, &Default::default(), &c, &log)?;
                next.preset = preset;
                atomic_json(Path::new("ai-settings.json"), &next)?;
                Ok(studio::StudioMessage::Setup(Box::new(next)))
            });
        }
        ui.separator();
        ui.checkbox(
            &mut self.state.settings.experience.check_updates,
            "Check for updates daily when idle",
        );
        help(ui, "experience.check_updates");
        let previous_channel = self.state.settings.experience.update_channel;
        egui::ComboBox::from_label("App update channel")
            .selected_text(previous_channel.label())
            .show_ui(ui, |ui| {
                for channel in [UpdateChannel::Stable, UpdateChannel::Preview] {
                    ui.selectable_value(
                        &mut self.state.settings.experience.update_channel,
                        channel,
                        channel.label(),
                    );
                }
            });
        if previous_channel != self.state.settings.experience.update_channel {
            self.experience.updates = None;
            self.experience.checked_session = false;
        }
        help(ui, "experience.update_channel");
        if self.state.settings.experience.update_channel == UpdateChannel::Preview {
            ui.colored_label(Color32::LIGHT_YELLOW, "Preview app releases are enabled. Check compatibility notes and keep a project backup before installing.");
        }
        let now = chrono::Utc::now();
        if let Some(until) = self
            .state
            .settings
            .experience
            .defer_updates_until
            .filter(|until| *until > now)
        {
            Self::accessible_status(
                ui,
                "update-deferral",
                &format!(
                    "Automatic publisher checks are deferred until {} UTC. Check all updates now bypasses the deferral.",
                    until.format("%Y-%m-%d %H:%M")
                ),
                false,
            );
        } else if !self.state.settings.experience.check_updates {
            ui.label(
                "Automatic publisher checks are off. Check all updates now remains available.",
            );
        }
        ui.horizontal_wrapped(|ui| {
            for days in [7, 30] {
                if ui
                    .button(format!("Defer automatic checks for {days} days"))
                    .clicked()
                {
                    self.state.settings.experience.defer_updates_until =
                        Some(now + chrono::Duration::days(days));
                    self.experience.checked_session = false;
                }
            }
            if self.state.settings.experience.defer_updates_until.is_some()
                && ui.button("Clear update deferral").clicked()
            {
                self.state.settings.experience.defer_updates_until = None;
                self.experience.checked_session = false;
            }
        });
        help(ui, "experience.defer_updates_until");
        if ui
            .add_enabled(
                !self.experience.checking,
                egui::Button::new(if self.experience.checking {
                    "Checking publishers…"
                } else {
                    "Check all updates now"
                }),
            )
            .clicked()
        {
            self.request_update_check(ctx, maintenance::CheckMode::Manual);
        }
        if self.experience.checking {
            Self::accessible_status(
                ui,
                "publisher-check-status",
                "Checking publishers for updates.",
                false,
            );
        } else if let Some(report) = &self.experience.updates {
            Self::accessible_status(
                ui,
                "publisher-check-status",
                &format!(
                    "Update check finished: {} updates available; {} components could not be checked.",
                    report.count(),
                    report.errors.len()
                ),
                false,
            );
        }
        if let Some(report) = self.experience.updates.clone() {
            if report.channel != self.state.settings.experience.update_channel {
                ui.label("The completed check used the previous app channel. Check again to see results for the selected channel.");
                return;
            }
            ui.small(format!("Checked app channel: {}", report.channel.label()));
            ui.small(format!("Last checked: {}", report.checked_at));
            if report.count() == 0 && report.errors.is_empty() {
                ui.label("All checked components are current.");
            }
            for m in &report.models {
                ui.label(format!(
                    "{} · new model revision {}",
                    m.model.name,
                    &m.update.revision[..8.min(m.update.revision.len())]
                ));
            }
            if let Some(r) = &report.runtime {
                ui.label(format!("AI runtime {} available", r.build));
            }
            if (!report.models.is_empty() || report.runtime.is_some())
                && ui
                    .add_enabled(
                        self.worker.is_none(),
                        egui::Button::new(format!(
                            "Install AI updates (up to {:.1} GiB)",
                            report.download_bytes() as f64 / 1073741824.
                        )),
                    )
                    .clicked()
            {
                let s = self.state.settings.clone();
                let report = report.clone();
                self.studio_task(ctx, move |c, log| {
                    Ok(studio::StudioMessage::Updated(Box::new(
                        maintenance::apply(&s, &report, &c, &log)?,
                    )))
                });
            }
            if let Some(app) = report.app {
                ui.label(format!(
                    "NoteGen {} includes bundled dependency updates.",
                    app.version
                ));
                ui.small(maintenance::app_compatibility(&app));
                egui::CollapsingHeader::new("Publisher release notes (plain text)")
                    .id_salt(("app-release-notes", &app.version))
                    .show(ui, |ui| {
                        ui.label(if app.release_notes.is_empty() {
                            "No release notes were provided by the publisher."
                        } else {
                            &app.release_notes
                        });
                    });
                if cfg!(windows)
                    && ui
                        .add_enabled(
                            self.worker.is_none(),
                            egui::Button::new(if app.prerelease {
                                "Download & run preview app update"
                            } else {
                                "Download & run app update"
                            }),
                        )
                        .clicked()
                {
                    self.studio_task(ctx, move |c, log| {
                        Ok(studio::StudioMessage::Installer(maintenance::download_app(
                            &app, &c, &log,
                        )?))
                    });
                }
            }
            for error in &report.errors {
                ui.colored_label(Color32::LIGHT_YELLOW, format!("Could not check: {error}"));
            }
            egui::CollapsingHeader::new("Component details").show(ui, |ui| {
                for note in &report.notes {
                    ui.small(note);
                }
            });
        }
        ui.add_space(8.);
        ui.small("PDFium, Tesseract, English language data and Visual C++ runtime are included in the installer. Their updates arrive together in NoteGen app updates. Individual model rollback is in Studio > Readers.");
        if cfg!(windows)
            && ui
                .add_enabled(
                    self.worker.is_none()
                        && crate::setup::runtime_root(&self.state.settings)
                            .join("previous-ai-runtime.json")
                            .is_file(),
                    egui::Button::new("Restore previous AI runtime"),
                )
                .clicked()
        {
            let s = self.state.settings.clone();
            self.studio_task(ctx, move |c, _| {
                Ok(studio::StudioMessage::Updated(Box::new(
                    maintenance::rollback_runtime(&s, &c)?,
                )))
            });
        }
    }
    fn tuning_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Optimize for this PC");
        ui.label("Tests CPU worker counts and, when local AI is installed, up to two timed recognition trials on a synthetic scan. Checks the exact number 1250 and verifies real GPU offload. Usually takes 1–3 minutes. Cancel from the workspace at any time.");
        if ui
            .add_enabled(
                self.worker.is_none(),
                egui::Button::new("Run hardware test"),
            )
            .clicked()
        {
            self.release_warm_reader();
            let s = self.state.settings.clone();
            self.studio_task(ctx, move |c, log| {
                Ok(studio::StudioMessage::Tuned(Box::new(tuning::run(
                    &s, &c, &log,
                )?)))
            });
        }
        if let Some(report) = self.experience.tuning.clone() {
            ui.label(format!(
                "{} · {} physical / {} logical cores · {} MiB RAM",
                report.hardware.cpu,
                report.hardware.physical_cores,
                report.hardware.logical_cores,
                report.hardware.ram_mib
            ));
            ui.strong(if report.measured_ai {
                "AI performance measured"
            } else {
                "CPU measured · AI not measured"
            });
            egui::Grid::new("hardware-trials")
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Probe");
                    ui.strong("Workers");
                    ui.strong("Time");
                    ui.strong("Result");
                    ui.end_row();
                    for (kind, trials) in [("CPU", &report.cpu), ("AI", &report.ai)] {
                        for trial in trials {
                            ui.label(kind);
                            ui.label(trial.threads.to_string());
                            ui.label(format!("{:.0} ms", trial.milliseconds));
                            ui.label(if trial.correct {
                                if trial.gpu_verified {
                                    "Correct · GPU verified"
                                } else {
                                    "Passed"
                                }
                            } else {
                                "Failed"
                            })
                            .on_hover_text(&trial.detail);
                            ui.end_row();
                        }
                    }
                });
            ui.label(format!(
                "Proposed CPU workers: {}. Quality settings remain unchanged.",
                report.proposed.cpu_threads
            ));
            for note in &report.notes {
                ui.small(note);
            }
            if ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new("Apply tested worker count"),
                )
                .clicked()
            {
                self.experience.previous_threads = Some(self.state.settings.cpu_threads);
                self.state.settings.cpu_threads = report.proposed.cpu_threads;
                self.notice =
                    "Hardware recommendation applied. Undo is available in Settings > Hardware."
                        .into();
            }
            if ui.button("Save hardware report").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("NoteGen-hardware-report.json")
                    .save_file()
                && let Err(e) = atomic_json(&path, &report)
            {
                self.experience.error = e.to_string();
            }
        }
        if self.experience.previous_threads.is_some()
            && ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new("Undo hardware recommendation"),
                )
                .clicked()
        {
            self.state.settings.cpu_threads = self.experience.previous_threads.take().unwrap_or(0);
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn clearing_or_disabling_preview_releases_the_source_texture() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.png");
        image::DynamicImage::new_rgb8(32, 48).save(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Desktop::new(Workspace::default());
        app.experience.preview(
            telemetry::Preview::source(path.clone(), 1, "reader", None),
            &ctx,
        );
        assert!(app.experience.texture.is_some());
        app.experience.preview(telemetry::Preview::clear(), &ctx);
        assert!(app.experience.texture.is_none());
        assert!(app.experience.live.image.is_none());
        app.experience
            .preview(telemetry::Preview::source(path, 1, "reader", None), &ctx);
        app.state.settings.experience.live_preview = false;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.live_preview_ui(ui));
        output.textures_delta.clear();
        assert!(app.experience.texture.is_none());
        assert!(app.experience.live.image.is_none());
        app.experience
            .preview(telemetry::Preview::draft(1, "late text"), &ctx);
        assert!(app.experience.live.draft.is_none());
    }
}

#[cfg(test)]
mod accessibility_tests {
    use super::*;
    use egui::accesskit::{Live, Role, TreeUpdate};

    fn tree(ctx: &egui::Context, mut render: impl FnMut(&mut egui::Ui)) -> TreeUpdate {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 1100.),
                )),
                ..Default::default()
            },
            |ui| render(ui),
        );
        output.textures_delta.clear();
        output.platform_output.accesskit_update.take().unwrap()
    }

    fn named_node<'a>(tree: &'a TreeUpdate, name: &str, role: Role) -> &'a egui::accesskit::Node {
        // egui stores visible Label text in value; explicit widget names use label.
        let text = |node: &egui::accesskit::Node| {
            if node.role() == Role::Label {
                node.value().or(node.label()).map(str::to_owned)
            } else {
                node.label().map(str::to_owned)
            }
        };
        &tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == role
                    && (text(node).as_deref() == Some(name)
                        || node.labelled_by().iter().any(|label| {
                            tree.nodes.iter().any(|(id, target)| {
                                id == label && text(target).as_deref() == Some(name)
                            })
                        }))
            })
            .unwrap_or_else(|| {
                let nodes = tree
                    .nodes
                    .iter()
                    .take(80)
                    .map(|(id, node)| {
                        format!(
                            "{id:?} {:?} label={:?} value={:?} labelled_by={:?}",
                            node.role(),
                            node.label()
                                .map(|s| s.chars().take(100).collect::<String>()),
                            node.value()
                                .map(|s| s.chars().take(100).collect::<String>()),
                            node.labelled_by()
                        )
                    })
                    .collect::<Vec<_>>();
                panic!("Missing accessible name: {name}, role: {role:?}; nodes={nodes:#?}")
            })
            .1
    }

    #[test]
    fn settings_inputs_have_persistent_meaningful_accessible_names() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Desktop::new(Workspace::default());
        app.settings_open(0);
        tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        let everyday = tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        assert_eq!(
            named_node(&everyday, "OCR languages", Role::TextInput).role(),
            Role::TextInput
        );
        app.settings_open(3);
        tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        let reference = tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        assert_eq!(
            named_node(
                &reference,
                "Search setting names and descriptions",
                Role::TextInput
            )
            .role(),
            Role::TextInput
        );
    }

    #[test]
    fn opening_settings_focuses_selected_tab_once_without_stealing_subsequent_focus() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Desktop::new(Workspace::default());
        app.settings_open(2);
        tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        let selected = tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        let focused = selected.nodes.iter().find(|(id, _)| *id == selected.focus);
        assert_eq!(focused.unwrap().1.label(), Some("Hardware test"));

        let mut outside = None;
        let next = tree(&ctx, |ui| {
            let response = ui.button("Outside settings");
            outside = Some(response.id);
            response.request_focus();
            app.settings_ui(ui.ctx());
        });
        assert_eq!(next.focus, outside.unwrap().accesskit_id());
        assert!(!app.experience.focus_settings);
    }

    #[test]
    fn task_status_is_polite_atomic_stable_and_does_not_take_focus() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut focus = None;
        let mut render = |ui: &mut egui::Ui, changed: bool| {
            let button = ui.button("Continue reviewing");
            focus = Some(button.id);
            button.request_focus();
            if changed {
                ui.label("Additional task details");
            }
            Desktop::accessible_status(
                ui,
                "test-status",
                if changed {
                    "Export failed"
                } else {
                    "Export started"
                },
                changed,
            );
        };
        let first = tree(&ctx, |ui| render(ui, false));
        let second = tree(&ctx, |ui| render(ui, true));
        let first_id = first
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::Status)
            .unwrap()
            .0;
        let (second_id, node) = second
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::Status)
            .unwrap();
        assert_eq!(first_id, *second_id);
        assert_eq!(node.label(), Some("Export failed"));
        assert_eq!(node.live(), Some(Live::Polite));
        assert!(node.is_live_atomic());
        assert_eq!(second.focus, focus.unwrap().accesskit_id());
    }

    #[test]
    fn publisher_result_and_settings_error_are_announced_without_document_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Desktop::new(Workspace::default());
        app.experience.updates = Some(maintenance::Report::default());
        app.experience.error = "Cannot save the selected hardware report".into();
        app.settings_open(1);
        tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        let result = tree(&ctx, |ui| app.settings_ui(ui.ctx()));
        for label in [
            "Update check finished: 0 updates available; 0 components could not be checked.",
            "Cannot save the selected hardware report",
        ] {
            let node = named_node(&result, label, Role::Status);
            assert_eq!(node.role(), Role::Status);
            assert_eq!(node.live(), Some(Live::Polite));
        }
    }

    #[test]
    fn source_image_is_named_and_generated_draft_is_not_a_live_announcement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.png");
        image::DynamicImage::new_rgb8(32, 48).save(&path).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Desktop::new(Workspace::default());
        app.experience.preview(
            telemetry::Preview::source(path, 7, "Local reader", Some(BoxRect::full())),
            &ctx,
        );
        app.experience
            .preview(telemetry::Preview::draft(7, "PRIVATE DRAFT"), &ctx);
        tree(&ctx, |ui| app.live_preview_ui(ui));
        let result = tree(&ctx, |ui| app.live_preview_ui(ui));
        let source = named_node(&result, "Source page 7 for Local reader", Role::Image);
        assert_eq!(source.role(), Role::Image);
        assert!(source.description().unwrap().contains("Highlighted region"));
        let draft = named_node(&result, "PRIVATE DRAFT", Role::Label);
        assert_eq!(draft.live(), Some(Live::Off));
        assert_ne!(draft.role(), Role::Status);
        assert!(!result.nodes.iter().any(|(_, node)| {
            matches!(node.live(), Some(Live::Polite | Live::Assertive))
                && node
                    .label()
                    .is_some_and(|text| text.contains("PRIVATE DRAFT"))
        }));
    }
}

#[cfg(test)]
mod warm_preferences_tests {
    use super::*;
    #[test]
    fn legacy_preferences_are_cold_and_expiry_is_validated_even_when_disabled() {
        let mut preferences: ExperienceOptions = serde_json::from_str("{}").unwrap();
        assert!(!preferences.warm_reader);
        assert_eq!(preferences.warm_reader_ttl_secs, 30);
        for value in [0, 61, u64::MAX] {
            preferences.warm_reader_ttl_secs = value;
            assert!(preferences.validate().is_err());
            let settings = Settings {
                experience: preferences.clone(),
                ..Default::default()
            };
            assert!(settings.validate().is_err());
        }
        for value in [1, 60] {
            preferences.warm_reader_ttl_secs = value;
            preferences.validate().unwrap();
        }
    }
    #[test]
    fn changed_and_disabled_preferences_close_old_leases_and_preserve_other_settings() {
        let mut app = Desktop::new(Workspace::default());
        app.state.settings.subject = "Keep document context".into();
        app.state.settings.experience.warm_reader = true;
        app.sync_warm_reader_preferences().unwrap();
        let original = app.warm_pool.clone().unwrap();
        let control = Control::default();
        let job = original.begin(&control).unwrap();
        app.state.settings.experience.warm_reader_ttl_secs = 1;
        app.sync_warm_reader_preferences().unwrap();
        assert!(control.cancelled.load(Ordering::Acquire));
        assert!(job.commit().is_err());
        assert!(original.begin(&Control::default()).is_err());
        let replacement = app.warm_pool.clone().unwrap();
        assert!(!Arc::ptr_eq(&original, &replacement));
        app.state.settings.experience.warm_reader = false;
        app.sync_warm_reader_preferences().unwrap();
        assert!(app.warm_pool.is_none());
        assert!(replacement.begin(&Control::default()).is_err());
        assert_eq!(app.state.settings.subject, "Keep document context");
    }
    #[test]
    fn dropping_desktop_shuts_a_pool_still_referenced_by_a_worker() {
        let mut app = Desktop::new(Workspace::default());
        app.state.settings.experience.warm_reader = true;
        app.sync_warm_reader_preferences().unwrap();
        let pool = app.warm_pool.clone().unwrap();
        let control = Control::default();
        let job = pool.begin(&control).unwrap();
        drop(app);
        assert!(control.cancelled.load(Ordering::Acquire));
        assert!(job.commit().is_err());
        assert!(pool.begin(&Control::default()).is_err());
    }
    #[test]
    fn visible_accessible_release_button_actually_cancels_the_owned_job() {
        let mut app = Desktop::new(Workspace::default());
        app.state.settings.experience.warm_reader = true;
        app.sync_warm_reader_preferences().unwrap();
        let pool = app.warm_pool.clone().unwrap();
        let control = Control::default();
        let job = pool.begin(&control).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000., 400.));
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ui| app.warm_reader_status_ui(ui),
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let button = &tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::Button
                    && node.label() == Some("Stop task and release reader")
            })
            .unwrap()
            .1;
        assert!(!button.is_disabled());
        let bounds = button.bounds().unwrap();
        assert!(bounds.x1 > bounds.x0 && bounds.y1 > bounds.y0);
        assert!(bounds.x0 >= 0. && bounds.y0 >= 0. && bounds.x1 <= 1000. && bounds.y1 <= 400.);
        let position = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.) as f32,
            ((bounds.y0 + bounds.y1) / 2.) as f32,
        );
        assert!(viewport.contains(position));
        for pressed in [true, false] {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    events: vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.warm_reader_status_ui(ui),
            );
        }
        assert!(control.cancelled.load(Ordering::Acquire));
        assert!(job.commit().is_err());
        assert!(app.notice.contains("released"));
    }
}
