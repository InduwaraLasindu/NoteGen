//! Keyboard-first review helpers shared by the native workbench.
use super::*;
use std::collections::BTreeSet;
use zeroize::Zeroize;

struct WordPreview {
    project: PathBuf,
    input: PathBuf,
    preview: crate::roundtrip::Preview,
    selected: BTreeSet<String>,
    page: usize,
}

enum Report {
    Catalog(Vec<crate::projects::Entry>),
    Search(crate::projects::SearchReport),
    Text(String),
    Word(WordPreview),
    Rule(PathBuf, crate::review::RulePreview),
    Resource(crate::resources::ResourcePreview),
    Credentials(crate::credentials::CredentialStatus),
}
#[derive(Default)]
pub(super) struct Workbench {
    pub open: bool,
    pub palette: bool,
    tab: usize,
    query: String,
    palette_query: String,
    entries: Vec<crate::projects::Entry>,
    catalog_loaded: bool,
    hits: Vec<crate::projects::SearchHit>,
    report: String,
    metadata: String,
    format: usize,
    find: String,
    replace: String,
    rule: Option<(PathBuf, crate::review::RulePreview)>,
    word: Option<WordPreview>,
    reader: String,
    region_edge: u32,
    region_task: usize,
    region_context: String,
    pdf_font: Option<PathBuf>,
    pdf_strict: bool,
    pdf_hide_marks: bool,
    resource_profile: usize,
    reserved_cores: usize,
    resource: Option<crate::resources::ResourcePreview>,
    resource_applied: bool,
    secret: String,
    credentials: Option<crate::credentials::CredentialStatus>,
    confirm_credential_removal: bool,
    rx: Option<mpsc::Receiver<std::result::Result<Report, String>>>,
}
impl Drop for Workbench {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}
impl Workbench {
    fn clear_secret(&mut self, ctx: &egui::Context) {
        self.secret.zeroize();
        ctx.data_mut(|data| {
            data.remove::<egui::text_edit::TextEditState>(egui::Id::unique("private-update-token"))
        });
    }
    fn task(&mut self, ctx: &egui::Context, f: impl FnOnce() -> Result<Report> + Send + 'static) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let value = match result {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => {
                    Err("The local tool stopped unexpectedly. Saved data is preserved.".into())
                }
            };
            let _ = tx.send(value);
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        let value = self.rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Local task disconnected".into())),
            Err(_) => None,
        });
        if let Some(value) = value {
            self.rx = None;
            match value {
                Ok(Report::Catalog(entries)) => {
                    self.entries = entries;
                    self.hits.clear();
                }
                Ok(Report::Search(report)) => {
                    self.hits = report.hits;
                    self.report = report.warnings.join("\n");
                    if report.truncated {
                        self.report
                            .push_str("\nMore matches exist; narrow the search.");
                    }
                }
                Ok(Report::Text(text)) => self.report = text,
                Ok(Report::Word(preview)) => {
                    self.report = "Word corrections inspected. Select only the changes you want to import; conflicts remain unchanged.".into();
                    self.word = Some(preview);
                }
                Ok(Report::Rule(project, preview)) => {
                    self.report = format!(
                        "{} replacement fields found. Inspect before applying.",
                        preview.matches.len()
                    );
                    self.rule = Some((project, preview));
                }
                Ok(Report::Resource(preview)) => {
                    self.resource = Some(preview);
                    self.resource_applied = false;
                }
                Ok(Report::Credentials(status)) => {
                    self.report = if status.supported {
                        format!(
                            "Private-update credential: {} in {}.",
                            if status.stored {
                                "stored"
                            } else {
                                "not stored"
                            },
                            status.storage
                        )
                    } else {
                        "Windows Credential Manager is unavailable on this platform.".into()
                    };
                    self.credentials = Some(status);
                    self.confirm_credential_removal = false;
                }
                Err(e) => self.report = e,
            }
        }
    }
}

impl Review {
    pub(super) fn save(&mut self, approved: bool) -> Result<()> {
        let mut page = self.page.clone();
        page.edited = true;
        page.reviewed = approved;
        let label = if self.revision_name.trim().is_empty() {
            if approved {
                "Approve page"
            } else {
                "Save draft"
            }
        } else {
            self.revision_name.trim()
        };
        crate::review::commit(&self.project, &self.baseline, &page, label)?;
        self.page = page.clone();
        self.baseline = page;
        self.dirty = false;
        self.issues = crate::review::issues(&self.project)?;
        self.revision_name.clear();
        Ok(())
    }
    fn history_action(&mut self, redo: bool) -> Result<()> {
        anyhow::ensure!(
            !self.dirty,
            Fault::new("NG500", "Save the current draft before undo or redo")
        );
        if redo {
            crate::review::redo(&self.project)?;
        } else {
            crate::review::undo(&self.project)?;
        }
        self.reload()
    }
    fn reload(&mut self) -> Result<()> {
        self.page = self.project.load(self.page.page)?;
        self.baseline = self.page.clone();
        self.raw = serde_json::to_string_pretty(&self.page.content)?;
        self.selected = self
            .selected
            .min(self.page.content.blocks.len().saturating_sub(1));
        self.issues = crate::review::issues(&self.project)?;
        self.dirty = false;
        Ok(())
    }
}

