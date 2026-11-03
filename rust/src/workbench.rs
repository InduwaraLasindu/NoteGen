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
    Routing(PathBuf, crate::language_routing::RereadPreview),
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
    palette_focus: bool,
    entries: Vec<crate::projects::Entry>,
    catalog_loaded: bool,
    hits: Vec<crate::projects::SearchHit>,
    report: String,
    metadata: String,
    format: usize,
    find: String,
    replace: String,
    layer_kind: usize,
    layer_label: String,
    layer_note: String,
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
    mask_preview: Option<crate::redactions::ConfirmationPreview>,
    mask_selected: BTreeSet<usize>,
    mask_reason: usize,
    share_preview: Option<crate::redactions::SharePreview>,
    notation: String,
    notation_reaction: bool,
    shortcut_draft: Option<crate::shortcuts::Bindings>,
    line_shapes: String,
    line_ordinal: usize,
    grid_repair: String,
    grid_preview: Option<crate::geometry_repair::GridPreview>,
    route_codes: String,
    route_line: bool,
    route_preview: Option<crate::language_routing::RereadPreview>,
    geometry_project: Option<PathBuf>,
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
                Ok(Report::Routing(project, preview)) => {
                    if self.geometry_project.as_ref() == Some(&project) {
                        self.report="Regional OCR preview is ready. Compare its source and visible text before applying.".into();
                        self.route_preview = Some(preview);
                    } else {
                        self.report="The completed regional preview belongs to another project. Reopen it and refresh the preview before applying.".into();
                    }
                }
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
    pub(super) fn inherit_session(&mut self, previous: &mut Self) {
        if self.project.root == previous.project.root {
            self.auto_advance = previous.auto_advance;
            self.zoom = previous.zoom;
            self.heatmap = previous.heatmap;
            self.issue_page = previous.issue_page;
            self.deferred_issues = std::mem::take(&mut previous.deferred_issues);
            self.active_issue = previous.active_issue.take();
            self.prune_session();
        }
    }
    fn prune_session(&mut self) {
        let live: BTreeSet<&str> = self
            .issues
            .iter()
            .filter(|i| i.page > 0)
            .map(|i| i.id.as_str())
            .collect();
        self.deferred_issues.retain(|id| live.contains(id.as_str()));
        if self
            .active_issue
            .as_ref()
            .is_some_and(|id| !live.contains(id.as_str()))
        {
            self.active_issue = None;
        }
        self.issue_page = self.issue_page.min(
            self.issues
                .iter()
                .filter(|i| i.page > 0)
                .count()
                .saturating_sub(1)
                / 100,
        );
    }
    pub(super) fn navigate(
        &mut self,
        index: usize,
        block: usize,
        ctx: &egui::Context,
    ) -> Result<()> {
        anyhow::ensure!(
            !self.dirty && !self.raw_open,
            Fault::new(
                "NG500",
                "Save the draft and close page JSON before navigating"
            )
        );
        let mut next = Self::load(self.project.clone(), index, ctx)?;
        anyhow::ensure!(
            block < next.page.content.blocks.len() || block == 0,
            Fault::new("NG500", "The selected source block changed; refresh review")
        );
        next.selected = block;
        next.inherit_session(self);
        *self = next;
        Ok(())
    }
    fn selected_issue(&self) -> Option<&crate::review::Issue> {
        self.issues
            .iter()
            .find(|i| {
                i.page == self.page.page
                    && i.block == self.selected
                    && self.active_issue.as_ref() == Some(&i.id)
            })
            .or_else(|| {
                self.issues.iter().find(|i| {
                    i.page == self.page.page
                        && i.block == self.selected
                        && !self.deferred_issues.contains(&i.id)
                })
            })
    }
    fn open_issue(&mut self, id: &str, ctx: &egui::Context) -> Result<()> {
        let issue = self
            .issues
            .iter()
            .find(|i| i.page > 0 && i.id == id)
            .ok_or_else(|| Fault::new("NG500", "Review issue is no longer available"))?;
        let index = self
            .project
            .manifest
            .pages
            .iter()
            .position(|n| *n == issue.page)
            .ok_or_else(|| Fault::new("NG500", "Review issue page is unavailable"))?;
        let block = issue.block;
        self.navigate(index, block, ctx)?;
        self.active_issue = self
            .issues
            .iter()
            .any(|i| i.id == id)
            .then(|| id.to_owned());
        Ok(())
    }
    fn next_issue(&mut self, ctx: &egui::Context) -> Result<()> {
        anyhow::ensure!(
            !self.dirty && !self.raw_open,
            Fault::new(
                "NG500",
                "Save the draft and close page JSON before navigating"
            )
        );
        let start = self
            .active_issue
            .as_ref()
            .and_then(|id| self.issues.iter().position(|i| &i.id == id))
            .map_or(0, |position| position + 1);
        let id = self
            .issues
            .iter()
            .skip(start)
            .chain(self.issues.iter().take(start))
            .find(|i| i.page > 0 && !self.deferred_issues.contains(&i.id))
            .map(|i| i.id.clone());
        if let Some(id) = id {
            self.open_issue(&id, ctx)?;
        }
        Ok(())
    }
    fn defer_selected(&mut self, ctx: &egui::Context) -> Result<()> {
        anyhow::ensure!(
            !self.dirty && !self.raw_open,
            Fault::new(
                "NG500",
                "Save the draft and close page JSON before deferring an issue"
            )
        );
        if let Some(issue) = self.selected_issue() {
            let id = issue.id.clone();
            self.deferred_issues.insert(id.clone());
            self.active_issue = Some(id);
            self.next_issue(ctx)?;
        }
        Ok(())
    }
    pub(super) fn save(&mut self, approved: bool) -> Result<()> {
        let mut page = self.page.clone();
        if !page.confirmed_redactions.is_empty() {
            page = crate::redactions::effective_page(&self.project, &page, &Control::default())?.0;
        }
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
        if self.dirty {
            for issue in &self.issues {
                if issue.page == self.page.page {
                    self.deferred_issues.remove(&issue.id);
                }
            }
        }
        self.page = page.clone();
        self.baseline = page;
        self.baseline_id = fingerprint(&self.baseline)?;
        self.selected = self
            .selected
            .min(self.page.content.blocks.len().saturating_sub(1));
        self.raw = serde_json::to_string_pretty(&self.page.content)?;
        self.dirty = false;
        self.refresh_issues();
        self.revision_name.clear();
        Ok(())
    }
    fn history_action(&mut self, redo: bool) -> Result<()> {
        anyhow::ensure!(
            !self.dirty && !self.raw_open,
            Fault::new(
                "NG500",
                "Save the current draft and close page JSON before undo or redo"
            )
        );
        if redo {
            crate::review::redo(&self.project)?;
        } else {
            crate::review::undo(&self.project)?;
        }
        self.reload()
    }
    fn reload(&mut self) -> Result<()> {
        let canonical = self.project.load(self.page.page)?;
        let visible = if canonical.confirmed_redactions.is_empty() {
            canonical.clone()
        } else {
            crate::redactions::effective_page(&self.project, &canonical, &Control::default())?.0
        };
        let source = review_source(&self.project, &canonical);
        let source = if canonical.confirmed_redactions.is_empty() {
            source.ok()
        } else {
            Some(source?)
        };
        let limit = self
            .context
            .input(|input| input.max_texture_side)
            .clamp(1, 2048) as u32;
        self.texture = source.map(|source| {
            let image = source.thumbnail(limit, limit).to_rgba8();
            self.context.load_texture(
                "reloaded-review-source",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            )
        });
        self.magnifier = None;
        let compacted = fingerprint(&canonical)? != fingerprint(&visible)?;
        self.page = visible;
        self.baseline = canonical;
        self.baseline_id = fingerprint(&self.baseline)?;
        self.raw = serde_json::to_string_pretty(&self.page.content)?;
        self.selected = self
            .selected
            .min(self.page.content.blocks.len().saturating_sub(1));
        self.refresh_issues();
        if compacted {
            self.issues.retain(|issue| issue.page != self.page.page);
            self.issue_notice="Confirmed masks are applied to this history view. Save a draft to commit its visible block order before using this page's issue queue; Undo and Redo remain available.".into();
        }
        self.dirty = false;
        Ok(())
    }
    fn visible_revision_comparison(&self, revision: u64) -> Result<String> {
        let mut old = crate::review::snapshot(&self.project, revision, self.page.page)?;
        if !self.page.confirmed_redactions.is_empty() {
            old.confirmed_redactions = self.page.confirmed_redactions.clone();
        }
        let old = crate::redactions::analysis_page(&self.project, &old, &Control::default())?;
        let current =
            crate::redactions::analysis_page(&self.project, &self.page, &Control::default())?;
        Ok(serde_json::to_string_pretty(&crate::review::compare(
            &old, &current,
        )?)?)
    }
    fn refresh_issues(&mut self) {
        self.fine_heatmap = crate::geometry::heatmap(
            &self.project,
            &self.page,
            self.project.manifest.settings.review_threshold,
            &Control::default(),
        )
        .ok();
        match crate::layers::report(&self.page) {
            Ok(layers) => {
                self.layer_cache = layers;
                self.layer_notice.clear();
            }
            Err(e) => {
                self.layer_cache.clear();
                self.layer_notice = format!("Saved labels could not be refreshed: {e}");
            }
        }
        match crate::review::issues(&self.project) {
            Ok(issues) => {
                self.issues = issues;
                self.issue_notice.clear();
            }
            Err(e) => {
                self.issues.clear();
                self.issue_notice = format!(
                    "Saved changes are preserved. Review queue could not be refreshed: {e}. Open Recovery to inspect other checkpoints."
                );
            }
        }
        self.prune_session();
    }
}

