//! Native desktop presentation. Recognition runs in a worker and never on the UI thread.
use crate::{
    hardware::{self, Hardware},
    motion,
    pipeline::{self, Completed},
    planning, runtime,
    storage::*,
    types::*,
    word,
};
use eframe::egui::{self, Color32, RichText, Vec2};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
    time::Instant,
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Complete,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub input: PathBuf,
    pub status: JobStatus,
    pub output: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub detail: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub settings: Settings,
    pub queue: Vec<Job>,
    pub output_dir: String,
    pub recent: Vec<PathBuf>,
}
impl Workspace {
    pub fn recover(&mut self) {
        for j in &mut self.queue {
            if j.status == JobStatus::Running {
                j.status = JobStatus::Pending;
                j.detail = "Interrupted session; ready to resume checkpoints".into();
            }
        }
    }
    pub fn enqueue(&mut self, path: PathBuf) -> bool {
        if !path
            .extension()
            .is_some_and(|p| p.eq_ignore_ascii_case("pdf"))
            || self.queue.iter().any(|j| {
                j.input == path && matches!(j.status, JobStatus::Pending | JobStatus::Running)
            })
        {
            return false;
        }
        self.queue.push(Job {
            input: path,
            status: JobStatus::Pending,
            output: None,
            project: None,
            detail: String::new(),
        });
        true
    }
    pub fn next(&self) -> Option<usize> {
        self.queue
            .iter()
            .position(|j| j.status == JobStatus::Pending)
    }
    pub fn apply(&mut self, raw: &str) -> Result<()> {
        let next: Settings = serde_json::from_str(raw)?;
        next.validate()?;
        self.settings = next;
        Ok(())
    }
}
pub fn output_name(input: &Path, folder: &Path) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let mut path = folder.join(format!("{stem}.docx"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{stem}-{n}.docx"));
        n += 1;
    }
    path
}
pub fn save_review(project: &Project, page: &mut Page, approved: bool) -> Result<()> {
    page.content.validate()?;
    page.edited = true;
    page.reviewed = approved;
    project.save(page)
}
fn evidence_label(page: &Page) -> String {
    let families = page
        .candidates
        .iter()
        .map(|c| c.family.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    match families {
        0 => "No reader evidence".into(),
        1 => "Single reader family · independent agreement unavailable".into(),
        _ => format!(
            "Agreement {:.1}% · {families} reader families · {} readings",
            page.agreement * 100.,
            page.candidates.len()
        ),
    }
}
enum Message {
    Event(Event),
    Complete(Box<Completed>),
    Failed(String, String),
    Hardware(Hardware, Vec<planning::Model>, Option<String>),
    Notice(String),
    Exported(PathBuf),
}
struct Review {
    project: Project,
    index: usize,
    page: Page,
    texture: Option<egui::TextureHandle>,
    selected: usize,
    dirty: bool,
    raw: String,
    raw_open: bool,
}
impl Review {
    fn load(project: Project, index: usize, ctx: &egui::Context) -> Result<Self> {
        let n = *project
            .manifest
            .pages
            .get(index)
            .ok_or_else(|| Fault::new("NG500", "Page does not exist"))?;
        let page = project.load(n)?;
        let texture = image::open(project.image(n)).ok().map(|im| {
            let limit = ctx.input(|i| i.max_texture_side).min(2048) as u32;
            let rgba = im
                .thumbnail(im.width().min(limit), im.height().min(limit))
                .to_rgba8();
            ctx.load_texture(
                format!("{}:{n}", project.root.display()),
                egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            )
        });
        let raw = serde_json::to_string_pretty(&page.content)?;
        Ok(Self {
            project,
            index,
            page,
            texture,
            selected: 0,
            dirty: false,
            raw,
            raw_open: false,
        })
    }
}
pub struct Desktop {
    pub state: Workspace,
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    worker: Option<thread::JoinHandle<()>>,
    control: Control,
    active: Option<usize>,
    running_queue: bool,
    pub hardware: Option<Hardware>,
    installed: Vec<planning::Model>,
    inventory_error: Option<String>,
    logs: VecDeque<Event>,
    progress: f32,
    stage: String,
    started: Option<Instant>,
    advanced: bool,
    tab: usize,
    draft: Settings,
    raw: String,
    settings_error: String,
    model_id: String,
    weights: String,
    projector: String,
    log_open: bool,
    log_filter: String,
    warnings_only: bool,
    pub notice: String,
    error_code: String,
    review: Option<Review>,
    review_open: bool,
    busy_setup: bool,
    review_error: String,
    opened_at: Option<f64>,
    completed_at: Option<Instant>,
}
impl Desktop {
    pub fn new(state: Workspace) -> Self {
        let (tx, rx) = mpsc::channel();
        let draft = state.settings.clone();
        let raw = serde_json::to_string_pretty(&draft).unwrap_or_default();
        Self {
            state,
            tx,
            rx,
            worker: None,
            control: Control::default(),
            active: None,
            running_queue: false,
            hardware: None,
            installed: vec![],
            inventory_error: None,
            logs: VecDeque::new(),
            progress: 0.,
            stage: "Ready when you are".into(),
            started: None,
            advanced: false,
            tab: 0,
            draft,
            raw,
            settings_error: String::new(),
            model_id: "glm-ocr".into(),
            weights: String::new(),
            projector: String::new(),
            log_open: false,
            log_filter: String::new(),
            warnings_only: false,
            notice: String::new(),
            error_code: String::new(),
            review: None,
            review_open: false,
            busy_setup: false,
            review_error: String::new(),
            opened_at: None,
            completed_at: None,
        }
    }
    pub fn from_context(cc: &eframe::CreationContext<'_>) -> Self {
        let mut state = cc
            .storage
            .and_then(|s| s.get_string("workspace"))
            .and_then(|s| serde_json::from_str::<Workspace>(&s).ok())
            .filter(|s| s.settings.validate().is_ok())
            .unwrap_or_default();
        state.recover();
        let mut app = Self::new(state);
        app.refresh(&cc.egui_ctx);
        app
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let s = self.state.settings.clone();
        thread::spawn(move || {
            let h = hardware::detect_for(&s, &Control::default());
            let (models, error) = match runtime::inventory(&s.models_dir) {
                Ok(m) => (m, None),
                Err(e) => (vec![], Some(e.to_string())),
            };
            let _ = tx.send(Message::Hardware(h, models, error));
            ctx.request_repaint();
        });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        for _ in 0..500 {
            let Ok(msg) = self.rx.try_recv() else { break };
            match msg {
                Message::Event(e) => {
                    if e.total > 0 {
                        self.progress = (e.done as f32 / e.total as f32).clamp(0., 1.);
                    }
                    self.stage = e.message.clone();
                    self.logs.push_back(e);
                    if self.logs.len() > 2000 {
                        self.logs.pop_front();
                    }
                }
                Message::Hardware(h, m, e) => {
                    self.hardware = Some(h);
                    self.installed = m;
                    self.inventory_error = e;
                }
                Message::Complete(r) => {
                    if let Some(i) = self.active.take() {
                        let j = &mut self.state.queue[i];
                        j.status = JobStatus::Complete;
                        j.output = Some(r.output.clone());
                        j.project = Some(r.project.root.clone());
                        j.detail = format!(
                            "{} pages · {} to review · {:.1}s",
                            r.pages, r.review, r.seconds
                        );
                    }
                    self.notice = format!(
                        "Saved {} · {} pages need review",
                        r.output.display(),
                        r.review
                    );
                    self.progress = 1.;
                    self.completed_at = Some(Instant::now());
                    self.stage = "Conversion complete".into();
                    self.state.recent.retain(|p| p != &r.project.root);
                    self.state.recent.insert(0, r.project.root.clone());
                    self.state.recent.truncate(12);
                    if self.flush_review().is_ok()
                        && let Ok(v) = Review::load(r.project, 0, ctx)
                    {
                        self.review = Some(v);
                    }
                    self.finished();
                }
                Message::Failed(code, detail) => {
                    let cancelled = code == "NG900";
                    if let Some(i) = self.active.take() {
                        self.state.queue[i].status = if cancelled {
                            JobStatus::Cancelled
                        } else {
                            JobStatus::Failed
                        };
                        self.state.queue[i].detail = detail.clone();
                    }
                    self.notice = detail;
                    self.error_code = code;
                    self.stage = if cancelled {
                        "Cancelled · checkpoints preserved"
                    } else {
                        "Needs attention"
                    }
                    .into();
                    self.running_queue = false;
                    self.finished();
                }
                Message::Notice(s) => {
                    self.notice = s;
                    self.busy_setup = false;
                    self.finished();
                    self.refresh(ctx);
                }
                Message::Exported(p) => {
                    self.notice = format!("Corrected Word document saved: {}", p.display());
                    self.finished();
                }
            }
        }
        if self.running_queue && self.worker.is_none() {
            self.start_next(ctx);
        }
    }
    fn finished(&mut self) {
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        self.busy_setup = false;
        self.started = None;
    }
    fn start_next(&mut self, ctx: &egui::Context) {
        if self.worker.is_some() {
            return;
        }
        if let Err(e) = self.flush_review() {
            self.notice = e.to_string();
            self.running_queue = false;
            return;
        }
        let Some(i) = self.state.next() else {
            self.running_queue = false;
            return;
        };
        if let Err(e) = self.state.settings.validate() {
            self.notice = e.to_string();
            self.running_queue = false;
            return;
        }
        let input = self.state.queue[i].input.clone();
        let folder = if self.state.output_dir.trim().is_empty() {
            input.parent().unwrap_or(Path::new(".")).to_path_buf()
        } else {
            PathBuf::from(&self.state.output_dir)
        };
        if let Err(e) = fs::create_dir_all(&folder) {
            self.notice = format!("NG400: {e}");
            self.error_code = "NG400".into();
            self.stage = "Needs attention".into();
            self.state.queue[i].status = JobStatus::Failed;
            self.state.queue[i].detail = self.notice.clone();
            self.running_queue = false;
            return;
        }
        let output = output_name(&input, &folder);
        let s = self.state.settings.clone();
        self.control = Control::default();
        let c = self.control.clone();
        let tx = self.tx.clone();
        let repaint = ctx.clone();
        self.state.queue[i].status = JobStatus::Running;
        self.active = Some(i);
        self.progress = 0.;
        self.stage = "Planning this PC".into();
        self.started = Some(Instant::now());
        self.error_code.clear();
        self.notice.clear();
        self.worker = Some(thread::spawn(move || {
            let events = tx.clone();
            let cx = repaint.clone();
            let log = Activity {
                path: None,
                callback: Some(Arc::new(move |e| {
                    let _ = events.send(Message::Event(e));
                    cx.request_repaint();
                })),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pipeline::convert(&input, &output, &s, &c, &log)
            }));
            let msg=match result {
                Ok(Ok(r))=>Message::Complete(Box::new(r)),Ok(Err(e))=>failure(e),Err(_)=>Message::Failed("NG500".into(),"Worker stopped unexpectedly. Saved checkpoints remain available; inspect the activity log.".into())}
            ;
            let _ = tx.send(msg);
            repaint.request_repaint();
        }));
    }
    fn flush_review(&mut self) -> Result<()> {
        if let Some(r) = &mut self.review
            && r.dirty
        {
            save_review(&r.project, &mut r.page, false)?;
            r.dirty = false;
        }
        Ok(())
    }
    pub fn open_review(&mut self, path: PathBuf, ctx: &egui::Context) {
        if let Err(e) = self.flush_review() {
            self.notice = e.to_string();
            return;
        }
        match Project::read(path).and_then(|p| Review::load(p, 0, ctx)) {
            Ok(r) => {
                self.review = Some(r);
                self.review_open = true;
            }
            Err(e) => self.notice = e.to_string(),
        }
    }
    pub fn advanced_open(&mut self) {
        self.draft = self.state.settings.clone();
        self.raw = serde_json::to_string_pretty(&self.draft).unwrap_or_default();
        self.settings_error.clear();
        self.advanced = true;
    }
    pub fn render(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.opened_at.get_or_insert(now);
        self.poll(&ctx);
        self.theme(&ctx);
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for f in files {
            self.state.enqueue(f.path().to_path_buf());
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.pick_files();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.advanced = false;
            match self.flush_review() {
                Ok(()) => self.review_open = false,
                Err(e) => self.review_error = e.to_string(),
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && let Err(e) = self.flush_review()
        {
            self.notice = format!("Could not save review draft: {e}");
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(self.background()).inner_margin(if self.state.settings.ui.compact {
            18}
        else{
            30}
        )).show(ui,|ui|{
            self.background_art(ui);
            ui.horizontal(|ui|{
                self.logo(ui);
                ui.add_space(9.);
                ui.label(RichText::new("NoteGen").size(23.).strong());
                ui.label(RichText::new("LOCAL STUDIO").size(10.).color(self.muted()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    if ui.button("Advanced").clicked(){
                        self.advanced_open();
                    }
                    if ui.button("Open project").clicked() && let Some(p)=rfd::FileDialog::new().set_title("Open NoteGen project.json").add_filter("NoteGen project",&["json"]).pick_file(){
                        self.open_review(p.parent().unwrap_or(Path::new(".")).into(),&ctx);
                    }
                }
                );
            }
            );
            ui.add_space(30.);
            egui::ScrollArea::vertical().id_salt("workspace").show(ui,|ui|{
                ui.label(RichText::new("From scan to sense.").size(34.).strong());
                ui.add_space(8.);
                ui.label(RichText::new("Editable text. Rebuilt tables. Diagrams with context.").size(15.).color(self.muted()));
                ui.add_space(24.);
                self.input_card(ui);
                ui.add_space(18.);
                ui.horizontal_wrapped(|ui|{
                    for (preset,title,help) in [(Preset::Fast,"Fast","Printed OCR, no AI"),(Preset::Balanced,"Balanced","Local OCR and vision readers"),(Preset::Accuracy,"Accuracy","Strongest fitting OCR and vision models"),(Preset::Maximum,"Maximum accuracy","Independent readers plus difficult-region retries; slower")] {
                        let amount=ui.ctx().animate_bool_with_time(egui::Id::unique(format!("preset-{preset:?}")),self.state.settings.preset==preset,motion::transition(&self.state.settings.ui));
                        let fill=self.surface().lerp_to_gamma(self.accent(),0.20*amount);
                        let allowed=self.worker.is_none() && !(self.state.settings.handwriting && preset==Preset::Fast);
                        let response=ui.add_enabled(allowed,egui::Button::new(title).selected(self.state.settings.preset==preset).fill(fill).min_size(Vec2::new(118.,38.))).on_hover_text(help).on_disabled_hover_text("Fast is printed OCR only. Disable handwriting to use Fast; settings are locked during conversion.");
                        self.hover_ring(ui,&response,&format!("hover-{preset:?}"));
                        if response.clicked() && self.worker.is_none(){self.state.settings.preset=preset;ctx.request_repaint();}
                    }
                }
                );
                ui.add_space(10.);
                ui.horizontal_wrapped(|ui|{
                    let changed=ui.add_enabled(self.worker.is_none(),egui::Checkbox::new(&mut self.state.settings.handwriting,"Handwritten / difficult scan")).changed();
                    if changed && self.state.settings.handwriting && self.state.settings.preset==Preset::Fast {
                        self.state.settings.preset=Preset::Accuracy;
                    }
                    ui.add_space(12.);
                    ui.label("Pages");
                    ui.add_enabled(self.worker.is_none(),egui::TextEdit::singleline(&mut self.state.settings.page_range).hint_text("All · e.g. 1-4, 8").desired_width(145.));
                }
                );
                ui.add_space(20.);
                ui.horizontal(|ui|{
                    let ready=self.state.next().is_some() && self.worker.is_none();
                    let convert=ui.add_enabled(ready,egui::Button::new(RichText::new("Convert to Word   →").strong().color(self.accent_text())).fill(self.accent()).min_size(Vec2::new(220.,48.)));
                    self.hover_ring(ui,&convert,"convert-hover");
                    if convert.clicked(){
                        self.running_queue=true;
                        self.start_next(&ctx);
                    }
                    if self.active.is_some(){
                        let paused=self.paused();
                        if ui.button(if paused{
                            "Resume"}
                        else{
                            "Pause"}
                        ).clicked(){
                            self.control.paused.store(!paused,Ordering::Relaxed);
                        }
                        if ui.button("Cancel").clicked(){
                            self.running_queue=false;
                            self.control.cancelled.store(true,Ordering::Relaxed);
                        }
                    }
                    if self.review.is_some() && ui.button("Review results").clicked(){
                        self.review_open=true;
                    }
                }
                );
                ui.add_space(17.);
                self.progress_ui(ui);
                if !self.notice.is_empty(){
                    egui::Frame::new().fill(if self.error_code.is_empty(){
                        self.surface()}
                    else{
                        Color32::from_rgb(65,35,31)}
                    ).inner_margin(12).corner_radius(10).show(ui,|ui|{
                        ui.label(&self.notice);
                        if !self.error_code.is_empty(){
                            ui.small(Fault::new(code_static(&self.error_code),"").remedy());
                        }
                    }
                    );
                }
                ui.add_space(14.);
                self.hardware_ui(ui);
                ui.add_space(14.);
                self.queue_ui(ui);
                if !self.state.recent.is_empty(){
                    egui::CollapsingHeader::new("Recent projects").show(ui,|ui|{
                        let paths=self.state.recent.clone();
                        for p in paths {
                            if ui.link(p.file_name().unwrap_or_default().to_string_lossy()).clicked(){
                                self.open_review(p,&ctx);
                            }
                        }
                    }
                    );
                }
                self.log_ui(ui);
                ui.add_space(12.);
                ui.small(RichText::new("Runs on this PC. Source scans and alternative readings stay available for review.").color(self.muted()));
            }
            );
        }
        );
        self.advanced_ui(&ctx);
        self.review_ui(&ctx);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if let Some(delay) = motion::repaint_delay(
            &self.state.settings.ui,
            focused,
            self.worker.is_some(),
            self.paused(),
        ) {
            ctx.request_repaint_after(delay);
        }
    }
    fn pick_files(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Scanned PDF", &["pdf"])
            .pick_files()
        {
            for p in paths {
                self.state.enqueue(p);
            }
        }
    }
    fn input_card(&mut self, ui: &mut egui::Ui) {
        let hovered = !ui.ctx().input(|i| i.raw.hovered_files.is_empty());
        egui::Frame::new()
            .fill(self.surface())
            .stroke(egui::Stroke::new(
                1.,
                if hovered {
                    self.accent()
                } else {
                    self.border()
                },
            ))
            .corner_radius(self.state.settings.ui.rounding)
            .inner_margin(22)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Drop scanned PDFs here").size(19.).strong());
                        ui.add_space(6.);
                        ui.label(
                            RichText::new(
                                "Multiple files welcome. Completed pages save automatically.",
                            )
                            .color(self.muted()),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized([125., 40.], egui::Button::new("Choose PDFs"))
                            .clicked()
                        {
                            self.pick_files();
                        }
                    });
                });
                ui.add_space(16.);
                ui.horizontal(|ui| {
                    ui.small("SAVE TO");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.state.output_dir)
                            .hint_text("Same folder as each PDF")
                            .desired_width((ui.available_width() - 85.).max(100.)),
                    );
                    if ui.button("Browse").clicked()
                        && let Some(p) = rfd::FileDialog::new().pick_folder()
                    {
                        self.state.output_dir = p.display().to_string();
                    }
                });
            });
    }
    fn progress_ui(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if self.paused() {
                    "Pause requested · waiting at a safe checkpoint"
                } else {
                    &self.stage
                })
                .strong(),
            );
            if let Some(t) = self.started {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let sec = t.elapsed().as_secs();
                    ui.small(format!("{:02}:{:02} elapsed", sec / 60, sec % 60));
                    if self.progress > 0.05 && self.progress < 1. && !self.paused() {
                        ui.small(format!(
                            "~{}s remaining",
                            (t.elapsed().as_secs_f32() * (1. - self.progress) / self.progress)
                                as u64
                        ));
                    }
                });
            }
        });
        let value = ui.ctx().animate_value_with_time(
            egui::Id::unique("progress"),
            self.progress,
            motion::transition(&self.state.settings.ui),
        );
        let bar = ui.add(
            egui::ProgressBar::new(value)
                .desired_height(6.)
                .fill(self.accent())
                .animate(
                    self.worker.is_some()
                        && motion::enabled(&self.state.settings.ui)
                        && !self.paused(),
                ),
        );
        if self.active.is_some() && motion::enabled(&self.state.settings.ui) && !self.paused() {
            let phase = self.scene(ui).scan;
            let x = egui::lerp(bar.rect.x_range(), phase);
            ui.painter().with_clip_rect(bar.rect).rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(x, bar.rect.center().y),
                    Vec2::new(60., bar.rect.height()),
                ),
                3,
                Color32::from_white_alpha(40),
            );
        }
        if self.active.is_some() {
            ui.small(RichText::new("Pause takes effect at safe checkpoints. Cancel also stops an active model request.").color(self.muted()));
        }
    }
    fn hardware_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(self.surface()).corner_radius(10).inner_margin(12).show(ui,|ui|{
            if let Some(h)=&self.hardware {
                ui.horizontal_wrapped(|ui|{
                    ui.colored_label(self.accent(),"●");
                    ui.strong(format!("{} CPU threads",h.logical_cores));
                    ui.label(format!("{:.1}/{:.1} GB RAM available",h.free_ram_mib as f64/1024.,h.ram_mib as f64/1024.));
                    for g in &h.gpus {
                        ui.label(format!("{} · {:.1}/{:.1} GB VRAM · {}",g.name,g.free_mib as f64/1024.,g.total_mib as f64/1024.,g.backend.to_uppercase()));
                    }
                    if h.gpus.is_empty(){
                        ui.small("CPU mode · install a compatible GPU runtime for offload");
                    }
                }
                );
                if let Some(e)=&self.inventory_error {
                    ui.colored_label(Color32::from_rgb(241,187,111),e);
                }
                match planning::build(h,&self.state.settings,&self.installed){
                    Ok(p)=>{
                        ui.small(format!("Planned: {} · {} · {} DPI · context {}",if p.models.is_empty(){
                            "printed OCR".into()}
                        else{
                            p.models.iter().map(|m|m.name.as_str()).collect::<Vec<_>>().join(" + ")}
                        ,format_args!("{:?}",p.backend),p.dpi,p.context));
                        if p.models.is_empty() && self.state.settings.preset!=Preset::Fast {
                            ui.colored_label(Color32::from_rgb(241,187,111),"AI setup needed → Advanced / Models. Fast works with Tesseract.");
                        }
                    }
                    ,Err(e)=>{
                        ui.colored_label(Color32::from_rgb(241,187,111),e.to_string());
                    }
                }
            }
            else {
                ui.horizontal(|ui|{
                    self.activity_indicator(ui);
                    ui.label("Detecting CPU, memory and GPU runtime…");
                }
                );
            }
        }
        );
    }
    fn queue_ui(&mut self, ui: &mut egui::Ui) {
        if self.state.queue.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            ui.strong(format!("Queue · {} files", self.state.queue.len()));
            if self.worker.is_none() && ui.small_button("Clear finished").clicked() {
                self.state
                    .queue
                    .retain(|j| matches!(j.status, JobStatus::Pending | JobStatus::Running));
            }
        });
        let mut review = None;
        let mut remove = None;
        for (i, j) in self.state.queue.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    let icon = match j.status {
                        JobStatus::Pending => "○",
                        JobStatus::Running => "◌",
                        JobStatus::Complete => "✓",
                        JobStatus::Failed => "!",
                        JobStatus::Cancelled => "−",
                    };
                    ui.label(icon);
                    ui.label(j.input.file_name().unwrap_or_default().to_string_lossy())
                        .on_hover_text(j.input.display().to_string());
                    if !j.detail.is_empty() {
                        ui.small(&j.detail);
                    }
                    if let Some(p) = &j.project
                        && ui.small_button("Review").clicked()
                    {
                        review = Some(p.clone());
                    }
                    if let Some(p) = &j.output
                        && ui.small_button("Open Word").clicked()
                    {
                        open_path(p);
                    }
                    if matches!(j.status, JobStatus::Failed | JobStatus::Cancelled)
                        && ui.small_button("Retry").clicked()
                    {
                        j.status = JobStatus::Pending;
                    }
                    if self.active.is_none() && ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            self.state.queue.remove(i);
        }
        if let Some(p) = review {
            self.open_review(p, ui.ctx());
        }
    }
    fn log_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.);
        ui.horizontal(|ui| {
            ui.toggle_value(
                &mut self.log_open,
                format!("Live activity  ·  {} events", self.logs.len()),
            );
            if self.log_open {
                ui.checkbox(&mut self.warnings_only, "Warnings only");
                ui.add(
                    egui::TextEdit::singleline(&mut self.log_filter)
                        .hint_text("Filter stage, code, message")
                        .desired_width(180.),
                );
                if ui.small_button("Save log").clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .set_file_name("notegen-activity.jsonl")
                        .save_file()
                {
                    let raw = self
                        .logs
                        .iter()
                        .filter_map(|e| serde_json::to_string(e).ok())
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.notice = match fs::write(p, raw) {
                        Ok(_) => {
                            "Visible activity saved. Complete logs are also stored in each project."
                                .into()
                        }
                        Err(e) => e.to_string(),
                    };
                }
            }
        });
        let mut drawer = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id("activity-drawer"),
            false,
        );
        drawer.set_open(self.log_open);
        drawer.show_body_unindented(ui, |ui| {
            egui::Frame::new()
                .fill(self.surface())
                .inner_margin(10)
                .corner_radius(10)
                .show(ui, |ui| {
                    let filter = self.log_filter.to_lowercase();
                    let events = self
                        .logs
                        .iter()
                        .filter(|e| {
                            (!self.warnings_only || e.warning)
                                && (filter.is_empty()
                                    || format!("{} {} {}", e.stage, e.code, e.message)
                                        .to_lowercase()
                                        .contains(&filter))
                        })
                        .collect::<Vec<_>>();
                    egui::ScrollArea::vertical()
                        .id_salt("logs")
                        .max_height(210.)
                        .stick_to_bottom(true)
                        .show_rows(ui, 15., events.len(), |ui, range| {
                            for e in &events[range] {
                                let line = format!(
                                    "{}  {:12}  p{}  {} {}",
                                    e.timestamp.get(11..19).unwrap_or(""),
                                    e.stage,
                                    e.page,
                                    e.code,
                                    e.message
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&line).monospace().size(11.).color(
                                            if e.warning {
                                                Color32::from_rgb(241, 187, 111)
                                            } else {
                                                self.muted()
                                            },
                                        ),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(line);
                            }
                        });
                });
        });
    }

    fn advanced_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.advanced;
        egui::Window::new("Advanced").fade_in(motion::enabled(&self.state.settings.ui)).fade_out(motion::enabled(&self.state.settings.ui)).open(&mut open).default_size([760.,650.]).resizable(true).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{
                for(i,t)in["Compute","Recognition","Models","Appearance","All parameters"].iter().enumerate(){
                    if ui.selectable_label(self.tab==i,*t).clicked(){
                        if self.tab==4 {
                            match serde_json::from_str::<Settings>(&self.raw).map_err(anyhow::Error::from).and_then(|s|{
                                s.validate()?;
                                Ok(s)}
                            ) {
                                Ok(s)=>self.draft=s,Err(e)=>{
                                    self.settings_error=e.to_string();
                                    continue;
                                }
                            }
                        }
                        if i==4{
                            self.raw=serde_json::to_string_pretty(&self.draft).unwrap_or_default();
                        }
                        self.tab=i;
                    }
                }
            }
            );
            ui.separator();
            egui::ScrollArea::vertical().id_salt("advanced").max_height(510.).show(ui,|ui|{
                match self.tab{
                    0=>self.compute_controls(ui),1=>self.recognition_controls(ui),2=>self.model_controls(ui,ctx),3=>self.appearance_controls(ui),_=>{
                        ui.label("Every parameter is available here. Values are validated before applying; unknown fields are rejected.");
                        ui.add(egui::TextEdit::multiline(&mut self.raw).code_editor().desired_width(f32::INFINITY).desired_rows(25));
                    }
                }
            }
            );
            ui.separator();
            if !self.settings_error.is_empty(){
                ui.colored_label(Color32::from_rgb(244,161,140),&self.settings_error);
            }
            ui.horizontal(|ui|{
                if ui.add_enabled(self.worker.is_none(),egui::Button::new("Apply settings")).clicked(){
                    let next=if self.tab==4{
                        serde_json::from_str::<Settings>(&self.raw).map_err(anyhow::Error::from)}
                    else{
                        Ok(self.draft.clone())}
                    ;
                    match next.and_then(|s|{
                        s.validate()?;
                        Ok(s)}
                    ){
                        Ok(s)=>{
                            self.state.settings=s.clone();
                            self.draft=s;
                            self.settings_error.clear();
                            self.notice="Settings applied. Adaptive values use 0; current running jobs keep their original settings.".into();
                            self.refresh(ctx);
                        }
                        ,Err(e)=>self.settings_error=e.to_string()}
                }
                if ui.button("Reset draft").clicked(){
                    self.draft=Settings::default();
                    self.raw=serde_json::to_string_pretty(&self.draft).unwrap_or_default();
                }
                if ui.button("Export settings").clicked() && let Some(p)=rfd::FileDialog::new().set_file_name("notegen-settings.json").save_file(){
                    let value=if self.tab==4{
                        serde_json::from_str::<Settings>(&self.raw).map_err(anyhow::Error::from)}
                    else{
                        Ok(self.draft.clone())}
                    ;
                    match value.and_then(|s|{
                        s.validate()?;
                        atomic_json(&p,&s)}
                    ){
                        Ok(_)=>self.notice="Settings exported".into(),Err(e)=>self.settings_error=e.to_string()}
                }
                if ui.button("Import settings").clicked() && let Some(p)=rfd::FileDialog::new().add_filter("JSON",&["json"]).pick_file(){
                    match fs::read_to_string(p).map_err(anyhow::Error::from).and_then(|v|{
                        let s:Settings=serde_json::from_str(&v)?;
                        s.validate()?;
                        Ok(s)}
                    ){
                        Ok(s)=>{
                            self.draft=s;
                            self.raw=serde_json::to_string_pretty(&self.draft).unwrap_or_default();
                        }
                        ,Err(e)=>self.settings_error=e.to_string()}
                }
            }
            );
            if self.worker.is_some(){
                ui.small("Settings apply after the active task finishes.");
            }
        }
        );
        self.advanced = open;
    }
    fn compute_controls(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.draft;
        ui.label("0 means adaptive. GPU layers −1 means full offload when the model fits. Memory reserves keep Windows and the interface responsive.");
        egui::ComboBox::from_label("Backend")
            .selected_text(format!("{:?}", s.backend))
            .show_ui(ui, |ui| {
                for b in [Backend::Auto, Backend::Cuda, Backend::Vulkan, Backend::Cpu] {
                    ui.selectable_value(&mut s.backend, b, format!("{b:?}"));
                }
            });
        ui.horizontal(|ui| {
            let mut manual = s.gpu_index.is_some();
            if ui.checkbox(&mut manual, "Choose GPU index").changed() {
                s.gpu_index = if manual { Some(0) } else { None };
            }
            if let Some(i) = &mut s.gpu_index {
                ui.add(egui::DragValue::new(i).range(0..=16));
            }
        });
        egui::Grid::new("compute-grid")
            .num_columns(2)
            .spacing([25., 9.])
            .show(ui, |ui| {
                numeric(ui, "CPU threads", &mut s.cpu_threads, 0..=512);
                numeric(ui, "GPU layers", &mut s.gpu_layers, -1..=999);
                numeric(ui, "Context tokens", &mut s.context, 0..=131072);
                numeric(ui, "Image tokens", &mut s.image_tokens, 0..=16384);
                numeric(ui, "Batch", &mut s.batch, 0..=8192);
                numeric(ui, "Micro batch", &mut s.micro_batch, 0..=2048);
                numeric(ui, "Output token limit", &mut s.max_tokens, 256..=32768);
                numeric(ui, "RAM reserve MiB", &mut s.reserve_ram_mib, 256..=65536);
                numeric(ui, "VRAM reserve MiB", &mut s.reserve_vram_mib, 128..=16384);
                numeric(
                    ui,
                    "Request timeout s",
                    &mut s.request_timeout_secs,
                    5..=900,
                );
            });
        ui.add(egui::Slider::new(&mut s.ram_fraction, 0.1..=0.98).text("RAM fraction"));
        ui.add(egui::Slider::new(&mut s.vram_fraction, 0.1..=0.98).text("VRAM fraction"));
        ui.checkbox(&mut s.flash_attention, "Flash Attention");
        ui.checkbox(&mut s.projector_on_cpu, "Projector on CPU");
        ui.checkbox(
            &mut s.allow_cpu_offload,
            "Allow model larger than VRAM with CPU offload",
        );
        ui.checkbox(
            &mut s.require_acceleration,
            "Require verified GPU layer allocation",
        );
        egui::ComboBox::from_label("KV cache")
            .selected_text(&s.kv_cache)
            .show_ui(ui, |ui| {
                for t in ["f16", "q8_0", "q4_0"] {
                    ui.selectable_value(&mut s.kv_cache, t.into(), t);
                }
            });
    }
    fn recognition_controls(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.draft;
        egui::Grid::new("recognition-grid")
            .num_columns(2)
            .spacing([25., 9.])
            .show(ui, |ui| {
                numeric(ui, "Render DPI (0 adaptive)", &mut s.dpi, 0..=600);
                numeric(
                    ui,
                    "Maximum pixels",
                    &mut s.max_pixels,
                    1_000_000..=40_000_000,
                );
                numeric(ui, "Image edge (0 adaptive)", &mut s.image_edge, 0..=4096);
                numeric(ui, "Crop edge", &mut s.tile_edge, 512..=4096);
                numeric(ui, "Crop retries per reader", &mut s.crop_retries, 0..=64);
                numeric(ui, "Independent readers", &mut s.consensus_min, 2..=8);
                numeric(ui, "Seed", &mut s.seed, 0..=u64::MAX);
            });
        ui.add(egui::Slider::new(&mut s.tile_overlap, 0.0..=0.4).text("Tile overlap"));
        ui.add(
            egui::Slider::new(&mut s.agreement_threshold, 0.5..=1.0).text("Agreement threshold"),
        );
        ui.add(egui::Slider::new(&mut s.review_threshold, 0.0..=1.0).text("Review threshold"));
        ui.add(egui::Slider::new(&mut s.temperature, 0.0..=1.5).text("Temperature"));
        ui.add(egui::Slider::new(&mut s.top_p, 0.0..=1.0).text("Top p"));
        ui.horizontal(|ui| {
            ui.label("OCR languages");
            ui.text_edit_singleline(&mut s.languages);
        });
        ui.label("Document context (helps diagram interpretation; keep factual)");
        ui.add(
            egui::TextEdit::multiline(&mut s.subject)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        ui.checkbox(&mut s.deskew, "Deskew tilted scans");
        ui.checkbox(&mut s.auto_rotate, "Detect page orientation");
        ui.checkbox(&mut s.preserve_figures, "Embed source figures in Word");
        ui.checkbox(&mut s.keep_page_breaks, "Keep source page breaks");
        ui.horizontal(|ui| {
            ui.label("Word font");
            ui.text_edit_singleline(&mut s.font);
            ui.add(egui::DragValue::new(&mut s.font_size).range(6.0..=32.0));
        });
    }
    fn model_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label("Model weights stay on this PC. Download compatible GGUF weights and their matching projector from the catalog source. Registering verifies files and records SHA256.");
        path_control(ui, "llama-server", &mut self.draft.server, false);
        path_control(ui, "PDFium DLL/folder", &mut self.draft.pdfium, true);
        path_control(ui, "Tesseract", &mut self.draft.tesseract, false);
        path_control(ui, "Model registry", &mut self.draft.models_dir, true);
        ui.checkbox(
            &mut self.draft.external_server,
            "Use existing local server (advanced; model identity must match)",
        );
        if self.draft.external_server {
            ui.text_edit_singleline(&mut self.draft.endpoint);
            ui.small(
                "Only literal loopback HTTP is allowed. External mode cannot verify GPU offload.",
            );
        }
        ui.separator();
        ui.checkbox(
            &mut self.draft.automatic_models,
            "Automatically choose the strongest fitting installed models",
        );
        let catalog = planning::catalog();
        if !self.draft.automatic_models {
            ui.small("Allow these readers. Memory admission still applies.");
            for m in &catalog {
                let mut selected = self.draft.model_ids.contains(&m.id);
                if ui.checkbox(&mut selected, &m.name).changed() {
                    self.draft.model_ids.retain(|v| v != &m.id);
                    if selected {
                        self.draft.model_ids.push(m.id.clone());
                    }
                }
            }
        }
        egui::ComboBox::from_label("Register model")
            .selected_text(
                catalog
                    .iter()
                    .find(|m| m.id == self.model_id)
                    .map(|m| m.name.as_str())
                    .unwrap_or("Select model"),
            )
            .show_ui(ui, |ui| {
                for m in &catalog {
                    ui.selectable_value(&mut self.model_id, m.id.clone(), &m.name);
                }
            });
        if let Some(m) = catalog.iter().find(|m| m.id == self.model_id) {
            ui.hyperlink_to(
                format!(
                    "Official source · {} · estimated {} MiB resident",
                    m.released, m.resident_mib
                ),
                &m.source,
            );
        }
        file_string(ui, "Weights GGUF", &mut self.weights);
        file_string(ui, "Projector GGUF", &mut self.projector);
        if ui
            .add_enabled(
                self.worker.is_none(),
                egui::Button::new("Verify and register local model"),
            )
            .clicked()
        {
            let (dir, id, w, p) = (
                self.draft.models_dir.clone(),
                self.model_id.clone(),
                PathBuf::from(&self.weights),
                PathBuf::from(&self.projector),
            );
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            self.busy_setup = true;
            self.worker = Some(thread::spawn(move || {
                let msg = match runtime::register(&dir, &id, &w, &p) {
                    Ok(m) => Message::Notice(format!(
                        "Registered {}. Apply settings to use this registry.",
                        m.name
                    )),
                    Err(e) => failure(e),
                };
                let _ = tx.send(msg);
                ctx.request_repaint();
            }));
        }
        if self.busy_setup {
            ui.horizontal(|ui| {
                self.activity_indicator(ui);
                ui.label("Verifying local files and computing checksums…");
            });
        }
        {
            let models = &self.installed;
            ui.separator();
            ui.strong(format!("{} registered models", models.len()));
            for m in models {
                ui.label(format!(
                    "{} · {} MiB files · {}",
                    m.name,
                    m.weights_mib,
                    m.weights.display()
                ));
            }
        }
        if ui.button("Detect hardware again").clicked() {
            self.refresh(ctx);
        }
        if let Some(h) = &self.hardware
            && let Ok(p) = planning::build(h, &self.draft, &catalog)
        {
            ui.label("Catalog recommendation (models must be installed first)");
            for m in p.models {
                ui.label(format!("• {}", m.name));
            }
        }
    }
    fn appearance_controls(&mut self, ui: &mut egui::Ui) {
        let a = &mut self.draft.ui;
        ui.checkbox(&mut a.dark, "Dark appearance");
        ui.horizontal(|ui| {
            ui.label("Accent");
            ui.color_edit_button_srgb(&mut a.accent);
            ui.label("Background");
            ui.color_edit_button_srgb(&mut a.background);
        });
        ui.add(egui::Slider::new(&mut a.scale, 0.7..=2.0).text("Interface scale"));
        ui.add(egui::Slider::new(&mut a.motion, 0.0..=3.0).text("Motion speed"));
        ui.add(egui::Slider::new(&mut a.rounding, 0..=30).text("Corner rounding"));
        ui.add(egui::Slider::new(&mut a.fps, 15..=144).text("Animation FPS limit"));
        ui.add(egui::Slider::new(&mut a.idle_fps, 1..=60).text("Idle graphics FPS limit"));
        ui.add(egui::Slider::new(&mut a.transition_ms, 0..=1000).text("Transition time (ms)"));
        ui.add(egui::Slider::new(&mut a.ambient_density, 0..=8).text("Orbit density"));
        ui.checkbox(&mut a.scan_animation, "Scanning page graphic");
        ui.checkbox(&mut a.intro_animation, "Gentle entrance animation");
        ui.checkbox(&mut a.hover_effects, "Hover highlights");
        ui.checkbox(&mut a.reduced_motion, "Reduced motion");
        ui.checkbox(&mut a.particles, "Ambient motion graphics");
        ui.checkbox(&mut a.glow, "Soft accent glow");
        ui.checkbox(&mut a.compact, "Compact spacing");
        ui.checkbox(&mut a.autosave, "Remember settings and queue");
        ui.small("Reduced motion freezes decorative movement and disables transitions. Background animation sleeps when the window is unfocused. The UI graphics adapter is independent of AI GPU selection.");
        if ui.button("Preview appearance now").clicked() {
            let mut preview = self.state.settings.clone();
            preview.ui = self.draft.ui.clone();
            match preview.validate() {
                Ok(()) => {
                    self.state.settings.ui = preview.ui;
                    ui.ctx().request_repaint();
                }
                Err(e) => self.settings_error = e.to_string(),
            }
        }
    }
    fn review_ui(&mut self, ctx: &egui::Context) {
        let Some(mut r) = self.review.take() else {
            self.review_open = false;
            return;
        };
        let mut open = self.review_open;
        let mut next = None;
        let mut export = false;
        egui::Window::new("Review & correct").fade_in(motion::enabled(&self.state.settings.ui)).fade_out(motion::enabled(&self.state.settings.ui)).open(&mut open).default_size([1100.,740.]).resizable(true).show(ctx,|ui|{
            ui.horizontal(|ui|{
                ui.strong(format!("Page {} / {}",r.page.page,r.project.manifest.pages.len()));
                if ui.add_enabled(r.index>0 && !r.dirty,egui::Button::new("← Previous")).clicked(){
                    next=Some(r.index-1);
                }
                if ui.add_enabled(r.index+1<r.project.manifest.pages.len() && !r.dirty,egui::Button::new("Next →")).clicked(){
                    next=Some(r.index+1);
                }
                ui.small(evidence_label(&r.page)).on_hover_text("Reader agreement is not measured accuracy. Crops from one model family do not provide independent agreement; compare uncertain content with the source.");
                if r.dirty{
                    ui.colored_label(Color32::from_rgb(241,187,111),"Unsaved edits");
                }
            }
            );
            ui.separator();
            ui.columns(2,|cols|{
                egui::ScrollArea::both().id_salt("source").max_height(530.).show(&mut cols[0],|ui|{
                    if let Some(texture)=&r.texture {
                        let size=texture.size_vec2();
                        let width=ui.available_width().max(200.);
                        let response=ui.add(egui::Image::new((texture.id(),size)).fit_to_exact_size(Vec2::new(width,width*size.y/size.x)));
                        let rect=response.rect;
                        for(i,b)in r.page.content.blocks.iter().enumerate(){
                            if let Some(b)=&b.bbox{
                                let region=egui::Rect::from_min_max(egui::pos2(rect.left()+b.left*rect.width(),rect.top()+b.top*rect.height()),egui::pos2(rect.left()+b.right*rect.width(),rect.top()+b.bottom*rect.height()));
                                ui.painter().rect_stroke(region,2,egui::Stroke::new(if i==r.selected{
                                    2.}
                                else{
                                    0.7}
                                ,if i==r.selected{
                                    self.accent()}
                                else{
                                    Color32::from_rgba_unmultiplied(120,160,180,100)}
                                ),egui::StrokeKind::Inside);
                                if ui.interact(region,egui::Id::unique(("region",i)),egui::Sense::click()).clicked(){
                                    r.selected=i;
                                }
                            }
                        }
                    }
                    else{
                        ui.label("Source image is unavailable");
                    }
                }
                );
                egui::ScrollArea::vertical().id_salt("editor").max_height(530.).show(&mut cols[1],|ui|{
                    for w in &r.page.content.warnings {
                        ui.colored_label(Color32::from_rgb(241,187,111),w);
                    }
                    ui.add_space(6.);
                    ui.horizontal_wrapped(|ui|{
                        for(i,b)in r.page.content.blocks.iter().enumerate(){
                            ui.selectable_value(&mut r.selected,i,format!("{} {}",i+1,b.kind));
                        }
                    }
                    );
                    let mut changed=false;
                    if let Some(b)=r.page.content.blocks.get_mut(r.selected){
                        ui.separator();
                        ui.horizontal(|ui|{
                            ui.strong(&b.kind);
                            if let Some(c)=b.confidence{
                                ui.small(format!("Reader confidence {:.0}%",c*100.));
                            }
                            changed|=ui.checkbox(&mut b.uncertain,"Uncertain").changed();
                        }
                        );
                        if b.kind=="table"{
                            ui.small("Cells are editable. Use Page JSON for merged cells, headers and structural changes.");
                            egui::Grid::new("cells").striped(true).show(ui,|ui|{
                                for row in &mut b.rows{
                                    for cell in row{
                                        changed|=ui.add(egui::TextEdit::singleline(cell).desired_width(110.)).changed();
                                    }
                                    ui.end_row();
                                }
                            }
                            );
                        }
                        else if b.kind=="figure"{
                            ui.label("Visible evidence");
                            changed|=ui.add(egui::TextEdit::multiline(&mut b.description).desired_rows(4).desired_width(f32::INFINITY)).changed();
                            ui.label("Contextual interpretation");
                            changed|=ui.add(egui::TextEdit::multiline(&mut b.interpretation).desired_rows(4).desired_width(f32::INFINITY)).changed();
                        }
                        else{
                            changed|=ui.add(egui::TextEdit::multiline(&mut b.text).desired_rows(10).desired_width(f32::INFINITY)).changed();
                        }
                        for alt in &b.alternatives {
                            ui.small(format!("Alternative: {alt}"));
                        }
                    }
                    if changed{
                        r.dirty=true;
                        r.page.reviewed=false;
                    }
                    ui.add_space(10.);
                    egui::CollapsingHeader::new("All reader evidence").show(ui,|ui|{
                        for c in &r.page.candidates{
                            egui::CollapsingHeader::new(format!("{} · {} · {}",c.model_id,c.family,c.view)).id_salt((&c.model_id,&c.view)).show(ui,|ui|{
                                ui.label(c.content.plain());
                                for b in &c.content.blocks{
                                    if b.kind=="figure"{
                                        ui.label(&b.description);
                                        ui.label(&b.interpretation);
                                    }
                                }
                            }
                            );
                        }
                    }
                    );
                    if ui.button("Edit page JSON").clicked(){
                        r.raw=serde_json::to_string_pretty(&r.page.content).unwrap_or_default();
                        r.raw_open = !r.raw_open;
                    }
                    if r.raw_open{
                        ui.add(egui::TextEdit::multiline(&mut r.raw).code_editor().desired_rows(15).desired_width(f32::INFINITY));
                        if ui.button("Validate and replace page content").clicked(){
                            match serde_json::from_str::<PageContent>(&r.raw).map_err(anyhow::Error::from).and_then(|p|{
                                p.validate()?;
                                Ok(p)}
                            ){
                                Ok(p)=>{
                                    r.page.content=p;
                                    r.dirty=true;
                                    r.selected=0;
                                    self.review_error.clear();
                                }
                                ,Err(e)=>self.review_error=e.to_string()}
                        }
                    }
                }
                );
            }
            );
            ui.separator();
            ui.horizontal(|ui|{
                if ui.button("Save draft").clicked(){
                    match save_review(&r.project,&mut r.page,false){
                        Ok(_)=>{
                            r.dirty=false;
                            self.review_error.clear();
                        }
                        ,Err(e)=>self.review_error=e.to_string()}
                }
                if ui.button("Save & mark reviewed").clicked(){
                    match save_review(&r.project,&mut r.page,true){
                        Ok(_)=>{
                            r.dirty=false;
                            self.review_error.clear();
                        }
                        ,Err(e)=>self.review_error=e.to_string()}
                }
                if ui.add_enabled(!r.dirty && self.worker.is_none(),egui::Button::new("Export corrected Word")).clicked(){
                    export=true;
                }
                if ui.button("Open project folder").clicked(){
                    open_path(&r.project.root);
                }
            }
            );
            if !self.review_error.is_empty(){
                ui.colored_label(Color32::from_rgb(244,161,140),&self.review_error);
            }
            ui.small("Drafts preserve your edits across resume. Mark reviewed only after comparing with the source. Exports use a new filename.");
        }
        );
        if let Some(i) = next {
            match Review::load(r.project.clone(), i, ctx) {
                Ok(v) => r = v,
                Err(e) => self.review_error = e.to_string(),
            }
        }
        if export
            && let Some(p) = rfd::FileDialog::new()
                .set_file_name("corrected.docx")
                .add_filter("Word", &["docx"])
                .save_file()
        {
            let project = r.project.clone();
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            self.control = Control::default();
            let c = self.control.clone();
            self.worker = Some(thread::spawn(move || {
                let msg = match word::export(&project, &p, &c) {
                    Ok(_) => Message::Exported(p),
                    Err(e) => failure(e),
                };
                let _ = tx.send(msg);
                ctx.request_repaint();
            }));
        }
        if !open && r.dirty {
            match save_review(&r.project, &mut r.page, false) {
                Ok(()) => {
                    r.dirty = false;
                    self.notice = "Review draft saved".into();
                }
                Err(e) => {
                    self.review_error = e.to_string();
                    open = true;
                }
            }
        }
        self.review = Some(r);
        self.review_open = open;
    }
    fn accent(&self) -> Color32 {
        let a = self.state.settings.ui.accent;
        Color32::from_rgb(a[0], a[1], a[2])
    }
    fn background(&self) -> Color32 {
        if self.state.settings.ui.dark {
            let a = self.state.settings.ui.background;
            Color32::from_rgb(a[0], a[1], a[2])
        } else {
            Color32::from_rgb(245, 247, 250)
        }
    }
    fn surface(&self) -> Color32 {
        if self.state.settings.ui.dark {
            self.background().lerp_to_gamma(Color32::WHITE, 0.035)
        } else {
            Color32::WHITE
        }
    }
    fn muted(&self) -> Color32 {
        if self.state.settings.ui.dark {
            Color32::from_rgb(150, 164, 183)
        } else {
            Color32::from_rgb(88, 104, 123)
        }
    }
    fn border(&self) -> Color32 {
        if self.state.settings.ui.dark {
            Color32::from_rgb(45, 57, 74)
        } else {
            Color32::from_rgb(211, 221, 229)
        }
    }
    fn theme(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.state.settings.ui.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        ctx.set_zoom_factor(self.state.settings.ui.scale);
        let accent = self.accent();
        ctx.global_style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(10., 9.);
            s.spacing.button_padding = Vec2::new(12., 9.);
            s.visuals.selection.bg_fill = accent.gamma_multiply(0.25);
            s.visuals.selection.stroke = egui::Stroke::new(1., accent);
            s.animation_time = motion::transition(&self.state.settings.ui);
        });
    }
    fn logo(&self, ui: &mut egui::Ui) {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(30.), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, 8, self.accent());
        for (i, w) in [15., 10., 13.].iter().enumerate() {
            p.line_segment(
                [
                    r.min + Vec2::new(7., 9. + i as f32 * 6.),
                    r.min + Vec2::new(7. + w, 9. + i as f32 * 6.),
                ],
                egui::Stroke::new(2., self.accent_text()),
            );
        }
    }
    fn scene(&self, ui: &egui::Ui) -> motion::Scene {
        let time = ui.input(|i| i.time);
        let elapsed = time - self.opened_at.unwrap_or(time);
        let complete = if self.active.is_none() && self.progress >= 1. {
            self.completed_at.map(|t| t.elapsed().as_secs_f64())
        } else {
            None
        };
        motion::scene(
            elapsed,
            &self.state.settings.ui,
            self.active.is_some(),
            self.paused(),
            complete,
        )
    }
    fn paused(&self) -> bool {
        self.active.is_some() && self.control.paused.load(Ordering::Relaxed)
    }
    fn activity_indicator(&self, ui: &mut egui::Ui) {
        if motion::enabled(&self.state.settings.ui) {
            ui.spinner();
        } else {
            ui.colored_label(self.accent(), "●");
        }
    }
    fn accent_text(&self) -> Color32 {
        let a = self.state.settings.ui.accent;
        if 0.2126 * a[0] as f32 + 0.7152 * a[1] as f32 + 0.0722 * a[2] as f32 > 140. {
            Color32::from_rgb(8, 28, 28)
        } else {
            Color32::WHITE
        }
    }
    fn hover_ring(&self, ui: &egui::Ui, response: &egui::Response, salt: &str) {
        if !self.state.settings.ui.hover_effects {
            return;
        }
        let amount = ui.ctx().animate_bool_with_time(
            egui::Id::unique(salt),
            response.hovered() && response.enabled(),
            motion::transition(&self.state.settings.ui),
        );
        if amount > 0.001 {
            let a = self.accent();
            ui.painter().rect_stroke(
                response.rect.expand(1. + amount),
                self.state.settings.ui.rounding.min(16),
                egui::Stroke::new(
                    1.,
                    Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), (110. * amount) as u8),
                ),
                egui::StrokeKind::Outside,
            );
        }
    }
    fn background_art(&self, ui: &egui::Ui) {
        let style = &self.state.settings.ui;
        if !style.particles && !style.scan_animation {
            return;
        }
        let scene = self.scene(ui);
        let p = ui.painter();
        let rect = ui.max_rect();
        let center = egui::pos2(rect.right() - 100., rect.top() + 116.);
        let accent = self.accent();
        let tint = |alpha: f32| {
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                (alpha.clamp(0., 255.) * scene.reveal) as u8,
            )
        };
        if style.glow {
            for k in (1..=6).rev() {
                p.circle_filled(center, 30. + k as f32 * 17., tint(2.));
            }
        }
        if style.particles {
            for i in 0..style.ambient_density.min(8) {
                let radius = 54. + i as f32 * 23.;
                p.circle_stroke(center, radius, egui::Stroke::new(0.7, tint(16.)));
                let angle = scene.orbit + i as f32 * 2.1;
                let dot = center + Vec2::angled(angle) * radius;
                if style.glow {
                    p.circle_filled(dot, 5., tint(10. + scene.pulse * 8.));
                }
                p.circle_filled(dot, 2.2, tint(80. + scene.pulse * 35.));
            }
        }
        if !style.scan_animation {
            return;
        }
        let paper = egui::Rect::from_center_size(
            center + Vec2::new(0., (1. - scene.reveal) * 8.),
            Vec2::new(80., 104.),
        );
        p.rect_filled(
            paper.translate(Vec2::new(0., 4.)),
            10,
            Color32::from_black_alpha((35. * scene.reveal) as u8),
        );
        let fill = if style.dark {
            Color32::from_rgb(23, 35, 48)
        } else {
            Color32::from_rgb(253, 254, 255)
        };
        p.rect_filled(paper, 9, fill.gamma_multiply(scene.reveal));
        p.rect_stroke(
            paper,
            9,
            egui::Stroke::new(1., tint(105.)),
            egui::StrokeKind::Inside,
        );
        for (i, width) in [37., 51., 43., 49., 33.].iter().enumerate() {
            let pos = paper.min + Vec2::new(12., 16. + i as f32 * 11.);
            p.rect_filled(
                egui::Rect::from_min_size(pos, Vec2::new(*width, 3.)),
                2,
                if i == 0 {
                    tint(170.)
                } else {
                    self.muted().gamma_multiply(scene.reveal * 0.38)
                },
            );
        }
        for (i, height) in [9., 17., 13.].iter().enumerate() {
            p.rect_filled(
                egui::Rect::from_min_size(
                    paper.min + Vec2::new(13. + i as f32 * 12., 88. - height),
                    Vec2::new(7., *height),
                ),
                2,
                tint(50. + i as f32 * 25.),
            );
        }
        let clipped = p.with_clip_rect(paper.shrink(2.));
        if scene.check > 0. {
            let base = paper.min + Vec2::new(23., 71.);
            let points = [
                base,
                base + Vec2::new(11., 11.),
                base + Vec2::new(30., -10.),
            ];
            let amount = scene.check;
            if amount < 0.4 {
                clipped.line_segment(
                    [points[0], points[0].lerp(points[1], amount / 0.4)],
                    egui::Stroke::new(3., accent),
                );
            } else {
                clipped.line_segment([points[0], points[1]], egui::Stroke::new(3., accent));
                clipped.line_segment(
                    [points[1], points[1].lerp(points[2], (amount - 0.4) / 0.6)],
                    egui::Stroke::new(3., accent),
                );
            }
        } else {
            let y = egui::lerp(paper.top() + 10. ..=paper.bottom() - 10., scene.scan);
            if style.glow {
                for k in (1..=5).rev() {
                    clipped.rect_filled(
                        egui::Rect::from_center_size(
                            egui::pos2(paper.center().x, y),
                            Vec2::new(paper.width() - 8., k as f32 * 4.),
                        ),
                        2,
                        tint(5.),
                    );
                }
            }
            clipped.line_segment(
                [
                    egui::pos2(paper.left() + 5., y),
                    egui::pos2(paper.right() - 5., y),
                ],
                egui::Stroke::new(1.5, tint(220.)),
            );
            clipped.circle_filled(egui::pos2(paper.right() - 7., y), 2.2, tint(255.));
        }
    }
}
impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(ui);
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll(ctx);
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.state.settings.ui.autosave
            && let Ok(s) = serde_json::to_string(&self.state)
        {
            storage.set_string("workspace", s);
        }
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        let _ = self.flush_review();
        self.control.cancelled.store(true, Ordering::Relaxed);
        self.running_queue = false;
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
fn failure(e: anyhow::Error) -> Message {
    let f = e.downcast_ref::<Fault>();
    Message::Failed(
        f.map_or("NG500", |f| f.code).into(),
        format!(
            "{e:#}\n{}",
            f.map_or(
                "Inspect the local activity log and check the selected files.",
                |f| f.remedy()
            )
        ),
    )
}
fn code_static(s: &str) -> &'static str {
    match s {
        "NG100" => "NG100",
        "NG102" => "NG102",
        "NG110" => "NG110",
        "NG200" => "NG200",
        "NG201" => "NG201",
        "NG202" => "NG202",
        "NG203" => "NG203",
        "NG300" => "NG300",
        "NG400" => "NG400",
        "NG401" => "NG401",
        "NG500" => "NG500",
        "NG900" => "NG900",
        _ => "NG500",
    }
}
fn numeric<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut T,
    range: std::ops::RangeInclusive<T>,
) {
    ui.label(label);
    ui.add(egui::DragValue::new(v).range(range));
    ui.end_row();
}
fn path_control(ui: &mut egui::Ui, label: &str, value: &mut PathBuf, folder: bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut text = value.display().to_string();
        if ui
            .add(egui::TextEdit::singleline(&mut text).desired_width(330.))
            .changed()
        {
            *value = text.into();
        }
        if ui.small_button("Browse").clicked() {
            let dialog = rfd::FileDialog::new();
            if let Some(p) = if folder {
                dialog.pick_folder()
            } else {
                dialog.pick_file()
            } {
                *value = p;
            }
        }
    });
}
fn file_string(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(330.));
        if ui.small_button("Browse").clicked()
            && let Some(p) = rfd::FileDialog::new()
                .add_filter("GGUF", &["gguf"])
                .pick_file()
        {
            *value = p.display().to_string();
        }
    });
}
fn open_path(path: &Path) {
    #[cfg(windows)]
    let tool = "explorer.exe";
    #[cfg(not(windows))]
    let tool = "xdg-open";
    let _ = hardware::command(tool).arg(path).spawn();
}
#[cfg(test)]
mod acceptance {
    use super::*;
    use std::time::Duration;
    fn draw(app: &mut Desktop, ctx: &egui::Context) {
        let mut result = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1200., 900.),
                )),
                ..Default::default()
            },
            |ui| app.render(ui),
        );
        assert!(!result.shapes.is_empty());
        result.textures_delta.clear();
    }
    #[test]
    fn single_reader_display_never_implies_independent_agreement() {
        let mut page = Page {
            agreement: 1.,
            ..Default::default()
        };
        assert_eq!(evidence_label(&page), "No reader evidence");
        let candidate = Candidate {
            region: None,
            model_id: "ocr".into(),
            family: "one".into(),
            view: "full".into(),
            content: PageContent::default(),
            reliability: 1.,
        };
        page.candidates.push(candidate.clone());
        assert!(evidence_label(&page).contains("unavailable"));
        assert!(!evidence_label(&page).contains("100"));
        page.candidates.push(Candidate {
            view: "crop-0".into(),
            ..candidate.clone()
        });
        assert!(evidence_label(&page).contains("unavailable"));
        page.candidates.push(Candidate {
            family: "two".into(),
            ..candidate
        });
        assert!(evidence_label(&page).contains("2 reader families"));
    }
    #[test]
    fn every_advanced_tab_renders_and_logs_are_bounded() {
        let mut app = Desktop::new(Workspace::default());
        let ctx = egui::Context::default();
        app.advanced_open();
        for tab in 0..5 {
            app.tab = tab;
            draw(&mut app, &ctx);
            draw(&mut app, &ctx);
        }
        app.log_open = true;
        app.advanced = false;
        for _ in 0..2100 {
            app.tx
                .send(Message::Event(Event {
                    timestamp: "2026-10-30T12:00:00Z".into(),
                    stage: "inference".into(),
                    message: "Local reader".into(),
                    page: 1,
                    done: 1,
                    total: 4,
                    code: String::new(),
                    warning: false,
                }))
                .unwrap();
        }
        for _ in 0..5 {
            draw(&mut app, &ctx);
        }
        assert_eq!(app.logs.len(), 2000);
        assert_eq!(app.progress, 0.25);
    }
    #[test]
    fn worker_failure_stops_queue_and_explains_error() {
        let mut state = Workspace::default();
        state.settings.preset = Preset::Fast;
        let dir = tempfile::tempdir().unwrap();
        state.enqueue(dir.path().join("notegen.pdf"));
        state.enqueue(dir.path().join("second.pdf"));
        let mut app = Desktop::new(state);
        let ctx = egui::Context::default();
        app.running_queue = true;
        let limit = Instant::now() + Duration::from_secs(10);
        loop {
            draw(&mut app, &ctx);
            if app.worker.is_none() && !app.running_queue {
                break;
            }
            assert!(Instant::now() < limit);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.state.queue[0].status, JobStatus::Failed);
        assert_eq!(app.state.queue[1].status, JobStatus::Pending);
        assert!(!app.notice.is_empty());
    }
    #[test]
    fn review_editor_draws_text_table_and_figure() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(
            dir.path().into(),
            Manifest {
                schema: 2,
                input_sha256: "a".into(),
                fingerprint: "b".into(),
                pages: vec![1],
                models: vec![],
                settings: Settings::default(),
            },
        )
        .unwrap();
        image::RgbImage::from_pixel(200, 300, image::Rgb([255, 255, 255]))
            .save(p.image(1))
            .unwrap();
        p.save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks: vec![
                    Block::text("Text"),
                    Block {
                        kind: "table".into(),
                        rows: vec![vec!["A".into(), "1250".into()]],
                        ..Default::default()
                    },
                    Block {
                        kind: "figure".into(),
                        bbox: Some(BoxRect::full()),
                        description: "Source crop".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let ctx = egui::Context::default();
        let mut app = Desktop::new(Workspace::default());
        app.open_review(p.root, &ctx);
        for i in 0..3 {
            app.review.as_mut().unwrap().selected = i;
            draw(&mut app, &ctx);
            draw(&mut app, &ctx);
        }
    }
    #[test]
    fn switching_projects_saves_the_previous_review_draft() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(
            dir.path().into(),
            Manifest {
                schema: 2,
                input_sha256: "a".into(),
                fingerprint: "b".into(),
                pages: vec![1],
                models: vec![],
                settings: Settings::default(),
            },
        )
        .unwrap();
        p.save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks: vec![Block::text("Original")],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let ctx = egui::Context::default();
        let mut app = Desktop::new(Workspace::default());
        app.open_review(p.root.clone(), &ctx);
        let r = app.review.as_mut().unwrap();
        r.page.content.blocks[0].text = "Human correction".into();
        r.dirty = true;
        app.open_review(p.root.clone(), &ctx);
        assert!(p.load(1).unwrap().edited);
        assert_eq!(
            app.review.as_ref().unwrap().page.content.plain(),
            "Human correction"
        );
    }

    #[test]
    fn invalid_output_folder_marks_job_failed_without_starting_worker() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("a-file");
        fs::write(&blocked, b"file").unwrap();
        let mut state = Workspace::default();
        state.settings.preset = Preset::Fast;
        state.output_dir = blocked.display().to_string();
        state.enqueue(dir.path().join("scan.pdf"));
        let mut app = Desktop::new(state);
        let ctx = egui::Context::default();
        app.running_queue = true;
        draw(&mut app, &ctx);
        assert_eq!(app.state.queue[0].status, JobStatus::Failed);
        assert_eq!(app.error_code, "NG400");
        assert!(app.worker.is_none());
    }
}
