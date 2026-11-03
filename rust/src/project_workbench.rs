//! Background previews and explicit confirmation for private project transactions.
use super::*;
use crate::{migrations, project_layout as layout, project_reuse as reuse};

enum Preview {
    Migration(Box<migrations::Plan>),
    Layout(Box<Project>, Box<layout::LayoutPreview>),
    Reuse(Box<reuse::Preview>),
}
enum Reply {
    Preview(Preview),
    Catalog(PathBuf, Box<layout::Catalog>),
    Applied(PathBuf, String),
    Text(String),
}
#[derive(Default)]
pub(super) struct ProjectTools {
    project: Option<PathBuf>,
    action: usize,
    pages: String,
    image: Option<PathBuf>,
    position: usize,
    axis: usize,
    split: f32,
    preview: Option<Preview>,
    details: String,
    result: String,
    source: Option<PathBuf>,
    roots: Vec<PathBuf>,
    selected: usize,
    separate: bool,
    target: Option<PathBuf>,
    settings_file: Option<PathBuf>,
    receipt: Option<PathBuf>,
    reopen: Option<PathBuf>,
    revision: Option<(PathBuf, u64)>,
    rx: Option<mpsc::Receiver<std::result::Result<Reply, String>>>,
    control: Control,
}
impl Drop for ProjectTools {
    fn drop(&mut self) {
        self.control.cancelled.store(true, Ordering::Relaxed);
    }
}
impl ProjectTools {
    fn task(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce(Control) -> Result<Reply> + Send + 'static,
    ) {
        if self.rx.is_some() {
            return;
        }
        self.control = Control::default();
        let c = self.control.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(c)))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Project tool stopped unexpectedly; reopen the project to recover its journal")))
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        let result = self.rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "Project worker disconnected; reopen the project to recover its journal".into(),
            )),
            Err(_) => None,
        });
        if let Some(result) = result {
            self.rx = None;
            match result {
                Ok(Reply::Preview(preview)) => {
                    self.details = match &preview {
                        Preview::Migration(p) => serde_json::to_string_pretty(p),
                        Preview::Layout(_, p) => serde_json::to_string_pretty(p),
                        Preview::Reuse(p) => serde_json::to_string_pretty(p),
                    }
                    .unwrap_or_else(|e| e.to_string());
                    self.preview = Some(preview);
                    self.result = "Preview ready. Inspect the changes before confirming.".into();
                }
                Ok(Reply::Applied(path, text)) => {
                    self.preview = None;
                    self.revision = None;
                    self.reopen = Some(path);
                    self.result = text;
                }
                Ok(Reply::Text(text)) => {
                    self.preview = None;
                    self.result = text;
                }
                Ok(Reply::Catalog(path, catalog)) => {
                    self.revision = Some((path, catalog.states[catalog.cursor].revision));
                    self.details =
                        serde_json::to_string_pretty(&catalog).unwrap_or_else(|e| e.to_string());
                    self.result = "Current source order and layout revision loaded.".into();
                }
                Err(error) => {
                    self.preview = None;
                    self.result = error;
                }
            }
        }
    }
    fn operation(&self) -> Result<layout::Operation> {
        let pages = || -> Result<Vec<usize>> {
            anyhow::ensure!(self.pages.len() <= 20_000, "Page list is too long");
            self.pages
                .split([',', ' ', '\n'])
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<usize>().map_err(anyhow::Error::from))
                .collect()
        };
        let axis = if self.axis == 0 {
            layout::Axis::Vertical
        } else {
            layout::Axis::Horizontal
        };
        Ok(match self.action {
            0 => layout::Operation::Reorder { pages: pages()? },
            1 => layout::Operation::Remove { pages: pages()? },
            2 => layout::Operation::Insert {
                image: self
                    .image
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Choose an image"))?,
                position: self.position,
            },
            3 => {
                let pages = pages()?;
                anyhow::ensure!(pages.len() == 1, "Choose one page to split");
                layout::Operation::Split {
                    page: pages[0],
                    axis,
                    at: self.split,
                }
            }
            4 => layout::Operation::Merge {
                pages: pages()?,
                axis,
            },
            _ => anyhow::bail!("Choose a page organization action"),
        })
    }
    fn form_identity(&self) -> String {
        fingerprint(&(
            self.action,
            &self.pages,
            &self.image,
            self.position,
            self.axis,
            self.split.to_bits(),
            &self.source,
            &self.roots,
            &self.settings_file,
            &self.receipt,
        ))
        .unwrap_or_default()
    }
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        project: Option<Project>,
        settings: &Settings,
        recent: &[PathBuf],
        ready: bool,
    ) -> Option<PathBuf> {
        self.poll();
        let root = project.as_ref().map(|p| p.root.clone());
        if self.project != root && self.rx.is_none() {
            self.project = root;
            self.preview = None;
            self.details.clear();
            self.pages = project
                .as_ref()
                .map(|p| {
                    p.manifest
                        .pages
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            self.split = 0.5;
        }
        ui.heading("Organize and recover projects");
        ui.label("Preview changes, then confirm. Originals, archived pages and review history stay in the private project. Undo page organization separately from text corrections.");
        if !ready {
            ui.colored_label(Color32::LIGHT_YELLOW, "Save the draft, close page JSON and finish other work before changing project data.");
        }
        if self.rx.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Working on the selected private project…");
                if ui.button("Cancel project operation").clicked() {
                    self.control.cancelled.store(true, Ordering::Relaxed);
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let enabled = ready && self.rx.is_none();
        let form_before = self.form_identity();
        ui.add_enabled_ui(enabled, |ui| {
            let old = self.action;
            egui::ComboBox::from_label("Project action").selected_text(["Reorder pages", "Remove pages", "Insert image", "Split page", "Merge pages", "Enable stable source identities", "Find compatible duplicates", "Migrate settings", "Rollback migration"][self.action]).show_ui(ui, |ui| {
                for (n, label) in ["Reorder pages", "Remove pages", "Insert image", "Split page", "Merge pages", "Enable stable source identities", "Find compatible duplicates", "Migrate settings", "Rollback migration"].iter().enumerate() { ui.selectable_value(&mut self.action, n, *label); }
            });
            if old != self.action { self.preview = None; self.details.clear(); }
            match self.action {
                0..=4 => {
                    if self.action != 2 { ui.text_edit_singleline(&mut self.pages).accessible_name("Source page slots, separated by commas, in the selected order"); }
                    if self.action == 2 {
                        if ui.button("Choose image to insert").clicked() && let Some(image) = rfd::FileDialog::new().add_filter("Raster image", &["png", "jpg", "jpeg", "tif", "tiff", "bmp"]).pick_file() { self.image = Some(image); self.preview = None; }
                        if let Some(image) = &self.image { ui.label(image.display().to_string()); }
                        ui.add(egui::DragValue::new(&mut self.position).range(0..=3000).prefix("Insert position (0 = first): "));
                    }
                    if matches!(self.action, 3 | 4) {
                        ui.horizontal_wrapped(|ui| { ui.selectable_value(&mut self.axis, 0, if self.action == 3 {"Split left/right"} else {"Merge side by side"}); ui.selectable_value(&mut self.axis, 1, if self.action == 3 {"Split top/bottom"} else {"Merge vertically"}); });
                    }
                    if self.action == 3 { ui.add(egui::Slider::new(&mut self.split, 0.05..=0.95).text("Split position")); }
                }
                6 => {
                    if ui.button("Choose original input for duplicate matching").clicked() { self.source = rfd::FileDialog::new().pick_file(); self.preview = None; }
                    if let Some(source) = &self.source { ui.label(source.display().to_string()); }
                    if self.roots.is_empty() { self.roots = recent.to_vec(); }
                    ui.label(format!("{} selected project folders", self.roots.len()));
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Add project folder").clicked() && let Some(root) = rfd::FileDialog::new().pick_folder() && !self.roots.contains(&root) { self.roots.push(root); self.preview = None; }
                        if ui.button("Use recent projects").clicked() { self.roots = recent.to_vec(); self.preview = None; }
                        if ui.button("Clear selected folders").clicked() { self.roots.clear(); self.preview = None; }
                    });
                }
                7 => {
                    if ui.button("Choose settings file to migrate").clicked() { self.settings_file = rfd::FileDialog::new().add_filter("Settings", &["json"]).pick_file(); self.preview = None; }
                    if let Some(path) = &self.settings_file { ui.label(path.display().to_string()); }
                }
                8 => {
                    if ui.button("Choose migration receipt").clicked() { self.receipt = rfd::FileDialog::new().set_file_name("receipt.json").pick_file(); self.preview = None; }
                    if let Some(path) = &self.receipt { ui.label(path.display().to_string()); }
                }
                _ => {}
            }
            if ui.button("Preview selected project action").clicked() {
                self.preview = None;
                let action = self.action;
                let operation = if action <= 4 { Some(self.operation()) } else { None };
                let project = project.clone(); let source = self.source.clone(); let roots = self.roots.clone(); let settings = settings.clone(); let settings_file = self.settings_file.clone(); let receipt = self.receipt.clone();
                self.task(ctx, move |c| {
                    let preview = match action {
                        0..=4 => { let p = project.ok_or_else(|| anyhow::anyhow!("Open a project"))?; let preview = layout::preview(&p, operation.unwrap()?, &c)?; Preview::Layout(Box::new(p), Box::new(preview)) }
                        5 => Preview::Migration(Box::new(migrations::preview_project_sources(&project.ok_or_else(|| anyhow::anyhow!("Open a project"))?, &c)?)),
                        6 => Preview::Reuse(Box::new(reuse::preview(&source.ok_or_else(|| anyhow::anyhow!("Choose the original input"))?, &settings, &roots, &c)?)),
                        7 => Preview::Migration(Box::new(migrations::preview_settings(&settings_file.ok_or_else(|| anyhow::anyhow!("Choose a settings file"))?, &c)?)),
                        _ => Preview::Migration(Box::new(migrations::rollback_preview(&migrations::read_receipt(&receipt.ok_or_else(|| anyhow::anyhow!("Choose a migration receipt"))?, &c)?, &c)?)),
                    }; Ok(Reply::Preview(preview))
                });
            }
            if form_before != self.form_identity() { self.preview = None; self.details.clear(); }
            if let Some(preview) = self.preview.take() {
                let mut keep = true;
                if let Preview::Reuse(preview) = &preview {
                    for (n, candidate) in preview.candidates.iter().enumerate() { ui.radio_value(&mut self.selected, n, format!("{} · {}", candidate.root.display(), if candidate.compatible {"compatible"} else {"compare or copy only"})); }
                    ui.checkbox(&mut self.separate, "Create a separate private revision instead of reusing");
                    if self.separate && ui.button("Choose a new revision folder").clicked() { self.target = rfd::FileDialog::new().set_file_name("new-private-revision").save_file(); }
                    if let Some(path) = &self.target { ui.label(path.display().to_string()); }
                    if let Some(current) = &project && ui.button("Compare selected duplicate with open project").clicked() {
                        let other = preview.candidates.get(self.selected).map(|p| p.root.clone()); let current = current.clone();
                        self.task(ctx, move |c| Ok(Reply::Text(serde_json::to_string_pretty(&reuse::compare(&current, &Project::read(other.ok_or_else(|| anyhow::anyhow!("Choose a duplicate"))?)?, &c)?)?)));
                        keep = false;
                    }
                }
                if ui.add_enabled(keep, egui::Button::new("Confirm this exact preview")).clicked() {
                    let selected = self.selected; let target = self.target.clone(); let separate = self.separate;
                    self.task(ctx, move |c| match preview {
                        Preview::Layout(p, preview) => { let p = layout::apply(&p, &preview, &preview.token, &c)?; Ok(Reply::Applied(p.root, "Page organization saved. Originals and review history are preserved.".into())) }
                        Preview::Migration(preview) => { let receipt = migrations::apply(&preview, &preview.token, &c)?; Ok(Reply::Text(format!("Migration saved. Backup and rollback receipt: {}", receipt.backup.join("receipt.json").display()))) }
                        Preview::Reuse(preview) => { let p = reuse::choose(&preview, &preview.token, selected, if separate { reuse::Choice::SeparateRevision } else { reuse::Choice::Reuse }, target.as_deref(), &c)?; Ok(Reply::Applied(p.root, "Selected private project is ready to open.".into())) }
                    });
                } else if keep { self.preview = Some(preview); }
            }
            if let Some(p) = &project {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Inspect current page order and revision").clicked() { let p=p.clone(); self.task(ctx, move |_c| { let catalog=layout::catalog(&p)?.ok_or_else(||anyhow::anyhow!("Enable stable source identities first"))?; Ok(Reply::Catalog(p.root,Box::new(catalog))) }); }
                    for (redo, label) in [(false, "Undo page organization"), (true, "Redo page organization")] {
                        let revision=self.revision.as_ref().filter(|(root,_)|root==&p.root).map(|(_,revision)|*revision);
                        if ui.add_enabled(revision.is_some(),egui::Button::new(label)).clicked() { let p = p.clone(); let revision=revision.unwrap(); self.preview = None; self.revision=None; self.task(ctx, move |c| { let p = if redo { layout::redo(&p, revision, &c)? } else { layout::undo(&p, revision, &c)? }; Ok(Reply::Applied(p.root, format!("{label} saved."))) }); }
                    }
                });
            }
        });
        if !self.details.is_empty() {
            egui::CollapsingHeader::new("Inspect exact preview")
                .default_open(true)
                .show(ui, |ui| {
                    let mut details = self.details.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut details)
                            .desired_rows(12)
                            .desired_width(f32::INFINITY),
                    )
                    .accessible_name("Read-only project change preview");
                });
        }
        if !self.result.is_empty() {
            Desktop::accessible_status(ui, "project-transaction-result", &self.result, false);
            ui.label(&self.result);
        }
        if ready && self.rx.is_none() {
            self.reopen.take()
        } else {
            None
        }
    }
}