impl Desktop {
    pub(super) fn paint_source_geometry(&self, review: &Review, ui: &egui::Ui, rect: egui::Rect) {
        let point = |x: f32, y: f32| {
            egui::pos2(
                rect.left() + x * rect.width(),
                rect.top() + y * rect.height(),
            )
        };
        if review.heatmap
            && let Some(report) = &review.fine_heatmap
        {
            for spot in &report.spots {
                let region = &spot.region;
                let bounds = egui::Rect::from_min_max(
                    point(region.left, region.top),
                    point(region.right, region.bottom),
                );
                let color = match spot.kind {
                    crate::geometry::HeatKind::LowWordConfidence => {
                        Color32::from_rgba_unmultiplied(255, 166, 45, 65)
                    }
                    _ => Color32::from_rgba_unmultiplied(235, 80, 115, 70),
                };
                ui.painter().rect_filled(bounds, 1, color);
            }
        }
        let preview = self.workbench.grid_preview.as_ref().filter(|preview| {
            self.workbench.geometry_project.as_ref() == Some(&review.project.root)
                && preview.before_fingerprint == review.baseline_id
        });
        let block = preview
            .map_or(&review.page, |preview| &preview.after)
            .content
            .blocks
            .get(review.selected);
        let Some(data) = block.and_then(|block| block.geometry.as_ref()) else {
            return;
        };
        let color = if preview.is_some() {
            Color32::from_rgb(255, 220, 100)
        } else {
            Color32::from_rgb(100, 225, 205)
        };
        for line in &data.lines {
            for edge in 0..4 {
                let a = line.shape.points[edge];
                let b = line.shape.points[(edge + 1) % 4];
                ui.painter().line_segment(
                    [point(a.x, a.y), point(b.x, b.y)],
                    egui::Stroke::new(1.2, color),
                );
            }
        }
        if let Some(grid) = &data.grid {
            if let (Some(left), Some(right), Some(top), Some(bottom)) = (
                grid.column_edges.first(),
                grid.column_edges.last(),
                grid.row_edges.first(),
                grid.row_edges.last(),
            ) {
                for y in &grid.row_edges {
                    ui.painter().line_segment(
                        [point(*left, *y), point(*right, *y)],
                        egui::Stroke::new(1.4, color),
                    );
                }
                for x in &grid.column_edges {
                    ui.painter().line_segment(
                        [point(*x, *top), point(*x, *bottom)],
                        egui::Stroke::new(1.4, color),
                    );
                }
            }
        }
    }
    fn geometry_tools(&mut self, w: &mut Workbench, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((project, page, index, region, dirty)) = self.review.as_ref().map(|r| {
            (
                r.project.clone(),
                r.page.clone(),
                r.selected,
                r.crop.clone().or_else(|| {
                    r.page
                        .content
                        .blocks
                        .get(r.selected)
                        .and_then(|b| b.bbox.clone())
                }),
                r.dirty || r.raw_open,
            )
        }) else {
            return;
        };
        let ready = !dirty && self.worker.is_none() && w.rx.is_none();
        if w.geometry_project.as_ref() != Some(&project.root) {
            w.geometry_project = Some(project.root.clone());
            w.grid_preview = None;
            w.route_preview = None;
            w.line_shapes.clear();
            w.grid_repair.clear();
        }
        ui.separator();
        ui.heading("Source geometry and language regions");
        if let Some(detection) = &page.script_detection {
            ui.label(format!(
                "Detected script: {:?} · native score {:?} · suggestions {}",
                detection.observation.script,
                detection.observation.script_confidence,
                detection.observation.suggested_languages.join(" + ")
            ));
            ui.small(&detection.observation.note);
        }
        if let Some(data) = page
            .content
            .blocks
            .get(index)
            .and_then(|block| block.geometry.as_ref())
        {
            ui.label(format!(
                "Selected block: {} word bounds · {} cell bounds · {} line regions",
                data.words.len(),
                data.cells.len(),
                data.lines.len()
            ));
        } else {
            ui.label("The selected block has no word/cell geometry. Only supplied source bounds are shown.");
        }
        ui.small("Review overlays show supplied word confidence scores, line outlines and table grids. Native confidence scores are not calibrated error probabilities. Previewed grid repairs are shown before application.");
        egui::CollapsingHeader::new("Mixed-language region recognition").show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut w.route_codes)
                    .hint_text("eng+sin, eng+tam, ara or heb")
                    .char_limit(100),
            )
            .accessible_name("OCR languages for selected source region");
            ui.checkbox(
                &mut w.route_line,
                "Selected region is one line (otherwise paragraph)",
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::new("Inspect image-based paragraph scripts"),
                    )
                    .clicked()
                {
                    let project = project.clone();
                    let page = page.clone();
                    let settings = self.state.settings.clone();
                    w.task(ctx, move || {
                        Ok(Report::Text(serde_json::to_string_pretty(
                            &crate::language_routing::suggest_regions(
                                &project,
                                &page,
                                &settings,
                                8,
                                &Control::default(),
                            )?,
                        )?))
                    });
                }
                if ui
                    .add_enabled(
                        ready && region.is_some() && !w.route_codes.trim().is_empty(),
                        egui::Button::new("Confirm language selection for this region"),
                    )
                    .clicked()
                {
                    let selected = region.clone().expect("enabled measured region");
                    let layout = if w.route_line {
                        crate::language_routing::RegionLayout::Line
                    } else {
                        crate::language_routing::RegionLayout::Paragraph
                    };
                    let target = page
                        .content
                        .blocks
                        .get(index)
                        .and_then(|block| block.bbox.as_ref())
                        .filter(|bounds| **bounds == selected)
                        .map(|_| index);
                    let result = crate::language_routing::select_region(
                        &project,
                        &page,
                        selected,
                        &w.route_codes,
                        layout,
                        target,
                        &Control::default(),
                    )
                    .and_then(|selection| {
                        let mut selections = page.region_languages.clone();
                        selections.push(selection);
                        crate::language_routing::commit_selections(
                            &project,
                            &page,
                            selections,
                            &Control::default(),
                        )
                    });
                    match result {
                        Ok(revision) => {
                            w.report =
                                format!("Region language selection saved as revision {revision}");
                            let _ = self.open_review(project.root.clone(), ctx);
                        }
                        Err(error) => w.report = error.to_string(),
                    }
                }
                if ui
                    .add_enabled(
                        ready && !page.region_languages.is_empty(),
                        egui::Button::new("Preview selected-language OCR"),
                    )
                    .clicked()
                {
                    let project = project.clone();
                    let page = page.clone();
                    let settings = self.state.settings.clone();
                    w.task(ctx, move || {
                        Ok(Report::Routing(
                            project.root.clone(),
                            crate::language_routing::reread_preview(
                                &project,
                                &page,
                                &page.region_languages,
                                &settings,
                                &Control::default(),
                            )?,
                        ))
                    });
                }
            });
            for selection in &page.region_languages {
                ui.label(format!(
                    "{} · {:?} · {:?}",
                    selection.languages, selection.layout, selection.region
                ));
            }
            if let Some(preview) = w.route_preview.clone() {
                ui.label(&preview.note);
                ui.label(format!(
                    "Preview: {} blocks, {} warnings",
                    preview.after.content.blocks.len(),
                    preview.after.content.warnings.len()
                ));
                ui.add(
                    egui::TextEdit::multiline(&mut preview.after.content.plain())
                        .desired_rows(4)
                        .interactive(false)
                        .desired_width(f32::INFINITY),
                );
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::new("Apply reviewed regional OCR preview"),
                    )
                    .clicked()
                {
                    match crate::language_routing::apply_preview(
                        &project,
                        &page,
                        &preview,
                        &preview.token,
                        &Control::default(),
                    ) {
                        Ok(revision) => {
                            w.route_preview = None;
                            w.report = format!("Regional OCR applied as revision {revision}");
                            let _ = self.open_review(project.root.clone(), ctx);
                        }
                        Err(error) => w.report = error.to_string(),
                    }
                }
            }
        });
        egui::CollapsingHeader::new("Repair handwritten line boundaries").show(ui,|ui| {
            ui.small("Confirm ordered convex quadrilaterals in normalized page coordinates. Original pixels stay unchanged; re-reading whites pixels outside the selected line.");
            if ui.add_enabled(region.is_some(),egui::Button::new("Start line draft from selected source bounds")).clicked(){let bounds=region.as_ref().unwrap();let quad=crate::geometry::Quad{points:[crate::geometry::Point{x:bounds.left,y:bounds.top},crate::geometry::Point{x:bounds.right,y:bounds.top},crate::geometry::Point{x:bounds.right,y:bounds.bottom},crate::geometry::Point{x:bounds.left,y:bounds.bottom}]};w.line_shapes=serde_json::to_string_pretty(&vec![quad]).unwrap_or_default();}
            ui.add(egui::TextEdit::multiline(&mut w.line_shapes).code_editor().char_limit(100_000).desired_rows(7).desired_width(f32::INFINITY)).accessible_name("Ordered line quadrilaterals JSON");
            if ui.add_enabled(ready && !w.line_shapes.trim().is_empty(),egui::Button::new("Validate and confirm line boundaries")).clicked(){let result=serde_json::from_str::<Vec<crate::geometry::Quad>>(&w.line_shapes).map_err(anyhow::Error::from).and_then(|quads|crate::geometry_repair::repair_lines(&project,&page,index,&quads,&Control::default()));match result{Ok(revision)=>{w.report=format!("Line boundaries saved as revision {revision}");let _=self.open_review(project.root.clone(),ctx);},Err(error)=>w.report=error.to_string()}}
            ui.add(egui::DragValue::new(&mut w.line_ordinal).range(0..=999).prefix("Zero-based line ")).accessible_name("Repaired line ordinal");
            if ui.add_enabled(ready && !w.reader.is_empty(),egui::Button::new("Re-read this repaired line with selected reader")).clicked(){let request=pipeline::LineReadRequest{page:page.page,block:index,ordinal:w.line_ordinal,reader_id:w.reader.clone(),task:"handwriting".into(),image_edge:w.region_edge,context:w.region_context.clone()};let settings=self.state.settings.clone();let project=project.clone();self.studio_task(ctx,move|c,log|{let revision=pipeline::reread_line(&project,&request,&settings,&c,&log)?;Ok(studio::StudioMessage::Done(format!("Repaired-line reading retained as revision {revision}; compare its evidence before accepting.")))});}
        });
        if let Some(block) = page
            .content
            .blocks
            .get(index)
            .filter(|block| block.kind == "table")
        {
            egui::CollapsingHeader::new("Source-overlaid table grid repair").show(ui,|ui| {
            if ui.add_enabled(region.is_some(),egui::Button::new("Create editable grid and cell mapping draft")).clicked(){let bounds=region.as_ref().unwrap();let rows=block.rows.len();let columns=block.rows.first().map_or(0,Vec::len);if rows>0 && columns>0{let grid=block.geometry.as_ref().and_then(|data|data.grid.clone()).unwrap_or_else(||crate::geometry::TableGrid{row_edges:(0..=rows).map(|row|bounds.top+(bounds.bottom-bounds.top)*row as f32/rows as f32).collect(),column_edges:(0..=columns).map(|column|bounds.left+(bounds.right-bounds.left)*column as f32/columns as f32).collect(),origin:crate::geometry::Origin::HumanConfirmed});let repair=crate::geometry_repair::GridRepair{grid,mapping:(0..rows).flat_map(|row|(0..columns).map(move|col|crate::geometry_repair::CellMove{from_row:row,from_col:col,to_row:row,to_col:col})).collect(),merges:block.merges.clone(),header_rows:block.header_rows};w.grid_repair=serde_json::to_string_pretty(&repair).unwrap_or_default();}}
            ui.small("A draft without measured grid geometry uses an evenly spaced proposal. Adjust boundaries against the source before applying. Every old cell must be mapped explicitly; new split cells start empty.");
            ui.add(egui::TextEdit::multiline(&mut w.grid_repair).code_editor().char_limit(200_000).desired_rows(10).desired_width(f32::INFINITY)).accessible_name("Table grid boundaries, merges and cell mappings JSON");
            if ui.add_enabled(ready && !w.grid_repair.trim().is_empty(),egui::Button::new("Preview repaired grid on the source")).clicked(){let result=serde_json::from_str::<crate::geometry_repair::GridRepair>(&w.grid_repair).map_err(anyhow::Error::from).and_then(|repair|crate::geometry_repair::preview_grid(&project,&page,index,&repair,&Control::default()));match result{Ok(preview)=>w.grid_preview=Some(preview),Err(error)=>w.report=error.to_string()}}
            if let Some(preview)=w.grid_preview.clone(){ui.label(&preview.note);ui.label("Previewed grid is overlaid in Review. Inspect the cell mapping and all retained values before applying.");if ui.add_enabled(ready,egui::Button::new("Apply reviewed grid repair")).clicked(){match crate::geometry_repair::apply_grid(&project,&page,&preview,&preview.token,&Control::default()){Ok(revision)=>{w.grid_preview=None;w.report=format!("Table grid saved as revision {revision}");let _=self.open_review(project.root.clone(),ctx);},Err(error)=>w.report=error.to_string()}}}
        });
        }
    }
    fn redaction_tools(&mut self, w: &mut Workbench, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.separator();
        ui.heading("Confirmed source masks and sharing");
        ui.label(crate::redactions::LIMITS);
        if ui
            .add_enabled(
                self.worker.is_none() && w.rx.is_none(),
                egui::Button::new("Prepare source pages before recognition"),
            )
            .clicked()
            && let Some(input) = rfd::FileDialog::new()
                .add_filter("Scans", &["pdf", "png", "jpg", "jpeg", "tif", "tiff"])
                .pick_file()
            && let Some(output) = rfd::FileDialog::new()
                .add_filter("Output naming hint", &["docx"])
                .set_file_name("prepared-notes.docx")
                .save_file()
        {
            let settings = self.state.settings.clone();
            self.studio_task(ctx,move|c,log|{
                let project=pipeline::prepare_for_review(&input,&output,&settings,&c,&log)?;
                Ok(studio::StudioMessage::Project(project,"Source pages prepared without OCR or AI. Inspect the source and confirm masks in Privacy tools before recognizing this prepared project.".into()))
            });
        }
        let Some((project, page, region, dirty)) = self.review.as_ref().map(|r| {
            (
                r.project.clone(),
                r.page.clone(),
                r.crop.clone().or_else(|| {
                    r.page
                        .content
                        .blocks
                        .get(r.selected)
                        .and_then(|b| b.bbox.clone())
                }),
                r.dirty || r.raw_open,
            )
        }) else {
            ui.label("Open a project page to confirm source masks or create a sharing copy.");
            return;
        };
        let ready = !dirty && self.worker.is_none() && w.rx.is_none();
        if project.root.join("prepared-input.json").is_file() {
            ui.label("Prepared source project. Recognition checks the original input and intended settings before reading its confirmed masked images.");
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new("Recognize confirmed prepared pages"),
                )
                .clicked()
                && let Some(output) = rfd::FileDialog::new()
                    .add_filter("Word", &["docx"])
                    .set_file_name("recognized-notes.docx")
                    .save_file()
            {
                let project = project.clone();
                let settings = self.state.settings.clone();
                self.studio_task(ctx, move |c, log| {
                    let done =
                        pipeline::recognize_prepared(&project, &output, &settings, &c, &log)?;
                    let notice = format!(
                        "Recognized {} prepared pages. {} need review. Word copy: {}",
                        done.pages,
                        done.review,
                        done.output.display()
                    );
                    Ok(studio::StudioMessage::Project(done.project, notice))
                });
            }
        }
        let reasons = [
            crate::redactions::Reason::PersonalInformation,
            crate::redactions::Reason::AccountInformation,
            crate::redactions::Reason::Address,
            crate::redactions::Reason::ConfirmedOpaqueCover,
            crate::redactions::Reason::Other,
        ];
        let labels = [
            "Personal information",
            "Account information",
            "Address",
            "Confirmed opaque cover",
            "Other",
        ];
        egui::ComboBox::from_label("Mask reason")
            .selected_text(labels[w.mask_reason.min(4)])
            .show_ui(ui, |ui| {
                for (i, label) in labels.iter().enumerate() {
                    ui.selectable_value(&mut w.mask_reason, i, *label);
                }
            });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    ready && region.is_some(),
                    egui::Button::new("Preview selected source region as mask"),
                )
                .clicked()
            {
                let spec = crate::redactions::MaskSpec {
                    region: region.clone().expect("enabled measured selection"),
                    reason: reasons[w.mask_reason.min(4)],
                };
                match crate::redactions::preview_confirmation(
                    &project,
                    &page,
                    &[spec],
                    &Control::default(),
                ) {
                    Ok(preview) => {
                        w.mask_preview = Some(preview);
                        w.mask_selected = [0].into_iter().collect();
                    }
                    Err(error) => w.report = error.to_string(),
                }
            }
            if ui
                .add_enabled(ready, egui::Button::new("Inspect possible opaque covers"))
                .clicked()
            {
                let proposals =
                    crate::redactions::effective_page(&project, &page, &Control::default())
                        .and_then(|(_, image, _)| {
                            crate::redactions::suggest_opaque_covers(&image, &Control::default())
                        })
                        .map(|items| {
                            items
                                .into_iter()
                                .map(|item| crate::redactions::MaskSpec {
                                    region: item.region,
                                    reason: crate::redactions::Reason::ConfirmedOpaqueCover,
                                })
                                .collect::<Vec<_>>()
                        });
                match proposals.and_then(|items| {
                    crate::redactions::preview_confirmation(
                        &project,
                        &page,
                        &items,
                        &Control::default(),
                    )
                }) {
                    Ok(preview) => {
                        w.mask_preview = Some(preview);
                        w.mask_selected.clear();
                    }
                    Err(error) => w.report = error.to_string(),
                }
            }
        });
        if let Some(preview) = w.mask_preview.clone() {
            ui.label(&preview.note);
            for (index, proposal) in preview.proposals.iter().enumerate() {
                let mut selected = w.mask_selected.contains(&index);
                if ui
                    .checkbox(
                        &mut selected,
                        format!(
                            "Cover {} · {:?} · {:?}",
                            index + 1,
                            proposal.reason,
                            proposal.region
                        ),
                    )
                    .changed()
                {
                    if selected {
                        w.mask_selected.insert(index);
                    } else {
                        w.mask_selected.remove(&index);
                    }
                }
            }
            if ui
                .add_enabled(
                    ready && !w.mask_selected.is_empty(),
                    egui::Button::new("Confirm selected source masks"),
                )
                .clicked()
            {
                let project = project.clone();
                let number = page.page;
                let selected = w.mask_selected.iter().copied().collect::<Vec<_>>();
                self.studio_task(ctx, move |c,_| {
                    let before = project.load(number)?;
                    let mut after = before.clone();
                    after.confirmed_redactions = crate::redactions::confirm(&project, &before, &preview, &preview.token, &selected, &c)?;
                    after = crate::redactions::effective_page(&project, &after, &c)?.0;
                    after.edited = true; after.reviewed = false;
                    crate::review::commit(&project, &before, &after, "Confirm source redactions")?;
                    Ok(studio::StudioMessage::Done("Source masks confirmed. Review and exports use the masked view. Private originals and undo history are retained.".into()))
                });
                w.mask_preview = None;
                w.mask_selected.clear();
                w.share_preview = None;
            }
        }
        for mask in &page.confirmed_redactions {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Confirmed {:?} · {:?}", mask.reason, mask.region));
                if ui.add_enabled(ready, egui::Button::new(format!("Remove mask {}", &mask.id[..8]))).clicked() {
                    let before = page.clone(); let mut after = before.clone();
                    after.confirmed_redactions.retain(|item| item.id != mask.id);
                    after.edited = true; after.reviewed = false;
                    match crate::review::commit(&project, &before, &after, "Remove source mask; text stays withheld") {
                        Ok(_) => { let _ = self.open_review(project.root.clone(), ctx); w.report = "Mask removed. Text remains withheld until explicitly re-read or restored through undo.".into(); w.share_preview = None; }
                        Err(error) => w.report = error.to_string(),
                    }
                }
            });
        }
        if ui
            .add_enabled(
                ready,
                egui::Button::new("Preview sharing copy of project pages"),
            )
            .clicked()
        {
            match crate::redactions::preview_share(
                &project,
                &project.manifest.pages,
                &Control::default(),
            ) {
                Ok(preview) => w.share_preview = Some(preview),
                Err(error) => w.report = error.to_string(),
            }
        }
        if let Some(preview) = w.share_preview.clone() {
            ui.label(format!(
                "{} pages · {} confirmed masks · {} withheld blocks",
                preview.pages.len(),
                preview.pages.iter().map(|p| p.masks).sum::<usize>(),
                preview
                    .pages
                    .iter()
                    .map(|p| p.withheld_blocks)
                    .sum::<usize>()
            ));
            ui.label(&preview.note);
            if ui
                .add_enabled(ready, egui::Button::new("Build reviewed sharing ZIP"))
                .clicked()
                && let Some(output) = rfd::FileDialog::new()
                    .add_filter("Sharing copy", &["zip"])
                    .set_file_name("sharing-copy.zip")
                    .save_file()
            {
                self.studio_task(ctx, move |c, _| {
                    let report = crate::redactions::export_share(
                        &project,
                        &preview,
                        &preview.token,
                        &output,
                        &c,
                    )?;
                    Ok(studio::StudioMessage::Done(serde_json::to_string_pretty(
                        &report,
                    )?))
                });
            }
        }
    }
    pub(super) fn layer_controls(&mut self, r: &mut Review, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Manual annotation layers").show(ui, |ui| {
            ui.small("Label the selected block as original text, a highlight, marginal note, stamp or correction. Labels retain all content. They do not remove ink or redact hidden text. Changed source bindings need explicit reconfirmation.");
            let kinds=[crate::layers::LayerKind::OriginalText,crate::layers::LayerKind::Highlight,crate::layers::LayerKind::MarginalNote,crate::layers::LayerKind::Stamp,crate::layers::LayerKind::TeacherCorrection,crate::layers::LayerKind::Other];
            self.workbench.layer_kind=self.workbench.layer_kind.min(kinds.len()-1);
            let editable=!r.dirty && !r.raw_open && self.worker.is_none();
            ui.add_enabled_ui(editable,|ui| {
                egui::ComboBox::from_label("Annotation kind").selected_text(kinds[self.workbench.layer_kind].name()).show_ui(ui,|ui|{
                    for (index,kind) in kinds.iter().enumerate(){ui.selectable_value(&mut self.workbench.layer_kind,index,kind.name());}
                });
                ui.add(egui::TextEdit::singleline(&mut self.workbench.layer_label).hint_text("Optional label")).accessible_name("Manual annotation label");
                ui.add(egui::TextEdit::multiline(&mut self.workbench.layer_note).desired_rows(2).hint_text("Optional annotation note")).accessible_name("Manual annotation note");
                if ui.button(format!("Label selected block {}",r.selected+1)).clicked(){
                    let result=crate::layers::set(&r.project,&r.page,r.selected,kinds[self.workbench.layer_kind],&self.workbench.layer_label,&self.workbench.layer_note,None).and_then(|_|r.reload());
                    match result{Ok(())=>self.review_error.clear(),Err(e)=>self.review_error=e.to_string()}
                }
            });
            if !r.layer_notice.is_empty(){Self::accessible_status(ui,"annotation-error",&r.layer_notice,true);}
            if r.dirty{ui.small("Labels below refer to saved source bindings. Save the draft to check changes.");}
            {
                    let layers=r.layer_cache.clone();
                    if layers.len()>100{ui.label(format!("Showing 100 of {} labels. The CLI layers report includes all labels.",layers.len()));}
                    egui::ScrollArea::vertical().id_salt("annotation-layers").max_height(180.).show(ui,|ui|{
                        for layer in layers.into_iter().take(100){
                            ui.horizontal_wrapped(|ui|{
                                ui.label(format!("Block {} · {} · {} · {}",layer.assignment.block+1,layer.assignment.kind.name(),layer.assignment.label,layer.status));
                                if ui.add_enabled(editable,egui::Button::new("Remove label")).clicked(){
                                    match crate::layers::remove(&r.project,&r.page,&layer.assignment.id).and_then(|_|r.reload()) {Ok(())=>self.review_error.clear(),Err(e)=>self.review_error=e.to_string()}
                                }
                                if !layer.confirmed() && ui.add_enabled(editable,egui::Button::new("Reconfirm on selected block")).clicked(){
                                    match crate::layers::reconfirm(&r.project,&r.page,&layer.assignment.id,r.selected).and_then(|_|r.reload()) {Ok(())=>self.review_error.clear(),Err(e)=>self.review_error=e.to_string()}
                                }
                            });
                        }
                    });
            }
        });
    }
    fn review_shortcut(&self, action: ReviewAction) -> egui::KeyboardShortcut {
        self.state.settings.ui.shortcuts.egui(match action {
            ReviewAction::Save => crate::shortcuts::Action::Save,
            ReviewAction::Approve => crate::shortcuts::Action::Approve,
            ReviewAction::NextIssue => crate::shortcuts::Action::NextIssue,
            ReviewAction::Defer => crate::shortcuts::Action::Defer,
            ReviewAction::Undo => crate::shortcuts::Action::Undo,
            ReviewAction::Redo => crate::shortcuts::Action::Redo,
        })
    }
    pub(super) fn review_action_label(&self, ctx: &egui::Context, action: ReviewAction) -> String {
        let name = match action {
            ReviewAction::Save => "Save draft",
            ReviewAction::Approve => "Save & mark reviewed",
            ReviewAction::NextIssue => "Next issue",
            ReviewAction::Defer => "Defer selected issue",
            ReviewAction::Undo => "Undo revision",
            ReviewAction::Redo => "Redo revision",
        };
        format!(
            "{name} · {}",
            ctx.format_shortcut(&self.review_shortcut(action))
        )
    }
    pub(super) fn review_action(
        &mut self,
        r: &mut Review,
        action: ReviewAction,
        ctx: &egui::Context,
    ) -> Result<()> {
        anyhow::ensure!(
            self.worker.is_none(),
            Fault::new("NG500", "Wait for the current task before reviewing")
        );
        anyhow::ensure!(
            !r.raw_open,
            Fault::new(
                "NG500",
                "Apply page JSON and close its editor before reviewing"
            )
        );
        match action {
            ReviewAction::Save => r.save(false),
            ReviewAction::Approve => {
                r.save(true)?;
                if r.auto_advance && r.index + 1 < r.project.manifest.pages.len() {
                    r.navigate(r.index + 1, 0, ctx).map_err(|e| {
                        Fault::new(
                            "NG500",
                            format!("Page approval was saved. The next page could not open: {e}"),
                        )
                    })?;
                }
                Ok(())
            }
            ReviewAction::NextIssue => r.next_issue(ctx),
            ReviewAction::Defer => r.defer_selected(ctx),
            ReviewAction::Undo => r.history_action(false),
            ReviewAction::Redo => r.history_action(true),
        }
    }
    pub(super) fn review_shortcut_controls(
        &self,
        r: &Review,
        ui: &mut egui::Ui,
    ) -> Option<ReviewAction> {
        let response = ui
            .scope_builder(
                egui::UiBuilder::new().scope_id(egui::Id::unique("review-keyboard-scope")),
                |ui| ui.button("Keyboard review · focus here for shortcuts"),
            )
            .inner;
        if response.clicked() {
            response.request_focus();
        }
        let ctx = ui.ctx();
        let can_use = response.has_focus()
            && ui.is_enabled()
            && !ctx.text_edit_focused()
            && !egui::Popup::is_any_open(ctx)
            && ctx.top_layer_id() == Some(ui.layer_id())
            && self.worker.is_none()
            && !r.raw_open
            && !self.advanced
            && !self.experience.open
            && !self.studio.open
            && self.studio.pending.is_none()
            && !self.workbench.open
            && !self.workbench.palette;
        response.on_hover_text("Tab or click to focus this button. Shortcuts run only with this button focused, no other tools open, and page JSON closed. Tab to an editor to type normally.");
        if can_use {
            for action in [
                ReviewAction::Save,
                ReviewAction::Approve,
                ReviewAction::NextIssue,
                ReviewAction::Defer,
                ReviewAction::Undo,
                ReviewAction::Redo,
            ] {
                if ctx.input_mut(|i| {
                    let shortcut = self.review_shortcut(action);
                    i.events.iter().any(|event| {
                        matches!(event,
                            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                            if *key == shortcut.logical_key && modifiers.alt==shortcut.modifiers.alt && modifiers.shift==shortcut.modifiers.shift
                                && modifiers.matches_logically(shortcut.modifiers)
                        )
                    }) && i.consume_shortcut(&shortcut)
                }) {
                    return Some(action);
                }
            }
        }
        None
    }
    pub(super) fn repeated_review_activation(ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Enter | egui::Key::Space,
                        pressed: true,
                        repeat: true,
                        ..
                    }
                )
            })
        })
    }
    fn export_tools(&mut self, w: &mut Workbench, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((project, dirty)) = self
            .review
            .as_ref()
            .map(|r| (r.project.clone(), r.dirty || r.raw_open))
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
            ui.add(egui::TextEdit::singleline(&mut w.find).char_limit(1000))
                .accessible_name("Correction rule: exact text to find");
            ui.label("Replace");
            ui.add(egui::TextEdit::singleline(&mut w.replace).char_limit(1000))
                .accessible_name("Correction rule: replacement text");
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
        egui::CollapsingHeader::new("Chemical notation checks").show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut w.notation)
                    .desired_rows(2)
                    .char_limit(4096)
                    .hint_text("H₂SO₄, [Fe(CN)₆]³⁻, or 2H₂ + O₂ → 2H₂O"),
            )
            .accessible_name("Chemical formula or reaction to validate");
            ui.checkbox(
                &mut w.notation_reaction,
                "This is a reaction with spaced + separators",
            );
            if ui.button("Check symbols, counts and charges").clicked() {
                let result = if w.notation_reaction {
                    crate::chemistry::reaction(&w.notation).and_then(|report| {
                        serde_json::to_string_pretty(&report).map_err(Into::into)
                    })
                } else {
                    crate::chemistry::formula(&w.notation).and_then(|report| {
                        serde_json::to_string_pretty(&report).map_err(Into::into)
                    })
                };
                w.report = result.unwrap_or_else(|error| error.to_string());
            }
            ui.small(crate::chemistry::LIMITS);
        });
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
        )
        .accessible_name("Optional recognition context for the selected source region");
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
            ui.add(egui::DragValue::new(&mut w.reserved_cores).range(0..=511))
                .accessible_name("CPU cores to leave available for other applications");
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
        output.response.response = output.response.response.accessible_name("Private-update bearer token. Masked input; stored only after choosing Store token in Windows.");
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
            let can_use = !r.dirty && !r.raw_open && self.worker.is_none();
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
            )
            .accessible_name("Name the next saved review revision");
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
                                match r.visible_revision_comparison(revision.id) {Ok(text)=>self.review_error=text,Err(error)=>self.review_error=error.to_string()}
                            }
                            if ui.add_enabled(!r.dirty && !r.raw_open && self.worker.is_none(),egui::Button::new("Restore")).clicked() {
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
        let count = r.issues.iter().filter(|i| i.page > 0).count();
        egui::CollapsingHeader::new(format!("Review queue · {count} suggestions · {} deferred", r.deferred_issues.len()))
            .show(ui, |ui| {
                ui.small("Suggestions never rewrite values. Deferrals last only for this open project session; all issues and source overlays remain visible. Next issue skips deferred items and wraps through the ranked queue.");
                if !r.issue_notice.is_empty() {
                    ui.colored_label(Color32::from_rgb(241, 187, 111), &r.issue_notice);
                }
                for notice in r.issues.iter().filter(|i| i.page == 0) {
                    ui.colored_label(Color32::from_rgb(241, 187, 111), &notice.message);
                }
                let can_use = !r.dirty && !r.raw_open && self.worker.is_none();
                ui.horizontal_wrapped(|ui| {
                    for action in [ReviewAction::NextIssue, ReviewAction::Defer] {
                        let available = can_use && match action {
                            ReviewAction::Defer => r.selected_issue().is_some(),
                            _ => r.issues.iter().any(|i| i.page > 0 && !r.deferred_issues.contains(&i.id)),
                        };
                        if ui.add_enabled(available, egui::Button::new(self.review_action_label(ui.ctx(), action))).clicked() {
                            match self.review_action(r, action, ui.ctx()) {
                                Ok(()) => self.review_error.clear(),
                                Err(e) => self.review_error = e.to_string(),
                            }
                        }
                    }
                    if ui.add_enabled(!r.deferred_issues.is_empty(), egui::Button::new("Reset session deferrals")).clicked() {
                        r.deferred_issues.clear();
                        r.active_issue = None;
                    }
                });
                if let Some(issue) = r.selected_issue() {
                    ui.strong(format!("Selected: {}", issue.message));
                    if let (Some(row), Some(col)) = (issue.row, issue.col) {
                        ui.small(format!("Table row {}, column {}", row + 1, col + 1));
                    }
                    ui.label(&issue.reading);
                    for alternative in &issue.alternatives {
                        ui.small(format!("Reader alternative: {alternative}"));
                    }
                }
                let pages = count.div_ceil(100).max(1);
                r.issue_page = r.issue_page.min(pages - 1);
                ui.horizontal(|ui| {
                    if ui.add_enabled(r.issue_page > 0, egui::Button::new("Previous issues")).clicked() { r.issue_page -= 1; }
                    ui.small(format!("{} / {pages}", r.issue_page + 1));
                    if ui.add_enabled(r.issue_page + 1 < pages, egui::Button::new("Next issues")).clicked() { r.issue_page += 1; }
                });
                let mut chosen = None;
                let mut toggled = None;
                for issue in r.issues.iter().filter(|i| i.page > 0).skip(r.issue_page * 100).take(100) {
                    let deferred = r.deferred_issues.contains(&issue.id);
                    ui.horizontal_wrapped(|ui| {
                        let label = format!("{}P{} · page {} · {}", if deferred { "Deferred · " } else { "" }, issue.priority, issue.page, issue.message);
                        if ui.add_enabled(can_use, egui::Button::new(label)).clicked() { chosen = Some(issue.id.clone()); }
                        if ui.add_enabled(can_use, egui::Button::new(if deferred { "Restore issue" } else { "Defer issue" })).clicked() { toggled = Some((issue.id.clone(), deferred)); }
                    });
                }
                if let Some((id, was_deferred)) = toggled {
                    if was_deferred { r.deferred_issues.remove(&id); }
                    else { r.deferred_issues.insert(id); }
                }
                if let Some(id) = chosen {
                    match r.open_issue(&id, ui.ctx()) {
                        Ok(()) => self.review_error.clear(),
                        Err(e) => self.review_error = e.to_string(),
                    }
                }
            });
    }
    pub fn tools_open(&mut self, tab: usize) {
        self.workbench.open = true;
        self.workbench.tab = tab.min(3);
        self.workbench.catalog_loaded = false;
    }
    pub(super) fn global_shortcut(
        &self,
        ctx: &egui::Context,
        action: crate::shortcuts::Action,
    ) -> bool {
        if ctx.text_edit_focused() || egui::Popup::is_any_open(ctx) || self.vault.blocks_close() {
            return false;
        }
        let shortcut = self.state.settings.ui.shortcuts.egui(action);
        ctx.input_mut(|input| input.events.iter().any(|event|matches!(event,egui::Event::Key{key,pressed:true,repeat:false,modifiers,..} if *key==shortcut.logical_key && modifiers.alt==shortcut.modifiers.alt && modifiers.shift==shortcut.modifiers.shift && modifiers.matches_logically(shortcut.modifiers))) && input.consume_shortcut(&shortcut))
    }
    pub(super) fn workbench_ui(&mut self, ctx: &egui::Context) {
        self.workbench.poll();
        if self.global_shortcut(ctx, crate::shortcuts::Action::Palette) {
            self.workbench.palette = true;
            self.workbench.palette_focus = true;
        }
        if self.workbench.palette {
            let mut open = true;
            egui::Window::new(format!(
                "Command palette · {}",
                ctx.format_shortcut(
                    &self
                        .state
                        .settings
                        .ui
                        .shortcuts
                        .egui(crate::shortcuts::Action::Palette)
                )
            ))
            .open(&mut open)
            .default_width(440.)
            .show(ctx, |ui| {
                let search = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.workbench.palette_query)
                            .hint_text("Find an action…"),
                    )
                    .accessible_name("Command palette: find an action");
                if self.workbench.palette_focus {
                    search.request_focus();
                    self.workbench.palette_focus = false;
                }
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
                            ui.text_edit_singleline(&mut w.query).accessible_name("Filter projects or search recognized text");
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
                            if !w.metadata.is_empty(){ui.add(egui::TextEdit::multiline(&mut w.metadata).code_editor().desired_rows(12).desired_width(f32::INFINITY)).accessible_name("Project details JSON. Validate and save to apply changes.");if ui.add_enabled(self.worker.is_none(),egui::Button::new("Validate and save project details")).clicked(){match serde_json::from_str(&w.metadata).map_err(anyhow::Error::from).and_then(|m|crate::projects::save_metadata(&r.project,&m)){Ok(())=>w.report="Project details saved.".into(),Err(e)=>w.report=e.to_string()}}}
                        }else{ui.label("Open a project to inspect recovery or attach references.");}
                        self.region_reread_tools(&mut w,ui,ctx);
                        self.geometry_tools(&mut w,ui,ctx);
                    }
                    _=>{
                        ui.heading("Local activity and data");
                        if ui.button("Encrypted project workspace").clicked() { self.release_warm_reader(); self.vault.open(); }
                        let connections = crate::connections::snapshot();
                        ui.label(format!("{} active NoteGen HTTP request(s)",connections.active.len()+connections.additional_active));
                        for connection in &connections.active { ui.label(format!("{:?} · {} · {:.1}s",connection.purpose,connection.destination,connection.seconds)); }
                        ui.small(connections.note);
                        ui.label(format!("Recognition endpoint: {}",self.state.settings.endpoint));
                        ui.label(format!("Models: {}",self.state.settings.models_dir.display()));
                        ui.label(format!("Writer examples: {}",self.state.settings.writer.root.display()));
                        ui.label(format!("Automatic update checks: {} · active check: {}",self.state.settings.experience.check_updates,self.experience.checking));
                        ui.label(format!("Optional public lookup: {}",self.state.settings.assistance.web_lookup));
                        ui.label("Recognition uses a literal loopback connection. Publisher downloads and update checks may use the internet. Public lookup sends the typed query only when enabled.");
                        ui.separator();ui.heading("Readable layout");
                        ui.horizontal_wrapped(|ui|{
                            if ui.button("Large text").clicked(){self.state.settings.ui.scale=1.4;self.state.settings.ui.compact=false;}
                            if ui.button("Spacious").clicked(){self.state.settings.ui.scale=1.1;self.state.settings.ui.compact=false;}
                            if ui.button("Compact").clicked(){self.state.settings.ui.scale=1.;self.state.settings.ui.compact=true;}
                            if ui.button("High contrast").clicked(){self.state.settings.ui.dark=true;self.state.settings.ui.background=[0,0,0];self.state.settings.ui.accent=[255,255,0];self.state.settings.ui.reduced_motion=true;}
                            if ui.button("Default appearance").clicked(){let locale=self.state.settings.ui.locale;self.state.settings.ui=Appearance::default();self.state.settings.ui.locale=locale;}
                        });
                        ui.small(format!("{} opens the command palette. Tab moves focus, Enter activates buttons. Review includes source zoom and disagreement markers.",ctx.format_shortcut(&self.state.settings.ui.shortcuts.egui(crate::shortcuts::Action::Palette))));
                        egui::CollapsingHeader::new("Keyboard shortcuts").show(ui,|ui| {
                            let draft=w.shortcut_draft.get_or_insert_with(||self.state.settings.ui.shortcuts.clone());
                            for (action,label) in crate::shortcuts::ACTIONS { ui.horizontal(|ui|{ui.label(label);ui.add(egui::TextEdit::singleline(draft.get_mut(action)).char_limit(40).desired_width(170.)).accessible_name(format!("Shortcut for {label}"));}); }
                            ui.small("Use Ctrl, Alt, Shift and one key, for example Ctrl+Shift+S or F6. Ctrl maps to the primary Command key where applicable. Text editors keep their editing shortcuts.");
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Apply keyboard shortcuts").clicked() {match draft.validate(){Ok(())=>{self.state.settings.ui.shortcuts=draft.clone();w.report="Keyboard shortcuts applied. Review actions require the keyboard-review focus button.".into();},Err(error)=>w.report=error.to_string()}}
                                if ui.button("Reset shortcut draft to defaults").clicked(){*draft=Default::default();}
                                if ui.button("Reload saved shortcuts").clicked(){*draft=self.state.settings.ui.shortcuts.clone();}
                            });
                        });
                        self.resource_tools(&mut w,ui,ctx);
                        self.credential_tools(&mut w,ui,ctx);
                        self.redaction_tools(&mut w,ui,ctx);
                    }
                }
                if !w.report.is_empty(){ui.separator();Self::accessible_status(ui,"workbench-task-result","Local tool result is available below.",false);ui.add(egui::TextEdit::multiline(&mut w.report).desired_rows(10).desired_width(f32::INFINITY)).accessible_name("Local tool result details");}
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
            && let Err(e) = r.navigate(index, block, ctx)
        {
            self.notice = e.to_string();
        }
    }
}
