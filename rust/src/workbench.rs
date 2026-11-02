//! Keyboard-first review helpers shared by the native workbench.
use super::*;

enum Report {
    Catalog(Vec<crate::projects::Entry>),
    Search(crate::projects::SearchReport),
    Text(String),
}
#[derive(Default)]
pub(super) struct Workbench {
    pub open: bool,
    pub palette: bool,
    tab: usize,
    query: String,
    palette_query: String,
    entries: Vec<crate::projects::Entry>,
    hits: Vec<crate::projects::SearchHit>,
    report: String,
    metadata: String,
    format: usize,
    find: String,
    replace: String,
    rule: Option<crate::review::RulePreview>,
    rx: Option<mpsc::Receiver<std::result::Result<Report, String>>>,
}
impl Workbench {
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
            return;
        }
        let mut w = std::mem::take(&mut self.workbench);
        let mut open = true;
        let mut jump = None;
        let mut reload_review = false;
        egui::Window::new("Project workbench").open(&mut open).default_size([850.,680.]).resizable(true).show(ctx,|ui| {
            ui.horizontal_wrapped(|ui| {for (i,label) in ["Projects & search","Export & rules","Recovery & references","Privacy & readability"].iter().enumerate(){ui.selectable_value(&mut w.tab,i,*label);}});
            ui.separator();
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
                    1=>{
                        if let Some(r)=&self.review {
                            ui.label(format!("Current project: {}",r.project.root.display()));
                            let formats=[crate::exports::Format::Markdown,crate::exports::Format::Html,crate::exports::Format::Latex,crate::exports::Format::Spreadsheet,crate::exports::Format::Traceability];
                            egui::ComboBox::from_label("Output format").selected_text(format!("{:?}",formats[w.format.min(4)])).show_ui(ui,|ui|{for(i,f)in formats.iter().enumerate(){ui.selectable_value(&mut w.format,i,format!("{f:?}"));}});
                            if ui.add_enabled(!r.dirty && self.worker.is_none(),egui::Button::new("Export a new file")).clicked(){let format=formats[w.format.min(4)];if let Some(output)=rfd::FileDialog::new().set_file_name(format!("notes.{}",format.extension())).save_file(){let project=r.project.clone();let tx=self.tx.clone();let ctx=ctx.clone();self.control=Control::default();let c=self.control.clone();self.worker=Some(thread::spawn(move||{let msg=match crate::exports::export(&project,&output,format,&c){Ok(())=>Message::Exported(output),Err(e)=>failure(e)};let _=tx.send(msg);ctx.request_repaint();}));}}
                            ui.separator();ui.strong("Reversible replacement rule");ui.label("Preview literal replacements in all project pages. Reader evidence stays preserved; apply creates one undoable revision. CLI rules also support page selection and block exclusions.");
                            ui.horizontal(|ui|{ui.label("Find");ui.text_edit_singleline(&mut w.find);ui.label("Replace");ui.text_edit_singleline(&mut w.replace);});
                            if ui.add_enabled(!r.dirty && self.worker.is_none(),egui::Button::new("Preview rule")).clicked(){let rule=crate::review::Rule{find:w.find.clone(),replace:w.replace.clone(),pages:r.project.manifest.pages.clone(),exclude_blocks:vec![]};match crate::review::preview_rule(&r.project,&rule){Ok(preview)=>w.rule=Some(preview),Err(e)=>w.report=e.to_string()}}
                            if let Some(preview)=&w.rule {for m in preview.matches.iter().take(40){ui.label(format!("Page {} block {}: {:?} → {:?}",m.page,m.block+1,m.before,m.after));}if ui.add_enabled(!r.dirty && self.worker.is_none(),egui::Button::new("Apply this preview")).clicked(){match crate::review::apply_rule(&r.project,preview,"Confirmed replacement rule"){Ok(id)=>{w.report=format!("Saved revision {id}. Undo is available in Review.");w.rule=None;reload_review=true;},Err(e)=>w.report=e.to_string()}}}
                        } else {ui.label("Open a project or finish a conversion first.");}
                    }
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
                            if ui.button("Default appearance").clicked(){self.state.settings.ui=Appearance::default();}
                        });
                        ui.small("Ctrl+K opens the command palette. Tab moves focus, Enter activates buttons. Review includes source zoom and disagreement markers.");
                    }
                }
                if !w.report.is_empty(){ui.separator();ui.add(egui::TextEdit::multiline(&mut w.report).desired_rows(10).desired_width(f32::INFINITY));}
            });
        });
        w.open = open;
        self.workbench = w;
        if reload_review
            && let Some(r) = &mut self.review
            && let Err(e) = r.reload()
        {
            self.notice = e.to_string();
        }
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