impl Desktop {
    fn export_tools(&mut self, w: &mut Workbench, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((project, dirty)) = self.review.as_ref().map(|r| (r.project.clone(), r.dirty))
        else {
            ui.label("Open a project or finish a conversion first.");
            return;
        };
        ui.label(format!("Current project: {}", project.root.display()));
        if dirty {
            ui.colored_label(
                Color32::LIGHT_YELLOW,
                "Save the current draft before exporting, previewing or importing changes.",
            );
        }
        let formats = [
            crate::exports::Format::Markdown,
            crate::exports::Format::Html,
            crate::exports::Format::Latex,
            crate::exports::Format::Spreadsheet,
            crate::exports::Format::Traceability,
        ];
        egui::ComboBox::from_label("Output format")
            .selected_text(format!("{:?}", formats[w.format.min(4)]))
            .show_ui(ui, |ui| {
                for (index, format) in formats.iter().enumerate() {
                    ui.selectable_value(&mut w.format, index, format!("{format:?}"));
                }
            });
        if ui
            .add_enabled(
                !dirty && self.worker.is_none() && w.rx.is_none(),
                egui::Button::new("Export a new file"),
            )
            .clicked()
        {
            let format = formats[w.format.min(4)];
            if let Some(output) = rfd::FileDialog::new()
                .set_file_name(format!("notes.{}", format.extension()))
                .save_file()
            {
                let project = project.clone();
                self.studio_task(ctx, move |c, _| {
                    crate::exports::export(&project, &output, format, &c)?;
                    Ok(studio::StudioMessage::Done(format!(
                        "Saved {format:?} export: {}",
                        output.display()
                    )))
                });
            }
        }
        egui::CollapsingHeader::new("Source-preserving searchable PDF").show(ui, |ui| {
            ui.label("Retains saved scan images and adds searchable text only where measured source boxes and font coverage are available. Unsupported regions stay image-only and are reported.");
            ui.small("The default export retains up to 80 million source pixels. Unresolved regions are marked unless you turn marks off.");
            let mut marks = !w.pdf_hide_marks;
            if ui.checkbox(&mut marks, "Mark unresolved regions").changed() { w.pdf_hide_marks = !marks; }
            ui.checkbox(&mut w.pdf_strict, "Stop if the font lacks a required character");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Choose local TrueType font").clicked() { w.pdf_font = rfd::FileDialog::new().add_filter("TrueType font", &["ttf"]).pick_file(); }
                if ui.button("Use default font").clicked() { w.pdf_font = None; }
                ui.small(w.pdf_font.as_ref().map_or("Automatic local font".into(), |p| p.display().to_string()));
            });
            if ui.add_enabled(!dirty && self.worker.is_none() && w.rx.is_none(), egui::Button::new("Export source PDF copy")).clicked()
                && let Some(output) = rfd::FileDialog::new().set_file_name("notes-source.pdf").add_filter("PDF", &["pdf"]).save_file() {
                let project = project.clone();
                let options = crate::searchable_pdf::Options { font: w.pdf_font.clone(), strict_coverage: w.pdf_strict, mark_uncertain: !w.pdf_hide_marks, ..Default::default() };
                self.studio_task(ctx, move |c, _| {
                    let result = crate::searchable_pdf::export(&project, &output, &options, &c)?;
                    let warnings = result.warnings.iter().take(8).map(|warning| format!("Page {}: {}", warning.page, warning.message)).collect::<Vec<_>>().join("\n");
                    Ok(studio::StudioMessage::Done(format!("PDF saved: {}. {} searchable blocks; {} image-only blocks; {} marked regions. {} warnings.\n{warnings}", output.display(), result.searchable_blocks, result.image_only_blocks, result.marked_regions, result.warnings.len() + result.omitted_warnings)))
                });
            }
        });
        ui.separator();
        self.word_correction_tools(w, &project, dirty, ui, ctx);
        ui.separator();
        ui.strong("Reversible replacement rule");
        ui.label("Preview literal replacements across the project. Applying selected rules creates one undoable revision and preserves reader evidence.");
        ui.horizontal(|ui| {
            ui.label("Find");
            ui.add(egui::TextEdit::singleline(&mut w.find).char_limit(1000));
            ui.label("Replace");
            ui.add(egui::TextEdit::singleline(&mut w.replace).char_limit(1000));
        });
        if ui
            .add_enabled(
                !dirty && self.worker.is_none() && w.rx.is_none() && !w.find.is_empty(),
                egui::Button::new("Preview rule"),
            )
            .clicked()
        {
            let rule = crate::review::Rule {
                find: w.find.clone(),
                replace: w.replace.clone(),
                pages: project.manifest.pages.clone(),
                exclude_blocks: vec![],
            };
            let project = project.clone();
            w.task(ctx, move || {
                let preview = crate::review::preview_rule(&project, &rule)?;
                Ok(Report::Rule(project.root, preview))
            });
        }
        let mut apply = false;
        if let Some((root, preview)) = &w.rule {
            if root != &project.root {
                ui.small("This rule preview belongs to another project. Create a new preview.");
            } else {
                for found in preview.matches.iter().take(40) {
                    ui.label(format!(
                        "Page {} block {}: {} → {}",
                        found.page,
                        found.block + 1,
                        found.before.chars().take(160).collect::<String>(),
                        found.after.chars().take(160).collect::<String>()
                    ));
                }
                ui.small(format!(
                    "{} matching fields; first 40 shown.",
                    preview.matches.len()
                ));
                apply = ui
                    .add_enabled(
                        !dirty
                            && self.worker.is_none()
                            && w.rx.is_none()
                            && !preview.matches.is_empty(),
                        egui::Button::new("Apply this rule preview"),
                    )
                    .clicked();
            }
        }
        if apply && let Some((_, preview)) = w.rule.take() {
            self.studio_task(ctx, move |_, _| {
                let revision =
                    crate::review::apply_rule(&project, &preview, "Confirmed replacement rule")?;
                Ok(studio::StudioMessage::Done(format!(
                    "Replacement rule saved as revision {revision}. Undo is available in Review."
                )))
            });
        }
    }

    fn word_correction_tools(
        &mut self,
        w: &mut Workbench,
        project: &Project,
        dirty: bool,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) {
        ui.strong("Import corrections from Word");
        ui.label("Choose a Word document exported by this version of NoteGen. Inspect changed fields and source conflicts before selecting corrections. Import preserves recognition history and can be undone.");
        if ui
            .add_enabled(
                !dirty && self.worker.is_none() && w.rx.is_none(),
                egui::Button::new("Inspect corrected Word document"),
            )
            .clicked()
            && let Some(input) = rfd::FileDialog::new()
                .add_filter("Word document", &["docx"])
                .pick_file()
        {
            let project = project.clone();
            w.word = None;
            w.task(ctx, move || {
                let preview = crate::roundtrip::preview(&project, &input, &Control::default())?;
                Ok(Report::Word(WordPreview {
                    project: project.root,
                    input,
                    preview,
                    selected: BTreeSet::new(),
                    page: 0,
                }))
            });
        }
        let mut apply = false;
        if let Some(word) = &mut w.word {
            if word.project != project.root {
                ui.small("This Word preview belongs to another project. Inspect the document again for the current project.");
                return;
            }
            ui.small(format!("Inspected: {}", word.input.display()));
            let changed = word
                .preview
                .fields
                .iter()
                .filter(|field| field.status == "changed")
                .count();
            let conflicts = word
                .preview
                .fields
                .iter()
                .filter(|field| field.status.contains("conflict"))
                .count();
            ui.label(format!(
                "{changed} changed fields · {conflicts} conflicts · {} selected",
                word.selected.len()
            ));
            for warning in word.preview.warnings.iter().take(12) {
                ui.colored_label(Color32::LIGHT_YELLOW, warning);
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !dirty && self.worker.is_none(),
                        egui::Button::new("Select all conflict-free changes"),
                    )
                    .clicked()
                {
                    word.selected = word
                        .preview
                        .fields
                        .iter()
                        .filter(|field| field.status == "changed")
                        .map(|field| field.tag.clone())
                        .collect();
                }
                if ui.button("Clear selection").clicked() {
                    word.selected.clear();
                }
            });
            let fields = word
                .preview
                .fields
                .iter()
                .filter(|field| field.status == "changed" || field.status.contains("conflict"))
                .collect::<Vec<_>>();
            let pages = fields.len().div_ceil(40).max(1);
            word.page = word.page.min(pages - 1);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(word.page > 0, egui::Button::new("Previous fields"))
                    .clicked()
                {
                    word.page -= 1;
                }
                ui.label(format!("{} / {pages}", word.page + 1));
                if ui
                    .add_enabled(word.page + 1 < pages, egui::Button::new("Next fields"))
                    .clicked()
                {
                    word.page += 1;
                }
            });
            for field in fields.into_iter().skip(word.page * 40).take(40) {
                ui.push_id(&field.tag, |ui| {
                    let mut selected = word.selected.contains(&field.tag);
                    let enabled = field.status == "changed" && !dirty && self.worker.is_none();
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Checkbox::new(
                                &mut selected,
                                format!(
                                    "Page {} · block {} · {} · {}",
                                    field.page,
                                    field.block + 1,
                                    field.field,
                                    field.status
                                ),
                            ),
                        )
                        .changed()
                    {
                        if selected {
                            word.selected.insert(field.tag.clone());
                        } else {
                            word.selected.remove(&field.tag);
                        }
                    }
                    egui::CollapsingHeader::new("Compare field").show(ui, |ui| {
                        ui.label(format!(
                            "Current: {}",
                            field.current.chars().take(1000).collect::<String>()
                        ));
                        ui.label(format!(
                            "Word: {}",
                            field
                                .imported
                                .as_deref()
                                .unwrap_or("Missing source control")
                                .chars()
                                .take(1000)
                                .collect::<String>()
                        ));
                        ui.small(&field.message);
                    });
                });
            }
            ui.small("Conflicts cannot be selected. Compare them against the scan and resolve them manually in Review. The document and project are checked again when importing.");
            apply = ui
                .add_enabled(
                    !dirty && self.worker.is_none() && w.rx.is_none() && !word.selected.is_empty(),
                    egui::Button::new("Import selected Word corrections"),
                )
                .clicked();
        }
        if apply && let Some(word) = w.word.take() {
            let project = project.clone();
            let selected = word.selected.into_iter().collect::<Vec<_>>();
            self.studio_task(ctx, move |c, _| { let revision = crate::roundtrip::apply(&project, &word.input, &word.preview, &selected, "Confirmed Word corrections", &c)?; Ok(studio::StudioMessage::Done(format!("Imported {} Word fields as revision {revision}. Review was refreshed; Undo is available.", selected.len()))) });
        }
    }

    fn region_reread_tools(&mut self, w: &mut Workbench, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((project, page, selected, region, dirty)) = self.review.as_ref().map(|r| {
            (
                r.project.clone(),
                r.page.page,
                r.selected,
                r.page
                    .content
                    .blocks
                    .get(r.selected)
                    .and_then(|b| b.bbox.clone()),
                r.dirty,
            )
        }) else {
            return;
        };
        ui.separator();
        ui.strong("Re-read one source region");
        ui.label(format!("Uses the selected Review block {} on page {page}. Choose a reader and task; the new reading is retained as evidence and alternatives.", selected + 1));
        if region.is_none() {
            ui.colored_label(Color32::LIGHT_YELLOW, "The selected block has no measured source bounds. Choose a grounded block in Review before re-reading.");
        }
        let readers = self
            .installed
            .iter()
            .filter(|reader| reader.endpoint.is_some() || !reader.weights.as_os_str().is_empty())
            .collect::<Vec<_>>();
        egui::ComboBox::from_label("Region reader")
            .selected_text(
                readers
                    .iter()
                    .find(|reader| reader.id == w.reader)
                    .map_or("Choose an installed reader", |reader| reader.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for reader in &readers {
                    ui.selectable_value(&mut w.reader, reader.id.clone(), &reader.name);
                }
            });
        egui::ComboBox::from_label("Region image edge")
            .selected_text(if w.region_edge == 0 {
                "Automatic".into()
            } else {
                format!("{} pixels", w.region_edge)
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut w.region_edge, 0, "Automatic");
                for edge in [768, 1024, 1536, 2048, 3072, 4096] {
                    ui.selectable_value(&mut w.region_edge, edge, format!("{edge} pixels"));
                }
            });
        let tasks = ["text", "handwriting", "table", "equation", "figure", "code"];
        egui::ComboBox::from_label("Region task")
            .selected_text(tasks[w.region_task.min(5)])
            .show_ui(ui, |ui| {
                for (index, task) in tasks.iter().enumerate() {
                    ui.selectable_value(&mut w.region_task, index, *task);
                }
            });
        ui.add(
            egui::TextEdit::multiline(&mut w.region_context)
                .desired_rows(2)
                .char_limit(8000)
                .hint_text("Optional context for this region"),
        );
        ui.small("Reader files and memory admission are verified in the worker. One reader supplies additional evidence, not independent consensus; confirmed text is not silently replaced.");
        if ui
            .add_enabled(
                !dirty
                    && self.worker.is_none()
                    && w.rx.is_none()
                    && region.is_some()
                    && readers.iter().any(|reader| reader.id == w.reader),
                egui::Button::new("Re-read selected block"),
            )
            .clicked()
        {
            let request = pipeline::RegionRequest {
                page,
                region: region.expect("enabled only for measured bounds"),
                reader: w.reader.clone(),
                image_edge: w.region_edge,
                context: w.region_context.clone(),
                task: tasks[w.region_task.min(5)].into(),
            };
            let settings = self.state.settings.clone();
            self.studio_task(ctx, move |c, log| { let revision = pipeline::reread_region(&project, &request, &settings, &c, &log)?; Ok(studio::StudioMessage::Done(format!("Region reading saved as revision {revision}. Compare source-linked alternatives in Review."))) });
        }
    }

    fn resource_tools(&mut self, w: &mut Workbench, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.separator();
        ui.heading("Application resource profile");
        ui.label("Preview worker, memory headroom and interface refresh limits. These settings affect future NoteGen work; they do not change Windows power settings or promise a measured thermal result.");
        let profiles = [
            crate::resources::PowerProfile::Quiet,
            crate::resources::PowerProfile::Battery,
            crate::resources::PowerProfile::Balanced,
            crate::resources::PowerProfile::Throughput,
        ];
        egui::ComboBox::from_label("Resource profile")
            .selected_text(profiles[w.resource_profile.min(3)].label())
            .show_ui(ui, |ui| {
                for (index, profile) in profiles.iter().enumerate() {
                    ui.selectable_value(&mut w.resource_profile, index, profile.label());
                }
            });
        ui.horizontal(|ui| {
            ui.label("CPU cores to leave available");
            ui.add(egui::DragValue::new(&mut w.reserved_cores).range(0..=511));
        });
        if ui
            .add_enabled(
                self.worker.is_none()
                    && w.rx.is_none()
                    && self.review.as_ref().is_none_or(|r| !r.dirty),
                egui::Button::new("Preview resource limits"),
            )
            .clicked()
        {
            let settings = self.state.settings.clone();
            let hardware = self.hardware.clone();
            let profile = profiles[w.resource_profile.min(3)];
            let reserve = crate::resources::Reservations {
                cpu_cores: w.reserved_cores,
                ..Default::default()
            };
            w.task(ctx, move || {
                let hardware = hardware
                    .unwrap_or_else(|| hardware::detect_for(&settings, &Control::default()));
                Ok(Report::Resource(crate::resources::preview(
                    &settings, &hardware, profile, &reserve,
                )?))
            });
        }
        let mut changed = false;
        if let Some(preview) = &w.resource {
            ui.label(format!(
                "Threads: {} → {} · RAM headroom: {} MiB · VRAM headroom: {} MiB",
                preview.before.cpu_threads,
                preview.after.cpu_threads,
                preview.after.reserve_ram_mib,
                preview.after.reserve_vram_mib
            ));
            ui.label(format!(
                "Admission budget: {} MiB RAM · {} MiB VRAM · {} active / {} idle FPS",
                preview.ram_budget_mib,
                preview.vram_budget_mib,
                preview.after.fps,
                preview.after.idle_fps
            ));
            for note in &preview.notes {
                ui.small(note);
            }
            let idle = self.worker.is_none()
                && w.rx.is_none()
                && self.review.as_ref().is_none_or(|r| !r.dirty);
            if ui
                .add_enabled(
                    idle && !w.resource_applied,
                    egui::Button::new("Apply previewed limits"),
                )
                .clicked()
            {
                match preview.apply(&mut self.state.settings) {
                    Ok(()) => {
                        w.resource_applied = true;
                        changed = true;
                        w.report =
                            "Resource limits applied to future jobs. Undo is available here."
                                .into();
                    }
                    Err(error) => w.report = error.to_string(),
                }
            }
            if ui
                .add_enabled(
                    idle && w.resource_applied,
                    egui::Button::new("Undo resource limits"),
                )
                .clicked()
            {
                match preview.undo(&mut self.state.settings) {
                    Ok(()) => {
                        w.resource_applied = false;
                        changed = true;
                        w.report = "Previous resource limits restored.".into();
                    }
                    Err(error) => w.report = error.to_string(),
                }
            }
        }
        if changed {
            self.draft = self.state.settings.clone();
            self.raw = serde_json::to_string_pretty(&self.draft).unwrap_or_default();
            self.refresh(ctx);
        }
    }

    fn credential_tools(&mut self, w: &mut Workbench, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.separator();
        ui.heading("Private publisher updates");
        ui.label("Store a token for private NoteGen release access in Windows Credential Manager. It is used only for the fixed NoteGen publisher release API and is excluded from settings, logs and diagnostics.");
        let idle = self.worker.is_none() && w.rx.is_none();
        if ui
            .add_enabled(idle, egui::Button::new("Check credential status"))
            .clicked()
        {
            w.task(ctx, || {
                Ok(Report::Credentials(crate::credentials::native().status()?))
            });
        }
        if let Some(status) = &w.credentials {
            ui.label(format!(
                "{} · {} · {}",
                status.storage,
                if status.stored {
                    "Token stored"
                } else {
                    "No token stored"
                },
                status.publisher
            ));
        }
        if !cfg!(windows) {
            ui.small("Credential storage is unavailable on this platform; public updates remain available.");
            w.clear_secret(ctx);
            return;
        }
        let mut output = egui::TextEdit::singleline(&mut w.secret)
            .id(egui::Id::unique("private-update-token"))
            .password(true)
            .char_limit(2048)
            .hint_text("Paste a private-update bearer token")
            .show(ui);
        output.state.clear_undoer();
        output.state.store(ctx, output.response.id);
        if ui
            .add_enabled(
                self.worker.is_none() && w.rx.is_none() && !w.secret.is_empty(),
                egui::Button::new("Store token in Windows"),
            )
            .clicked()
        {
            let secret = crate::credentials::Secret::new(std::mem::take(&mut w.secret));
            w.clear_secret(ctx);
            match secret {
                Ok(secret) => w.task(ctx, move || {
                    let store = crate::credentials::native();
                    store.set(secret)?;
                    Ok(Report::Credentials(store.status()?))
                }),
                Err(error) => w.report = error.to_string(),
            }
        }
        if ui.button("Clear pasted token").clicked() {
            w.clear_secret(ctx);
        }
        ui.checkbox(
            &mut w.confirm_credential_removal,
            "Remove the stored private-update token",
        );
        if ui
            .add_enabled(
                self.worker.is_none() && w.rx.is_none() && w.confirm_credential_removal,
                egui::Button::new("Delete stored token"),
            )
            .clicked()
        {
            w.clear_secret(ctx);
            w.confirm_credential_removal = false;
            w.task(ctx, || {
                let store = crate::credentials::native();
                store.delete()?;
                Ok(Report::Credentials(store.status()?))
            });
        }
    }

    pub(super) fn one_page_trial(&mut self, ctx: &egui::Context) {
        if self.worker.is_some() {
            return;
        }
        let Some(index) = self.state.next() else {
            return;
        };
        if let Err(e) = self.flush_review() {
            self.notice = e.to_string();
            return;
        }
        let input = self.state.queue[index].input.clone();
        let folder = if self.state.output_dir.trim().is_empty() {
            input.parent().unwrap_or(Path::new(".")).to_path_buf()
        } else {
            PathBuf::from(&self.state.output_dir)
        };
        let trial_name = PathBuf::from(format!(
            "{}-trial.pdf",
            input.file_stem().unwrap_or_default().to_string_lossy()
        ));
        let output = output_name(&trial_name, &folder);
        let mut settings = self.state.settings.clone();
        settings.assistance.ask_questions = false;
        let tx = self.tx.clone();
        let repaint = ctx.clone();
        self.control = Control::default();
        let c = self.control.clone();
        self.started = Some(Instant::now());
        self.estimate = Some(Estimate::new(self.clock.elapsed().as_secs_f64()));
        self.running_queue = false;
        self.batch = None;
        self.batch_index = None;
        self.stage = "Preparing a one-page trial".into();
        self.worker = Some(thread::spawn(move || {
            let events = tx.clone();
            let ctx = repaint.clone();
            let log = Activity {
                path: None,
                callback: Some(Arc::new(move |e| {
                    let _ = events.send(Message::Event(e));
                    ctx.request_repaint();
                })),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let total = crate::inputs::selected_page_count(&input, &settings.pdfium, "")?;
                let pages = pages_range(&settings.page_range, total)?;
                settings.page_range = pages
                    .first()
                    .ok_or_else(|| Fault::new("NG100", "Select a trial page"))?
                    .to_string();
                pipeline::convert(&input, &output, &settings, &c, &log)
            }));
            let msg = match result {
                Ok(Ok(done)) => Message::Complete(Box::new(done)),
                Ok(Err(e)) => failure(e),
                Err(_) => Message::Failed(
                    "NG500".into(),
                    "Trial stopped unexpectedly; completed checkpoints are preserved.".into(),
                ),
            };
            let _ = tx.send(msg);
            repaint.request_repaint();
        }));
    }
    pub(super) fn history_controls(&mut self, r: &mut Review, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let can_use = !r.dirty && self.worker.is_none();
            if ui
                .add_enabled(can_use, egui::Button::new("Undo revision"))
                .clicked()
                && let Err(e) = r.history_action(false)
            {
                self.review_error = e.to_string();
            }
            if ui
                .add_enabled(can_use, egui::Button::new("Redo revision"))
                .clicked()
                && let Err(e) = r.history_action(true)
            {
                self.review_error = e.to_string();
            }
            ui.add(
                egui::TextEdit::singleline(&mut r.revision_name)
                    .hint_text("Name the next saved revision")
                    .desired_width(210.),
            );
            ui.checkbox(&mut r.heatmap, "Disagreement overlay");
            ui.add(egui::Slider::new(&mut r.zoom, 0.5..=4.).text("Source zoom"));
        });
        egui::CollapsingHeader::new("Revision timeline").show(ui,|ui| {
            match crate::review::timeline(&r.project) {
                Ok(timeline) => {
                    if timeline.revisions.is_empty() { ui.small("Save a draft to create the first revision. Recognition evidence stays preserved."); }
                    for revision in timeline.revisions.iter().rev() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("{} · {} · {}",revision.id,revision.label,revision.created));
                            if ui.add_enabled(!r.dirty,egui::Button::new("Compare")).clicked() {
                                match crate::review::snapshot(&r.project,revision.id,r.page.page) {
                                    Ok(old) => match crate::review::compare(&old,&r.page).and_then(|d| Ok(serde_json::to_string_pretty(&d)?)) {Ok(text)=>self.review_error=text,Err(e)=>self.review_error=e.to_string()},
                                    Err(e) => self.review_error=e.to_string(),
                                }
                            }
                            if ui.add_enabled(!r.dirty,egui::Button::new("Restore")).clicked() {
                                match crate::review::restore(&r.project,revision.id).and_then(|_| r.reload()) {
                                    Ok(()) => self.review_error.clear(), Err(e)=>self.review_error=e.to_string(),
                                }
                            }
                        });
                    }
                }
                Err(e) => { ui.colored_label(Color32::LIGHT_RED,e.to_string()); }
            }
        });
    }
    pub(super) fn issue_controls(&mut self, r: &mut Review, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(format!("Review queue · {} suggestions",r.issues.len())).show(ui,|ui| {
            ui.small("Totals, number conflicts and uncertain regions are ranked by impact. Suggestions never rewrite a value.");
            let mut chosen=None;
            for issue in r.issues.iter().take(100) {
                let label=format!("P{} · page {} · {}",issue.priority,issue.page,issue.message);
                if ui.add_enabled(!r.dirty,egui::Button::new(label)).clicked() { chosen=Some((issue.page,issue.block)); }
            }
            if let Some((page,block))=chosen
                && let Some(index)=r.project.manifest.pages.iter().position(|n| *n==page) {
                    match Review::load(r.project.clone(),index,ui.ctx()) {
                        Ok(mut next) => {next.selected=block; *r=next;},
                        Err(e) => self.review_error=e.to_string(),
                    }
            }
        });
    }
    pub fn tools_open(&mut self, tab: usize) {
        self.workbench.open = true;
        self.workbench.tab = tab.min(3);
        self.workbench.catalog_loaded = false;
    }
    pub(super) fn workbench_ui(&mut self, ctx: &egui::Context) {
        self.workbench.poll();
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::K)) {
            self.workbench.palette = true;
        }
        if self.workbench.palette {
            let mut open = true;
            egui::Window::new("Command palette · Ctrl+K")
                .open(&mut open)
                .default_width(440.)
                .show(ctx, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.workbench.palette_query)
                            .hint_text("Find an action…"),
                    );
                    let query = self.workbench.palette_query.to_lowercase();
                    for (id, label) in [
                        "Import scans · Ctrl+O",
                        "Projects and search",
                        "Exports and correction rules",
                        "Recovery and references",
                        "Privacy and readability",
                        "Settings",
                        "Review results",
                        "Studio",
                    ]
                    .iter()
                    .enumerate()
                    {
                        if label.to_lowercase().contains(&query) && ui.button(*label).clicked() {
                            match id {
                                0 => self.pick_files(),
                                1..=4 => self.tools_open(id - 1),
                                5 => self.settings_open(0),
                                6 => self.review_open = true,
                                _ => self.studio.open = true,
                            }
                            self.workbench.palette = false;
                        }
                    }
                });
            self.workbench.palette &= open;
        }
        if !self.workbench.open {
            self.workbench.clear_secret(ctx);
            return;
        }
        let mut w = std::mem::take(&mut self.workbench);
        if w.tab == 0 && !w.catalog_loaded && w.rx.is_none() {
            w.catalog_loaded = true;
            let roots = self.state.recent.clone();
            let query = w.query.clone();
            w.task(ctx, move || {
                Ok(Report::Catalog(crate::projects::catalog(
                    &roots,
                    &crate::projects::Filter {
                        query,
                        ..Default::default()
                    },
                )?))
            });
        }
        let mut open = true;
        let mut jump = None;
        egui::Window::new("Project workbench").open(&mut open).default_size([850.,680.]).resizable(true).show(ctx,|ui| {
            ui.horizontal_wrapped(|ui| {for (i,label) in ["Projects & search","Export & rules","Recovery & references","Privacy & readability"].iter().enumerate(){ui.selectable_value(&mut w.tab,i,*label);}});
            ui.separator();
            if w.tab != 3 { w.clear_secret(ctx); }
            if w.rx.is_some(){ui.spinner();ui.small("Reading local project data…");ctx.request_repaint_after(std::time::Duration::from_millis(100));}
            egui::ScrollArea::vertical().max_height(600.).show(ui,|ui| {
                match w.tab {
                    0=>{
                        ui.label("Names, folders, tags and pending review across your recent projects. Search runs only in these project folders.");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut w.query);
                            if ui.add_enabled(w.rx.is_none(),egui::Button::new("Filter projects")).clicked(){let roots=self.state.recent.clone();let query=w.query.clone();w.task(ctx,move||Ok(Report::Catalog(crate::projects::catalog(&roots,&crate::projects::Filter{query,..Default::default()})?)));}
                            if ui.add_enabled(w.rx.is_none()&&!w.query.trim().is_empty(),egui::Button::new("Search recognized text")).clicked(){let roots=self.state.recent.clone();let query=w.query.clone();w.task(ctx,move||Ok(Report::Search(crate::projects::search(&roots,&query,100)?)));}
                            if ui.add_enabled(w.rx.is_none(),egui::Button::new("Find duplicates")).clicked(){let roots=self.state.recent.clone();w.task(ctx,move||Ok(Report::Text(serde_json::to_string_pretty(&crate::projects::duplicates(&roots)?)?)));}
                        });
                        for entry in &w.entries {ui.horizontal_wrapped(|ui| {if ui.button(&entry.metadata.name).clicked(){jump=Some((entry.root.clone(),None));}ui.small(format!("{} · {} pages · {} pending · {}",entry.metadata.status,entry.pages,entry.pending_review,entry.metadata.tags.join(", ")));if let Some(e)=&entry.error {ui.colored_label(Color32::LIGHT_RED,e);}});}
                        for hit in &w.hits {if ui.button(format!("Page {} · {}",hit.page,hit.excerpt)).clicked(){jump=Some((hit.project.clone(),Some((hit.page,hit.block))));}}
                        ui.separator();ui.strong("Project templates");
                        ui.horizontal(|ui| {
                            if ui.button("Save current policy as template").clicked() && let Some(path)=rfd::FileDialog::new().set_file_name("project-template.json").save_file(){match crate::projects::save_template(&path,"Project template",&self.state.settings){Ok(())=>w.report="Template saved. Device paths and writer identity are excluded.".into(),Err(e)=>w.report=e.to_string()}}
                            if ui.add_enabled(self.worker.is_none(),egui::Button::new("Load template")).clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Template",&["json"]).pick_file(){match crate::projects::load_template(&path){Ok(t)=>{self.state.settings=t.settings;w.report=format!("Loaded {}",t.name);},Err(e)=>w.report=e.to_string()}}
                        });
                    }
                    1=>self.export_tools(&mut w,ui,ctx),
                    2=>{
                        if let Some(r)=&self.review {
                            ui.horizontal(|ui| {
                                if ui.button("Inspect recovery").clicked(){let p=r.project.clone();w.task(ctx,move||Ok(Report::Text(serde_json::to_string_pretty(&crate::projects::recovery(&p)?)?)));}
                                if ui.button("Load project details").clicked(){match crate::projects::metadata(&r.project){Ok(m)=>w.metadata=serde_json::to_string_pretty(&m).unwrap_or_default(),Err(e)=>w.report=e.to_string()}}
                                if ui.button("Attach local reference").clicked() && let Some(path)=rfd::FileDialog::new().pick_file(){let p=r.project.clone();let label=path.file_name().unwrap_or_default().to_string_lossy().into_owned();w.task(ctx,move||Ok(Report::Text(serde_json::to_string_pretty(&crate::projects::attach_reference(&p,&path,&label,"background",&Control::default())?)?)));}
                            });
                            ui.label("Attached references are background context, separate from transcription evidence. They stay on this PC.");
                            if !w.metadata.is_empty(){ui.add(egui::TextEdit::multiline(&mut w.metadata).code_editor().desired_rows(12).desired_width(f32::INFINITY));if ui.add_enabled(self.worker.is_none(),egui::Button::new("Validate and save project details")).clicked(){match serde_json::from_str(&w.metadata).map_err(anyhow::Error::from).and_then(|m|crate::projects::save_metadata(&r.project,&m)){Ok(())=>w.report="Project details saved.".into(),Err(e)=>w.report=e.to_string()}}}
                        }else{ui.label("Open a project to inspect recovery or attach references.");}
                        self.region_reread_tools(&mut w,ui,ctx);
                    }
                    _=>{
                        ui.heading("Local activity and data");
                        ui.label(format!("Recognition endpoint: {}",self.state.settings.endpoint));
                        ui.label(format!("Models: {}",self.state.settings.models_dir.display()));
                        ui.label(format!("Writer examples: {}",self.state.settings.writer.root.display()));
                        ui.label(format!("Automatic update checks: {} · active check: {}",self.state.settings.experience.check_updates,self.experience.checking));
                        ui.label(format!("Optional public lookup: {}",self.state.settings.assistance.web_lookup));
                        ui.label("Recognition uses a literal loopback connection. Publisher download and update requests may use the internet. Public lookup sends the typed query only when enabled. This panel reports NoteGen settings and active tasks; it is not an OS network monitor.");
                        ui.separator();ui.heading("Readable layout");
                        ui.horizontal_wrapped(|ui|{
                            if ui.button("Large text").clicked(){self.state.settings.ui.scale=1.4;self.state.settings.ui.compact=false;}
                            if ui.button("Spacious").clicked(){self.state.settings.ui.scale=1.1;self.state.settings.ui.compact=false;}
                            if ui.button("Compact").clicked(){self.state.settings.ui.scale=1.;self.state.settings.ui.compact=true;}
                            if ui.button("High contrast").clicked(){self.state.settings.ui.dark=true;self.state.settings.ui.background=[0,0,0];self.state.settings.ui.accent=[255,255,0];self.state.settings.ui.reduced_motion=true;}
                            if ui.button("Default appearance").clicked(){let locale=self.state.settings.ui.locale;self.state.settings.ui=Appearance::default();self.state.settings.ui.locale=locale;}
                        });
                        ui.small("Ctrl+K opens the command palette. Tab moves focus, Enter activates buttons. Review includes source zoom and disagreement markers.");
                        self.resource_tools(&mut w,ui,ctx);
                        self.credential_tools(&mut w,ui,ctx);
                    }
                }
                if !w.report.is_empty(){ui.separator();ui.add(egui::TextEdit::multiline(&mut w.report).desired_rows(10).desired_width(f32::INFINITY));}
            });
        });
        w.open = open;
        if !open {
            w.clear_secret(ctx);
        }
        self.workbench = w;
        if let Some((path, target)) = jump
            && self.open_review(path, ctx)
            && let Some((page, block)) = target
            && let Some(r) = &mut self.review
            && let Some(index) = r.project.manifest.pages.iter().position(|n| *n == page)
        {
            match Review::load(r.project.clone(), index, ctx) {
                Ok(mut next) => {
                    next.selected = block;
                    self.review = Some(next);
                }
                Err(e) => self.notice = e.to_string(),
            }
        }
    }
}
