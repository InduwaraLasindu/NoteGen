//! Context, writer memory and model management, kept separate from the conversion view.
use super::*;
use crate::{
    assistance::{self, Answer, Batch},
    fontpack::{self, FontOptions},
    models, writers,
};

pub(super) enum StudioMessage {
    Project(Box<Project>, String),
    Update(models::Update),
    Web(Vec<assistance::WebSource>),
    Done(String),
    Setup(Box<Settings>),
    Tuned(Box<crate::tuning::Report>),
    Updated(Box<Settings>),
    Installer(PathBuf),
}
pub(super) struct Pending {
    pub batch: Batch,
    pub answers: Vec<Answer>,
    pub index: usize,
    pub reply: Option<mpsc::Sender<Vec<Answer>>>,
    pub texture: Option<egui::TextureHandle>,
    pub loaded: usize,
    pub error: String,
}
impl Pending {
    pub fn new(batch: Batch, reply: Option<mpsc::Sender<Vec<Answer>>>) -> Self {
        let answers = batch
            .questions
            .iter()
            .map(|q| Answer {
                id: q.id.clone(),
                ..Default::default()
            })
            .collect();
        Self {
            batch,
            answers,
            index: 0,
            reply,
            texture: None,
            loaded: usize::MAX,
            error: String::new(),
        }
    }
}
struct Teach {
    project: Project,
    page: usize,
    region: BoxRect,
    label: String,
    glyph: bool,
    font_shape: usize,
    alternate: u8,
    notes: String,
    texture: Option<egui::TextureHandle>,
}
pub(super) struct Studio {
    pub open: bool,
    pub tab: usize,
    pub profiles: Vec<writers::Profile>,
    name: String,
    rename: String,
    notes: String,
    selected: String,
    delete_confirm: bool,
    pub pending: Option<Pending>,
    teach: Option<Teach>,
    pub updates: Vec<models::Update>,
    custom: planning::Model,
    external: bool,
    hub: bool,
    hub_repo: String,
    hub_weights: String,
    hub_projector: String,
    query: String,
    sources: Vec<assistance::WebSource>,
    font: FontOptions,
    error: String,
}
impl Studio {
    pub fn new(s: &Settings) -> Self {
        let mut custom = planning::catalog()
            .into_iter()
            .find(|m| m.role == "vision")
            .expect("catalog vision reader");
        custom.id = "my-vision-reader".into();
        custom.name = "My vision reader".into();
        custom.family = "my-vision-family".into();
        custom.weights = PathBuf::new();
        custom.projector = PathBuf::new();
        custom.rank = 150;
        custom.alias = "local-model".into();
        custom.source = String::new();
        Self {
            open: false,
            tab: 0,
            profiles: writers::list(&s.writer.root).unwrap_or_default(),
            name: String::new(),
            rename: String::new(),
            notes: String::new(),
            selected: String::new(),
            delete_confirm: false,
            pending: None,
            teach: None,
            updates: vec![],
            custom,
            external: false,
            hub: false,
            hub_repo: String::new(),
            hub_weights: String::new(),
            hub_projector: String::new(),
            query: String::new(),
            sources: vec![],
            font: FontOptions::default(),
            error: String::new(),
        }
    }
}
impl Desktop {
    pub fn studio_open(&mut self, tab: usize) {
        self.studio.tab = tab.min(3);
        self.studio.open = true;
        self.reload_writers();
    }
    pub(super) fn reconsider_review(&mut self, project: &Project, ctx: &egui::Context) {
        let project = project.clone();
        let settings = self.state.settings.clone();
        self.studio_task(ctx,move|c,log|{let pages=project.manifest.pages.clone();let n=pipeline::reconsider(&project,&pages,&settings,&c,&log)?;Ok(StudioMessage::Done(format!("Re-read {n} pages with context. New alternatives are in Review; export a new Word file after checking.")))});
    }
    fn reload_writers(&mut self) {
        match writers::list(&self.state.settings.writer.root) {
            Ok(p) => self.studio.profiles = p,
            Err(e) => self.studio.error = e.to_string(),
        }
    }
    pub(super) fn studio_message(&mut self, msg: StudioMessage, ctx: &egui::Context) {
        match msg {
            StudioMessage::Tuned(report) => {
                self.notice =
                    "Hardware test complete. Review the measured results in Settings > Hardware."
                        .into();
                self.experience.tuning = Some(*report);
                self.experience.open = true;
                self.experience.tab = 2;
            }
            StudioMessage::Updated(settings) => {
                self.state.settings = *settings;
                self.draft = self.state.settings.clone();
                self.raw = serde_json::to_string_pretty(&self.draft).unwrap_or_default();
                self.experience.updates = None;
                self.notice = "Selected local AI updates installed and verified. Previous versions remain available for rollback.".into();
                self.refresh(ctx);
            }
            StudioMessage::Installer(path) => match hardware::command(path).spawn() {
                Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.notice = format!("Installer downloaded, but could not start: {e}"),
            },
            StudioMessage::Setup(settings) => {
                self.state.settings = *settings;
                self.draft = self.state.settings.clone();
                self.raw = serde_json::to_string_pretty(&self.draft).unwrap_or_default();
                self.notice = "Local AI is installed and verified. You can convert now.".into();
                self.refresh(ctx);
            }

            StudioMessage::Project(project, notice) => {
                let project = *project;
                if self.open_review(project.root, ctx) {
                    self.notice = notice;
                }
            }
            StudioMessage::Update(u) => {
                self.studio.updates.retain(|v| v.id != u.id);
                self.notice = if u.previous_revision == u.revision {
                    format!("{} is already at the current publisher revision", u.id)
                } else {
                    format!(
                        "{}: new revision available. Review and install in Studio.",
                        u.id
                    )
                };
                self.studio.updates.push(u);
            }
            StudioMessage::Web(s) => {
                self.studio.sources = s;
                self.notice =
                    "Public references received. Add only relevant background to context.".into();
            }
            StudioMessage::Done(s) => {
                self.notice = s;
                self.reload_writers();
                self.refresh(ctx);
                if let Some(r) = &self.review
                    && !r.dirty
                    && let Ok(next) = Review::load(r.project.clone(), r.index, ctx)
                {
                    self.review = Some(next);
                }
            }
        }
        self.busy_setup = false;
        self.stage = "Ready".into();
        if let Some(e) = &mut self.estimate {
            e.finish(self.clock.elapsed().as_secs_f64(), true);
        }
        self.progress = 1.;
        self.transfer = None;
        self.started = None;
        self.completed_at = None;
        self.finished();
    }
    pub(super) fn studio_task<F>(&mut self, ctx: &egui::Context, task: F)
    where
        F: FnOnce(Control, Activity) -> Result<StudioMessage> + Send + 'static,
    {
        if self.worker.is_some() {
            return;
        }
        if let Err(error) = self.flush_review() {
            self.notice = error.to_string();
            return;
        }
        self.release_warm_reader();
        self.control = self.fresh_operation_control();
        let c = self.control.clone();
        let tx = self.tx.clone();
        let cx = ctx.clone();
        self.busy_setup = true;
        self.progress = 0.;
        self.estimate = Some(Estimate::new(self.clock.elapsed().as_secs_f64()));
        self.batch = None;
        self.batch_index = None;
        self.transfer = None;
        self.started = Some(Instant::now());
        self.error_code.clear();

        self.worker = Some(thread::spawn(move || {
            let events = tx.clone();
            let repaint = cx.clone();
            let log = Activity {
                path: None,
                callback: Some(Arc::new(move |e| {
                    let _ = events.send(Message::Event(e));
                    repaint.request_repaint();
                })),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(c, log)));
            let msg = match result {
                Ok(Ok(v)) => Message::Studio(v),
                Ok(Err(e)) => failure(e),
                Err(_) => Message::Failed(
                    "NG500".into(),
                    "Background task stopped; existing files are preserved.".into(),
                ),
            };
            let _ = tx.send(msg);
            cx.request_repaint();
        }));
    }
    pub(super) fn context_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Writer").color(self.muted()));
            let writer = self.state.settings.writer.id.clone();
            let title = self
                .studio
                .profiles
                .iter()
                .find(|p| Some(&p.id) == writer.as_ref())
                .map_or("No writer profile", |p| p.name.as_str())
                .to_string();
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                egui::ComboBox::from_id_salt("active-writer")
                    .selected_text(title)
                    .width(150.)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.state.settings.writer.id,
                            None,
                            "No writer profile",
                        );
                        for p in &self.studio.profiles {
                            ui.selectable_value(
                                &mut self.state.settings.writer.id,
                                Some(p.id.clone()),
                                format!("{} · {} examples", p.name, p.samples.len()),
                            );
                        }
                    });
            });
            if ui.button("Context & writers").clicked() {
                self.studio_open(0);
            }
            if ui.button("Model studio").clicked() {
                self.studio_open(2);
            }
            if !self.state.settings.subject.trim().is_empty() {
                ui.small(RichText::new("● Context added").color(self.accent()));
            }
            if self.studio.pending.is_some() {
                ui.colored_label(self.accent(), "● Your clarification is needed");
            }
        });
    }
    pub(super) fn studio_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.studio.open;
        egui::Window::new("NoteGen Studio")
            .open(&mut open)
            .default_size([850., 700.])
            .resizable(true)
            .fade_in(motion::enabled(&self.state.settings.ui))
            .fade_out(motion::enabled(&self.state.settings.ui))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Make every page familiar.")
                            .size(24.)
                            .strong(),
                    );
                });
                ui.label(
                    RichText::new(
                        "Context, confirmed handwriting, and the local readers you choose.",
                    )
                    .color(self.muted()),
                );
                ui.add_space(12.);
                ui.horizontal_wrapped(|ui| {
                    for (i, label) in [
                        "Context & questions",
                        "Writers & fonts",
                        "Readers & updates",
                        "Public knowledge",
                    ]
                    .iter()
                    .enumerate()
                    {
                        let response = ui.selectable_label(self.studio.tab == i, *label);
                        self.hover_ring(ui, &response, &format!("studio-tab-{i}"));
                        if response.clicked() {
                            self.studio.tab = i;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("studio-body")
                    .max_height(540.)
                    .show(ui, |ui| match self.studio.tab {
                        0 => self.context_controls(ui),
                        1 => self.writer_controls(ui, ctx),
                        2 => self.reader_controls(ui, ctx),
                        _ => self.knowledge_controls(ui, ctx),
                    });
                if !self.studio.error.is_empty() {
                    ui.colored_label(Color32::from_rgb(241, 166, 137), &self.studio.error);
                }
                if self.busy_setup {
                    ui.separator();
                    self.progress_ui(ui);
                    ui.horizontal(|ui| {
                        self.activity_indicator(ui);
                        ui.label(&self.stage);
                        if ui.button("Cancel task").clicked() {
                            self.control.cancelled.store(true, Ordering::Relaxed);
                        }
                    });
                }
            });
        self.studio.open = open;
        self.clarification_ui(ctx);
        self.teach_ui(ctx);
    }
    fn context_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.worker.is_none(),|ui|{
   ui.strong("What are these notes about?");
   ui.label("Add the subject, purpose, people, abbreviations, dates or labels the reader needs to understand. Source text remains the evidence.");
   ui.add(egui::TextEdit::multiline(&mut self.state.settings.subject).hint_text("Example: Fluid mechanics lecture. A and B are tanks. Q denotes volume flow rate; handwritten r often looks like v.").desired_width(f32::INFINITY).desired_rows(4).char_limit(8000));
   ui.add_space(10.);ui.strong("Names, terminology and context");
   ui.add(egui::TextEdit::multiline(&mut self.state.settings.assistance.glossary).hint_text("Names, technical words, units and their meanings…").desired_width(f32::INFINITY).desired_rows(3).char_limit(8000));
   ui.checkbox(&mut self.state.settings.assistance.ask_questions,"Ask me about unclear words, numbers and pictures before export");
   ui.add(egui::Slider::new(&mut self.state.settings.assistance.max_questions,0..=32).text("Maximum questions per document"));
   ui.checkbox(&mut self.state.settings.writer.use_examples,"Use confirmed examples from the selected writer in compatible vision readers");
  });
        ui.small("You can skip any question. Unattended CLI jobs save questions for later review. Exact corrections never average or guess conflicting digits.");
        if let Some(r) = &self.review {
            let project = r.project.clone();
            if ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new("Open clarification inbox for reviewed project"),
                )
                .clicked()
            {
                match assistance::questions(&project, 32, self.state.settings.review_threshold) {
                    Ok(q) if !q.is_empty() => {
                        self.studio.pending = Some(Pending::new(
                            Batch {
                                project: project.root.clone(),
                                questions: q,
                            },
                            None,
                        ))
                    }
                    Ok(_) => {
                        self.notice =
                            "No unanswered uncertain regions. Use Review to label any other area."
                                .into()
                    }
                    Err(e) => self.studio.error = e.to_string(),
                }
            }
        }
        if !self.state.settings.assistance.sources.is_empty() {
            ui.separator();
            ui.strong("Attached public background");
            let mut remove = None;
            for (i, s) in self.state.settings.assistance.sources.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.hyperlink_to(&s.title, &s.url);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.state.settings.assistance.sources.remove(i);
            }
        }
    }
    fn writer_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label("A writer profile remembers only examples you confirm. It supplies real letter/word shapes to compatible vision readers. Neural model weights are unchanged.");
        ui.add_enabled_ui(self.worker.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.studio.name)
                        .hint_text("Name a writer")
                        .char_limit(120),
                );
                if ui.button("Add person").clicked() {
                    match writers::create(&self.state.settings.writer.root, &self.studio.name) {
                        Ok(p) => {
                            self.state.settings.writer.id = Some(p.id);
                            self.studio.name.clear();
                            self.reload_writers();
                        }
                        Err(e) => self.studio.error = e.to_string(),
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                for p in &self.studio.profiles {
                    ui.selectable_value(
                        &mut self.state.settings.writer.id,
                        Some(p.id.clone()),
                        &p.name,
                    );
                }
            });
        });
        let Some(profile) = self
            .studio
            .profiles
            .iter()
            .find(|p| Some(&p.id) == self.state.settings.writer.id.as_ref())
            .cloned()
        else {
            ui.add_space(14.);
            ui.label("Add a writer, then select a source region in Review → Label handwriting / context.");
            if ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new("Import writer profile (.zip)"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Writer profile", &["zip"])
                    .pick_file()
            {
                let root = self.state.settings.writer.root.clone();
                self.studio_task(ctx, move |c, _| {
                    let p = writers::import_profile(&root, &path, &c)?;
                    Ok(StudioMessage::Done(format!("Imported writer {}", p.name)))
                });
            }

            return;
        };
        if self.studio.selected != profile.id {
            self.studio.selected = profile.id.clone();
            self.studio.rename = profile.name.clone();
            self.studio.notes = profile.notes.clone();
            self.studio.font.family = format!("{} handwriting", profile.name);
            self.studio.delete_confirm = false;
        }
        ui.add_space(12.);
        ui.horizontal(|ui|{
   let progress=profile.samples.iter().filter(|s|s.glyph).map(|s|&s.label).collect::<std::collections::HashSet<_>>().len() as f32/62.;
   let (rect,_)=ui.allocate_exact_size(Vec2::splat(66.),egui::Sense::hover());ui.painter().circle_stroke(rect.center(),26.,egui::Stroke::new(4.,self.border()));
   let amount=ui.ctx().animate_value_with_time(egui::Id::unique(("writer-progress",&profile.id)),progress.min(1.),motion::transition(&self.state.settings.ui));
   let points=(0..=60).map(|i|{let angle=-std::f32::consts::FRAC_PI_2+i as f32/60.*std::f32::consts::TAU*amount;rect.center()+Vec2::angled(angle)*26.}).collect();ui.painter().add(egui::Shape::line(points,egui::Stroke::new(4.,self.accent())));
   ui.vertical(|ui|{ui.strong(format!("{} confirmed examples · revision {}",profile.samples.len(),profile.revision));ui.small("Visual memory grows when you label source crops. AI guesses are never added automatically.");});
  });
        ui.add_enabled_ui(self.worker.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.studio.rename);
            });
            ui.label("Writing habits / notes");
            ui.add(
                egui::TextEdit::multiline(&mut self.studio.notes)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .char_limit(4000),
            );
            if ui.button("Save writer details").clicked() {
                match writers::update(
                    &self.state.settings.writer.root,
                    &profile.id,
                    &self.studio.rename,
                    &self.studio.notes,
                ) {
                    Ok(_) => {
                        self.reload_writers();
                        self.notice = "Writer details saved locally.".into();
                    }
                    Err(e) => self.studio.error = e.to_string(),
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new("Export writer profile"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Writer profile", &["zip"])
                    .set_file_name(format!("{}-profile.zip", profile.id))
                    .save_file()
            {
                let root = self.state.settings.writer.root.clone();
                let id = profile.id.clone();
                self.studio_task(ctx, move |c, _| {
                    writers::export_profile(&root, &id, &path, &c)?;
                    Ok(StudioMessage::Done(format!(
                        "Writer profile exported: {}",
                        path.display()
                    )))
                });
            }
            if ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new("Import writer profile"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Writer profile", &["zip"])
                    .pick_file()
            {
                let root = self.state.settings.writer.root.clone();
                self.studio_task(ctx, move |c, _| {
                    let p = writers::import_profile(&root, &path, &c)?;
                    Ok(StudioMessage::Done(format!("Imported writer {}", p.name)))
                });
            }
        });
        ui.separator();
        ui.strong("Font coverage");
        ui.horizontal_wrapped(|ui| {
            for (redo, label) in [
                (false, "Undo learned-example edit"),
                (true, "Redo learned-example edit"),
            ] {
                if ui
                    .add_enabled(self.worker.is_none(), egui::Button::new(label))
                    .clicked()
                {
                    let root = self.state.settings.writer.root.clone();
                    let id = profile.id.clone();
                    self.studio_task(ctx, move |_, _| {
                        let profile = if redo {
                            crate::writer_history::redo(&root, &id)?
                        } else {
                            crate::writer_history::undo(&root, &id)?
                        };
                        Ok(StudioMessage::Done(format!(
                            "{} · active revision {}",
                            label, profile.revision
                        )))
                    });
                }
            }
        });
        ui.small(crate::writer_history::RETENTION_NOTE);
        let coverage = writers::coverage(&profile);
        ui.horizontal_wrapped(|ui| {
            for ch in ('A'..='Z').chain('a'..='z').chain('0'..='9') {
                ui.label(RichText::new(ch.to_string()).monospace().color(
                    if coverage.contains(ch) {
                        self.accent()
                    } else {
                        self.muted().gamma_multiply(0.45)
                    },
                ));
            }
        });
        ui.label(format!(
            "Confirmed Unicode glyphs: {}",
            if coverage.is_empty() {
                "none"
            } else {
                &coverage
            }
        ));
        match fontpack::coverage(&self.state.settings.writer.root, &profile.id) {
            Ok(report) => {
                for shape in report.shapes {
                    ui.label(format!(
                        "{} · {} · {} confirmed example(s)",
                        shape.label,
                        shape.feature,
                        shape.sample_ids.len()
                    ));
                }
                if !report.component_placeholders.is_empty() {
                    ui.label(format!(
                        "Unconfirmed component placeholders: {}",
                        report.component_placeholders
                    ));
                }
                ui.small(report.note);
            }
            Err(error) => {
                ui.colored_label(Color32::LIGHT_RED, error.to_string());
            }
        }
        ui.small("Teach a visible Unicode character, a whole grapheme cluster, a ligature, or a numbered stylistic alternate. Only confirmed samples provide outlines.");
        egui::CollapsingHeader::new("Font options & export")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Font family");
                    ui.text_edit_singleline(&mut self.studio.font.family);
                });
                ui.add(
                    egui::Slider::new(&mut self.studio.font.spacing, 0..=500)
                        .text("Letter spacing"),
                );
                ui.add(
                    egui::Slider::new(&mut self.studio.font.threshold, 32..=224)
                        .text("Ink threshold / average weight"),
                );
                if ui
                    .add_enabled(
                        self.worker.is_none(),
                        egui::Button::new("Save shaping proof sheet"),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("HTML proof", &["html"])
                        .set_file_name(format!("{}-proof.html", profile.id))
                        .save_file()
                {
                    let root = self.state.settings.writer.root.clone();
                    let id = profile.id.clone();
                    let options = self.studio.font.clone();
                    self.studio_task(ctx, move |c, _| {
                        crate::profile_tools::font_proof_sheet(&root, &id, &path, &options, &c)?;
                        Ok(StudioMessage::Done(format!(
                            "Shaping proof saved: {}",
                            path.display()
                        )))
                    });
                }
                if ui
                    .add_enabled(
                        self.worker.is_none(),
                        egui::Button::new("Export font pack (.ttf + SVG + profile + examples)"),
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Font pack", &["zip"])
                        .set_file_name(format!("{}-font.zip", profile.id))
                        .save_file()
                {
                    let root = self.state.settings.writer.root.clone();
                    let id = profile.id.clone();
                    let options = self.studio.font.clone();
                    self.studio_task(ctx, move |c, _| {
                        let n = fontpack::export(&root, &id, &path, &options, &c)?;
                        Ok(StudioMessage::Done(format!(
                            "Exported {n} confirmed glyphs to {}",
                            path.display()
                        )))
                    });
                }
            });
        egui::CollapsingHeader::new("Confirmed examples (latest 30)").show(ui, |ui| {
            let mut remove = None;
            for sample in profile.samples.iter().rev().take(30) {
                ui.horizontal(|ui| {
                    ui.monospace(&sample.label);
                    ui.small(match &sample.font_shape {
                        Some(writers::FontShape::Ligature) => "font ligature",
                        Some(writers::FontShape::Cluster) => "font cluster",
                        Some(writers::FontShape::Alternate { .. }) => "font alternate",
                        None if sample.glyph => "Unicode glyph",
                        None => "word / phrase",
                    });
                    if ui
                        .add_enabled(self.worker.is_none(), egui::Button::new("Forget example"))
                        .clicked()
                    {
                        remove = Some(sample.id.clone());
                    }
                });
            }
            if let Some(id) = remove {
                match writers::delete_example(&self.state.settings.writer.root, &profile.id, &id) {
                    Ok(_) => self.reload_writers(),
                    Err(e) => self.studio.error = e.to_string(),
                }
            }
        });
        ui.separator();
        ui.add_enabled_ui(self.worker.is_none(), |ui| {
            ui.checkbox(
                &mut self.studio.delete_confirm,
                "I want to delete this writer and their confirmed examples",
            );
            if ui
                .add_enabled(
                    self.studio.delete_confirm,
                    egui::Button::new("Delete writer profile"),
                )
                .clicked()
            {
                match writers::remove(&self.state.settings.writer.root, &profile.id) {
                    Ok(()) => {
                        self.state.settings.writer.id = None;
                        self.reload_writers();
                        self.studio.delete_confirm = false;
                    }
                    Err(e) => self.studio.error = e.to_string(),
                }
            }
        });
    }
    fn reader_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.ai_setup_controls(ui, ctx);
        ui.add_space(12.);
        ui.label("Use compatible third-party GGUF + projector pairs, or an OpenAI-compatible vision server on this PC. Local server aliases must match /v1/models. Remote document inference stays blocked.");
        let installed = self.installed.clone();
        for m in &installed {
            egui::Frame::new()
                .fill(self.surface())
                .corner_radius(self.state.settings.ui.rounding)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(&m.name);
                        ui.small(format!(
                            "{} · {} · {}",
                            m.family,
                            m.prompt,
                            if m.endpoint.is_some() {
                                "local service"
                            } else {
                                "managed native reader"
                            }
                        ));
                    });
                    ui.small(if m.revision.is_empty() {
                        "Revision: locally registered / untracked".into()
                    } else {
                        format!("Revision: {}", m.revision)
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                self.worker.is_none() && m.hub.is_some() && m.endpoint.is_none(),
                                egui::Button::new("Check publisher update"),
                            )
                            .clicked()
                        {
                            let model = m.clone();
                            self.studio_task(ctx, move |c, _| {
                                Ok(StudioMessage::Update(models::check(&model, &c)?))
                            });
                        }
                        if ui
                            .add_enabled(
                                self.worker.is_none()
                                    && self
                                        .state
                                        .settings
                                        .models_dir
                                        .join(format!("{}.previous.json", m.id))
                                        .exists(),
                                egui::Button::new("Roll back"),
                            )
                            .clicked()
                        {
                            let settings = self.state.settings.clone();
                            let id = m.id.clone();
                            self.studio_task(ctx, move |c, log| {
                                models::rollback_validated(&settings, &id, &c, &log)?;
                                Ok(StudioMessage::Done(format!(
                                    "Restored previous version of {id}"
                                )))
                            });
                        }
                        if !self.state.settings.automatic_models {
                            let mut allowed = self.state.settings.model_ids.contains(&m.id);
                            if ui
                                .add_enabled(
                                    self.worker.is_none(),
                                    egui::Checkbox::new(&mut allowed, "Allow reader"),
                                )
                                .changed()
                            {
                                self.state.settings.model_ids.retain(|id| id != &m.id);
                                if allowed {
                                    self.state.settings.model_ids.push(m.id.clone());
                                }
                            }
                        }
                    });
                    if let Some(update) = self
                        .studio
                        .updates
                        .iter()
                        .find(|u| u.id == m.id && u.revision != m.revision)
                        .cloned()
                    {
                        ui.label(format!(
                            "Available {} · {:.2} GiB pair · verified SHA256",
                            &update.revision[..12],
                            (update.weights.bytes + update.projector.bytes) as f64 / 1073741824.
                        ));
                        if ui
                            .add_enabled(
                                self.worker.is_none(),
                                egui::Button::new("Download, verify & activate this revision"),
                            )
                            .clicked()
                        {
                            if let Err(error) = self.prepare_download_control() {
                                self.notice = error.to_string();
                            } else {
                                let settings = self.state.settings.clone();
                                let model = m.clone();
                                self.studio_task(ctx, move |c, log| {
                                    let m = models::install_validated(
                                        &settings, &model, &update, &c, &log,
                                    )?;
                                    Ok(StudioMessage::Done(format!(
                                        "Activated {}. Previous version retained.",
                                        m.name
                                    )))
                                });
                            }
                        }
                    }
                });
            ui.add_space(6.);
        }
        if installed.is_empty() {
            ui.label("No registered readers yet. Advanced → Models contains the curated catalog and runtime setup.");
        }
        ui.add_enabled_ui(self.worker.is_none(), |ui| {
            ui.checkbox(
                &mut self.state.settings.automatic_models,
                "Automatically select the strongest fitting independent reader families",
            );
        });
        egui::CollapsingHeader::new("Add a third-party reader").show(ui,|ui|{
   ui.add_enabled_ui(self.worker.is_none(),|ui|{
    let form=&mut self.studio.custom;
    egui::Grid::new("custom-reader").num_columns(2).show(ui,|ui|{
     ui.label("Reader ID");ui.text_edit_singleline(&mut form.id);ui.end_row();ui.label("Display name");ui.text_edit_singleline(&mut form.name);ui.end_row();ui.label("Independent family");ui.text_edit_singleline(&mut form.family);ui.end_row();
     ui.label("Role");egui::ComboBox::from_id_salt("custom-role").selected_text(&form.role).show_ui(ui,|ui|{ui.selectable_value(&mut form.role,"vision".into(),"Vision + context");ui.selectable_value(&mut form.role,"ocr".into(),"OCR");});ui.end_row();
     ui.label("Prompt protocol");egui::ComboBox::from_id_salt("custom-protocol").selected_text(&form.prompt).show_ui(ui,|ui|{for label in ["structured","lighton","glm"]{ui.selectable_value(&mut form.prompt,label.into(),label);}});ui.end_row();
     numeric(ui,"Memory estimate (MiB)",&mut form.resident_mib,512..=200000);numeric(ui,"Preference rank",&mut form.rank,0..=1000);
    });
    ui.small("Use the same family ID for variants of the same model. Variants do not become independent consensus votes.");
    ui.checkbox(&mut self.studio.external,"Already running in a local vision server (LM Studio / Ollama / vLLM)");
    if self.studio.external{
     let endpoint=self.studio.custom.endpoint.get_or_insert_with(||"http://127.0.0.1:1234".into());ui.horizontal(|ui|{ui.label("Loopback root");ui.text_edit_singleline(endpoint);});ui.horizontal(|ui|{ui.label("Exact server alias");ui.text_edit_singleline(&mut self.studio.custom.alias);});ui.small("The service must accept images via /v1/chat/completions. Its own settings control GPU allocation.");
    }else{path_control(ui,"Weights GGUF",&mut self.studio.custom.weights,false);path_control(ui,"Matching projector",&mut self.studio.custom.projector,false);}
    ui.checkbox(&mut self.studio.hub,"Enable optional Hugging Face revision updates");
    if self.studio.hub{ui.horizontal(|ui|{ui.label("owner/repository");ui.text_edit_singleline(&mut self.studio.hub_repo);});ui.horizontal(|ui|{ui.label("Weights filename");ui.text_edit_singleline(&mut self.studio.hub_weights);});ui.horizontal(|ui|{ui.label("Projector filename");ui.text_edit_singleline(&mut self.studio.hub_projector);});}
    if ui.button("Verify & register third-party reader").clicked(){let mut model=self.studio.custom.clone();if !self.studio.external{model.endpoint=None;}
    if self.studio.hub{model.hub=Some(models::Hub{repository:self.studio.hub_repo.clone(),weights:self.studio.hub_weights.clone(),projector:self.studio.hub_projector.clone()});}else{model.hub=None;}let settings=self.state.settings.clone();self.studio_task(ctx,move|c,log|{let m=models::save_custom_validated(&settings,model,&c,&log)?;Ok(StudioMessage::Done(format!("Registered {}. Select it automatically or in the reader allow-list.",m.name)))});}
   });
  });
        if ui.button("Curated catalog & runtime settings").clicked() {
            self.advanced_open();
            self.tab = 2;
        }
    }
    fn knowledge_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.strong("Optional public background");
        ui.label("Search a public topic when a diagram or specialist term needs context. Only the query you type is sent to Wikipedia. Scans, OCR text and writer examples stay on this PC.");
        ui.checkbox(
            &mut self.state.settings.assistance.web_lookup,
            "Enable public topic lookup",
        );
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.studio.query)
                    .hint_text("Public topic, e.g. siphon diagram")
                    .desired_width(400.)
                    .char_limit(500),
            );
            if ui
                .add_enabled(
                    self.worker.is_none() && self.state.settings.assistance.web_lookup,
                    egui::Button::new("Search public references"),
                )
                .clicked()
            {
                let query = self.studio.query.clone();
                let enabled = self.state.settings.assistance.web_lookup;
                self.studio_task(ctx, move |c, _| {
                    Ok(StudioMessage::Web(assistance::search(&query, enabled, &c)?))
                });
            }
        });
        let mut add = None;
        for (i, s) in self.studio.sources.iter().enumerate() {
            ui.add_space(12.);
            ui.hyperlink_to(&s.title, &s.url);
            ui.label(&s.excerpt);
            if ui
                .add_enabled(
                    self.worker.is_none(),
                    egui::Button::new("Add this source to scan context"),
                )
                .clicked()
            {
                add = Some(i);
            }
        }
        if let Some(i) = add {
            let source = self.studio.sources[i].clone();
            if self.state.settings.assistance.sources.len() < 12
                && !self
                    .state
                    .settings
                    .assistance
                    .sources
                    .iter()
                    .any(|s| s.url == source.url)
            {
                self.state.settings.assistance.sources.push(source);
                self.notice = "Reference added as background for subsequent scans.".into();
            }
        }
        ui.add_space(10.);
        ui.small("Public references can be incomplete or wrong. They guide contextual interpretation and never supply missing source words or digits.");
    }
    pub(super) fn prepare_teach(&mut self, r: &Review, region: BoxRect, ctx: &egui::Context) {
        if r.page.confirmed_redactions.iter().any(|mask| {
            region.left < mask.region.right
                && region.right > mask.region.left
                && region.top < mask.region.bottom
                && region.bottom > mask.region.top
        }) {
            self.notice =
                "Choose visible writing outside confirmed source masks for a learned example."
                    .into();
            return;
        }
        let image =
            match crate::redactions::effective_page(&r.project, &r.page, &Control::default()) {
                Ok((_, image, _)) => image,
                Err(error) => {
                    self.notice = error.to_string();
                    return;
                }
            };
        let texture = Some({
            let im = image;
            let (x, y, w, h) = region.pixels(im.width(), im.height());
            texture(
                ctx,
                "teach-crop",
                im.crop_imm(x, y, w - x, h - y).thumbnail(600, 260),
            )
        });
        self.studio.teach = Some(Teach {
            project: r.project.clone(),
            page: r.page.page,
            region,
            label: String::new(),
            glyph: false,
            font_shape: 0,
            alternate: 1,
            notes: String::new(),
            texture,
        });
    }
    fn teach_ui(&mut self, ctx: &egui::Context) {
        let Some(mut teach) = self.studio.teach.take() else {
            return;
        };
        let mut open = true;
        let mut saved = false;
        egui::Window::new("Label handwriting / context").open(&mut open).default_width(650.).fade_in(motion::enabled(&self.state.settings.ui)).show(ctx,|ui|{
   if let Some(t)=&teach.texture{ui.image((t.id(),t.size_vec2()));}
   ui.label("What does this area say or represent?");ui.add(egui::TextEdit::singleline(&mut teach.label).hint_text("Exact confirmed letter, word, number or phrase").char_limit(160).desired_width(f32::INFINITY));
   ui.checkbox(&mut teach.glyph,"One visible Unicode character (also builds font coverage)");
   egui::ComboBox::from_label("Confirmed font form").selected_text(["Recognition example / default glyph", "Ligature", "Grapheme cluster", "Stylistic alternate"][teach.font_shape.min(3)]).show_ui(ui, |ui| {
       for (index,label) in ["Recognition example / default glyph", "Ligature", "Grapheme cluster", "Stylistic alternate"].iter().enumerate() { ui.selectable_value(&mut teach.font_shape,index,*label); }
   });
   if teach.font_shape == 3 { ui.add(egui::Slider::new(&mut teach.alternate, 1..=16).text("Alternate index")); }
   ui.label("Optional meaning, topic, special spelling or context");ui.add(egui::TextEdit::multiline(&mut teach.notes).desired_rows(3).char_limit(4000).desired_width(f32::INFINITY));
   let writer=self.state.settings.writer.id.clone();let name=self.studio.profiles.iter().find(|p|Some(&p.id)==writer.as_ref()).map(|p|p.name.clone());
   ui.horizontal_wrapped(|ui|{
    if ui.add_enabled(!teach.notes.trim().is_empty(),egui::Button::new("Save project context")).clicked(){match assistance::add_context(&teach.project,&format!("Page {}, region {:?}: {}. Label: {}",teach.page,teach.region,teach.notes,teach.label)){Ok(())=>{self.notice="Project context saved. Use Re-read with context to obtain a new interpretation.".into();saved=true;},Err(e)=>self.studio.error=e.to_string()}}
    if ui.add_enabled(self.worker.is_none() && name.is_some() && !teach.label.trim().is_empty(),egui::Button::new(format!("Teach {}",name.as_deref().unwrap_or("a writer")))).clicked(){let root=self.state.settings.writer.root.clone();let id=writer.clone().unwrap();let project=teach.project.clone();let number=teach.page;let region=teach.region.clone();let label=teach.label.clone();let glyph=teach.glyph;let shape=match teach.font_shape { 1=>Some(writers::FontShape::Ligature), 2=>Some(writers::FontShape::Cluster), 3=>Some(writers::FontShape::Alternate { index:teach.alternate }), _=>None };let source=format!("{} page {}",teach.project.root.display(),teach.page);self.studio_task(ctx,move|c,_|{let page=project.load(number)?;let (_,image,_)=crate::redactions::effective_page(&project,&page,&c)?;if let Some(shape)=shape { writers::add_font_example(&root,&id,&image,region,&label,shape,&source)?; } else { writers::add_example(&root,&id,&image,region,&label,glyph,&source)?; } Ok(StudioMessage::Done("Confirmed handwriting example saved. Future scans can use this shape.".into()))});saved=true;}
   });
   if name.is_none(){ui.small("Select or add a writer in Studio → Writers & fonts to remember letter and word shapes.");}
  });
        if open && !saved {
            self.studio.teach = Some(teach);
        }
    }
    fn clarification_ui(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.studio.pending.take() else {
            return;
        };
        let mut open = true;
        let mut submit = false;
        if pending.batch.questions.is_empty() {
            return;
        }
        pending.index = pending.index.min(pending.batch.questions.len() - 1);
        if pending.loaded != pending.index {
            let q = &pending.batch.questions[pending.index];
            pending.texture = writers::open_image(&q.image, 40_000_000).ok().map(|im| {
                let image = if let Some(b) = &q.region {
                    let (x, y, w, h) = b.pixels(im.width(), im.height());
                    im.crop_imm(x, y, w - x, h - y)
                } else {
                    im
                };
                texture(ctx, "clarify-region", image.thumbnail(640, 300))
            });
            pending.loaded = pending.index;
        }
        egui::Window::new("A little help with this page").open(&mut open).default_width(700.).fade_in(motion::enabled(&self.state.settings.ui)).show(ctx,|ui|{
   let question=&pending.batch.questions[pending.index];ui.label(RichText::new(format!("Question {} of {} · page {}",pending.index+1,pending.batch.questions.len(),question.page)).color(self.accent()));ui.strong(&question.prompt);
   if let Some(t)=&pending.texture{ui.image((t.id(),t.size_vec2()));}ui.label(format!("Current reading: {}",question.reading.chars().take(1000).collect::<String>()));
   for alt in &question.alternatives{ui.small(format!("Alternative: {}",alt.chars().take(500).collect::<String>()));}
   let answer=&mut pending.answers[pending.index];
   ui.label(if question.field=="meaning"{"Your explanation of this diagram (optional)"}else{"Exact replacement for the text / table cell shown above (optional)"});
   ui.add(egui::TextEdit::multiline(&mut answer.replacement).desired_rows(3).desired_width(f32::INFINITY).char_limit(120000));
   ui.label("Extra context for a new AI reading (optional)");ui.add(egui::TextEdit::multiline(&mut answer.context).desired_rows(2).char_limit(4000).desired_width(f32::INFINITY));
   let can_learn=self.state.settings.writer.id.is_some() && question.region.is_some() && question.field=="text";
   ui.add_enabled(can_learn,egui::Checkbox::new(&mut answer.learn,"Remember this confirmed crop for the selected writer"));
   if answer.learn{ui.checkbox(&mut answer.glyph,"One visible Unicode character");}
   if !pending.error.is_empty(){ui.colored_label(Color32::from_rgb(240,160,130),&pending.error);}
   ui.small("Leave the correction empty when unsure. Context can request a new local vision reading; it never silently replaces confirmed text.");
   ui.horizontal(|ui|{
    if ui.add_enabled(pending.index>0,egui::Button::new("← Previous")).clicked(){pending.index-=1;}
    if ui.button("Skip this").clicked(){pending.answers[pending.index].skip=true;if pending.index+1<pending.answers.len(){pending.index+=1;}else{submit=true;}}
    if ui.add_enabled(pending.index+1<pending.answers.len(),egui::Button::new("Next →")).clicked(){pending.answers[pending.index].skip=false;pending.index+=1;}
    if ui.button(if pending.reply.is_some(){"Apply answers & continue"}else{"Save answers"}).clicked(){pending.answers[pending.index].skip=false;submit=true;}
   });
  });
        if submit
            && let Err(error) =
                assistance::validate_answers(&pending.batch.questions, &pending.answers)
        {
            pending.error = error.to_string();
            self.studio.pending = Some(pending);
            return;
        }
        if !open {
            for answer in &mut pending.answers {
                if assistance::validate_answers(
                    &pending.batch.questions,
                    std::slice::from_ref(answer),
                )
                .is_err()
                {
                    answer.skip = true;
                }
            }
        }
        if !open || submit {
            if let Some(reply) = pending.reply.take() {
                let _ = reply.send(pending.answers);
            } else {
                match Project::read(pending.batch.project.clone()).and_then(|project| {
                    assistance::apply(&project, &pending.batch.questions, &pending.answers)
                }) {
                    Ok(n) => {
                        self.notice = format!(
                            "Saved {n} confirmed corrections. Export a new Word file from Review."
                        );
                        if let Some(r) = &self.review
                            && r.project.root == pending.batch.project
                        {
                            let project = r.project.clone();
                            let index = r.index;
                            if let Ok(next) = Review::load(project, index, ctx) {
                                self.review = Some(next);
                            }
                        }
                    }
                    Err(e) => self.studio.error = e.to_string(),
                }
            }
        } else {
            self.studio.pending = Some(pending);
        }
    }
}
fn texture(ctx: &egui::Context, name: &str, im: image::DynamicImage) -> egui::TextureHandle {
    let rgba = im.to_rgba8();
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}
