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
    }
    fn request_update_check(&mut self, ctx: &egui::Context) {
        if self.experience.checking {
            return;
        }
        self.experience.checking = true;
        self.experience.checked_session = true;
        self.experience.error.clear();
        let _ = atomic_json(
            Path::new("user-data/last-update-check.json"),
            &chrono::Utc::now(),
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
            && self.clock.elapsed().as_secs() >= 10
            && self.hardware.is_some()
            && self.worker.is_none()
            && self.state.settings.experience.check_updates
        {
            self.experience.checked_session = true;
            let recent = read_json::<chrono::DateTime<chrono::Utc>>(
                Path::new("user-data/last-update-check.json"),
                1000,
            )
            .is_ok_and(|last| chrono::Utc::now().signed_duration_since(last).num_hours() < 24);
            if !recent {
                self.request_update_check(ctx);
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
                        let response = columns[0].image((texture.id(), texture.size_vec2() * fit));
                        if let Some(b) = &self.experience.live.region {
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
                            ui.label(RichText::new(draft).monospace().size(12.));
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
        egui::Window::new("Settings").open(&mut open).default_size([760.,620.]).resizable(true).show(ctx,|ui| {
            ui.horizontal_wrapped(|ui| {
                for (index,title) in ["Everyday settings","Updates & installation","Hardware test","Setting descriptions"].iter().enumerate() { ui.selectable_value(&mut self.experience.tab,index,*title); }
            }); ui.separator();
            egui::ScrollArea::vertical().max_height(620.).show(ui,|ui| {
                match self.experience.tab {
                    0 => {
                        ui.add_enabled_ui(self.worker.is_none(),|ui| {
                            ui.heading("Recognition & Word output");
                            egui::ComboBox::from_label(crate::localization::text(self.state.settings.ui.locale,"interface_language")).selected_text(self.state.settings.ui.locale.label()).show_ui(ui,|ui|{for locale in crate::localization::locales(){ui.selectable_value(&mut self.state.settings.ui.locale,locale,locale.label());}});
                            ui.small(crate::localization::coverage_note());
                            egui::ComboBox::from_label("Quality").selected_text(format!("{:?}",self.state.settings.preset)).show_ui(ui,|ui| { for p in [Preset::Fast,Preset::Balanced,Preset::Accuracy,Preset::Maximum] { ui.selectable_value(&mut self.state.settings.preset,p,format!("{p:?}")); } }); help(ui,"preset");
                            ui.add_space(8.); ui.label("OCR languages"); ui.text_edit_singleline(&mut self.state.settings.languages); help(ui,"languages");
                            ui.checkbox(&mut self.state.settings.deskew,"Straighten tilted scans"); help(ui,"deskew");
                            ui.checkbox(&mut self.state.settings.auto_rotate,"Detect page orientation"); help(ui,"auto_rotate");
                            ui.checkbox(&mut self.state.settings.assistance.ask_questions,"Ask me about unclear writing"); help(ui,"assistance.ask_questions");
                            ui.checkbox(&mut self.state.settings.preserve_figures,"Keep diagrams in Word"); help(ui,"preserve_figures");
                            ui.checkbox(&mut self.state.settings.keep_page_breaks,"Keep original page breaks"); help(ui,"keep_page_breaks");
                            ui.add_space(12.); ui.heading("Workspace");
                            ui.checkbox(&mut self.state.settings.experience.live_preview,"Show live AI source and draft"); help(ui,"experience.live_preview");
                            ui.checkbox(&mut self.state.settings.ui.reduced_motion,"Reduce animation"); help(ui,"ui.reduced_motion");
                            ui.checkbox(&mut self.state.settings.ui.dark,"Dark appearance");
                            ui.checkbox(&mut self.state.settings.ui.compact,"Compact spacing");
                            ui.add(egui::Slider::new(&mut self.state.settings.ui.scale,0.75..=2.).text("Interface scale")); help(ui,"ui.scale");
                            if ui.button("Advanced settings for power users").clicked() { self.draft=self.state.settings.clone(); self.raw=serde_json::to_string_pretty(&self.draft).unwrap_or_default(); self.advanced=true; }
                        });
                    }
                    1 => self.updates_ui(ui,ctx),
                    2 => self.tuning_ui(ui,ctx),
                    _ => {
                        ui.heading("Configuration reference"); ui.label("Search by setting name or description. All options are editable in Advanced, including raw JSON.");
                        ui.add(egui::TextEdit::singleline(&mut self.experience.search).hint_text("Search settings…"));
                        let query=self.experience.search.to_lowercase();
                        for (key,description) in settings_help::GUIDE { if query.is_empty() || key.contains(&query) || description.to_lowercase().contains(&query) { ui.add_space(7.); ui.strong(*key); ui.label(*description); } }
                    }
                }
                if !self.experience.error.is_empty() { ui.colored_label(Color32::LIGHT_RED,&self.experience.error); }
            });
        });
        self.experience.open = open;
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
            self.request_update_check(ctx);
        }
        if let Some(report) = self.experience.updates.clone() {
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
                if cfg!(windows)
                    && ui
                        .add_enabled(
                            self.worker.is_none(),
                            egui::Button::new("Download & run app update"),
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
