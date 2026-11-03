//! Ephemeral encrypted-workspace UI. Never serialized into the desktop Workspace.
use super::*;
use crate::vault::{self, Credentials, SecretString, WorkCopy};
use std::{sync::Mutex, time::Duration};
use zeroize::{Zeroize, Zeroizing};

const EDITOR_ID: &str = "encrypted-workspace-page-editor";
const EDITOR_CHARS: usize = 200_000;
enum OwnedCredentials {
    Passphrase(SecretString),
    Recovery(PathBuf),
}
impl OwnedCredentials {
    fn borrowed(&self) -> Credentials<'_> {
        match self {
            Self::Passphrase(value) => Credentials::Passphrase(value),
            Self::Recovery(path) => Credentials::RecoveryKey(path),
        }
    }
}
struct Loaded {
    raw: Zeroizing<String>,
    expected: String,
    page: usize,
    index: usize,
    count: usize,
    image: egui::ColorImage,
}
struct Editor {
    loaded: Loaded,
    texture: egui::TextureHandle,
    dirty: bool,
}
enum Plain {
    Opened(Box<WorkCopy>),
    Text(String),
}
enum CopyUpdate {
    Editor(Loaded),
    Text(String),
    DraftRetained(PathBuf),
    Locked(vault::LockReport),
}
enum Reply {
    Plain(std::result::Result<Plain, String>),
    Copy(Box<WorkCopy>, std::result::Result<CopyUpdate, String>),
}
#[derive(Default)]
pub(super) struct VaultUi {
    open: bool,
    close_requested: bool,
    retain_confirmed: bool,
    retained_pending: bool,
    managed_path: Option<PathBuf>,
    container: String,
    original_project: Zeroizing<String>,
    recovery_key: String,
    work_parent: String,
    retained_folder: String,
    input: Zeroizing<String>,
    passphrase: Zeroizing<String>,
    replacement: Zeroizing<String>,
    confirmation: Zeroizing<String>,
    use_recovery: bool,
    offline_key_permissions: bool,
    inherited_work_permissions: bool,
    copy: Option<WorkCopy>,
    editor: Option<Editor>,
    worker: Option<thread::JoinHandle<()>>,
    rx: Option<mpsc::Receiver<Reply>>,
    control: Control,
    error: String,
    report: Zeroizing<String>,
}
fn load_page(copy: &WorkCopy, index: usize, c: &Control) -> Result<Loaded> {
    let _worker = copy.begin_worker()?;
    c.check()?;
    let project = Project::read(copy.project_path())?;
    let number = *project
        .manifest
        .pages
        .get(index)
        .ok_or_else(|| Fault::new("NG682", "This workspace has no selected page to edit"))?;
    let mut page = project.load(number)?;
    if !page.confirmed_redactions.is_empty() {
        let (mut effective, _, _) = crate::redactions::effective_page(&project, &page, c)?;
        if fingerprint(&effective)? != fingerprint(&page)? {
            effective.edited = true;
            effective.reviewed = false;
            crate::review::commit(
                &project,
                &page,
                &effective,
                "Apply confirmed mask visibility in encrypted workspace",
            )?;
            page = project.load(number)?;
        }
    }
    let raw = Zeroizing::new(serde_json::to_string_pretty(&page.content)?);
    anyhow::ensure!(
        raw.chars().count() <= EDITOR_CHARS,
        Fault::new(
            "NG682",
            "This page exceeds the bounded 200,000-character workspace editor. Its source and data remain retained; encrypted save, conversion and export are still available."
        )
    );
    let image = review_source(&project, &page)?
        .thumbnail(900, 900)
        .to_rgba8();
    Ok(Loaded {
        raw,
        expected: fingerprint(&page)?,
        page: number,
        index,
        count: project.manifest.pages.len(),
        image: egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
    })
}
fn external_output(copy: &WorkCopy, output: &Path) -> Result<PathBuf> {
    crate::watch::no_links(output)?;
    let absolute = std::path::absolute(output)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| Fault::new("NG682", "Choose an external export filename"))?
        .canonicalize()?;
    let name = absolute
        .file_name()
        .ok_or_else(|| Fault::new("NG682", "Choose an external export filename"))?;
    let output = parent.join(name);
    anyhow::ensure!(
        !output.starts_with(copy.work_root()),
        Fault::new("NG682", "Export outside the managed plaintext working copy")
    );
    Ok(output)
}
impl VaultUi {
    pub(super) fn open(&mut self) {
        self.open = true;
    }
    pub(super) fn blocks_close(&self) -> bool {
        self.copy.is_some() || self.worker.is_some() || self.retained_pending
    }
    pub(super) fn request_close(&mut self) {
        self.open = true;
        self.close_requested = true;
    }
    fn clear_secrets(&mut self, ctx: &egui::Context) {
        self.passphrase.zeroize();
        self.replacement.zeroize();
        self.confirmation.zeroize();
        ctx.data_mut(|data| {
            for name in [
                "vault-passphrase",
                "vault-replacement",
                "vault-confirmation",
            ] {
                data.remove::<egui::text_edit::TextEditState>(egui::Id::unique(name));
            }
        });
    }
    fn clear_editor(&mut self, ctx: &egui::Context) {
        self.editor = None;
        ctx.data_mut(|data| {
            data.remove::<egui::text_edit::TextEditState>(egui::Id::unique(EDITOR_ID));
        });
    }
    fn credentials(&mut self, ctx: &egui::Context) -> Result<OwnedCredentials> {
        if self.use_recovery {
            anyhow::ensure!(
                !self.recovery_key.trim().is_empty(),
                Fault::new("NG682", "Select the separate recovery key file")
            );
            Ok(OwnedCredentials::Recovery(PathBuf::from(
                &self.recovery_key,
            )))
        } else {
            let secret = SecretString::from(std::mem::take(&mut *self.passphrase));
            self.clear_secrets(ctx);
            Ok(OwnedCredentials::Passphrase(secret))
        }
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        op: impl FnOnce(&Control) -> Result<Plain> + Send + 'static,
    ) {
        if self.worker.is_some() || self.copy.is_some() {
            return;
        }
        self.control = Control::default();
        let c = self.control.clone();
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        let result = thread::Builder::new().name("notegen-vault".into()).stack_size(8*1024*1024).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| op(&c)))
                .map_err(|_| "Encrypted workspace operation stopped unexpectedly; existing originals and committed containers are preserved.".to_string())
                .and_then(|value| value.map_err(|error|error.to_string()));
            let _ = tx.send(Reply::Plain(result)); repaint.request_repaint();
        });
        match result {
            Ok(worker) => {
                self.worker = Some(worker);
                self.rx = Some(rx);
                self.error.clear();
            }
            Err(error) => self.error = format!("Cannot start encrypted workspace task: {error}"),
        }
    }
    fn copy_task(
        &mut self,
        ctx: &egui::Context,
        op: impl FnOnce(&mut WorkCopy, &Control) -> Result<CopyUpdate> + Send + 'static,
    ) {
        if self.worker.is_some() {
            return;
        }
        let Some(copy) = self.copy.take() else {
            return;
        };
        // Keep ownership recoverable even if the OS cannot create a worker.
        let owner = Arc::new(Mutex::new(Some(copy)));
        let task_owner = owner.clone();
        self.control = Control::default();
        let c = self.control.clone();
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        let result = thread::Builder::new()
            .name("notegen-vault-work".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut copy = task_owner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .expect("single workspace owner");
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| op(&mut copy, &c)))
                        .map_err(|_| {
                            format!(
                                "Workspace task stopped. Plaintext is retained at {}.",
                                copy.work_root().display()
                            )
                        })
                        .and_then(|value| value.map_err(|error| error.to_string()));
                let _ = tx.send(Reply::Copy(Box::new(copy), result));
                repaint.request_repaint();
            });
        match result {
            Ok(worker) => {
                self.worker = Some(worker);
                self.rx = Some(rx);
                self.error.clear();
            }
            Err(error) => {
                self.copy = owner.lock().unwrap_or_else(|e| e.into_inner()).take();
                self.error =
                    format!("Cannot start workspace task: {error}; the unlocked copy is retained.");
            }
        }
    }
    fn install_editor(&mut self, loaded: Loaded, ctx: &egui::Context) {
        self.clear_editor(ctx);
        let texture = ctx.load_texture(
            "encrypted-workspace-source",
            loaded.image.clone(),
            egui::TextureOptions::LINEAR,
        );
        self.editor = Some(Editor {
            loaded,
            texture,
            dirty: false,
        });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else {
            return;
        };
        let reply = match rx.try_recv() {
            Ok(reply) => reply,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error="Workspace worker ended without a result. Inspect the previously displayed private working-copy path; no plaintext cleanup is inferred.".into();
                self.rx = None;
                self.retained_pending = self.managed_path.is_some();
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                return;
            }
        };
        self.rx = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        match reply {
            Reply::Plain(Ok(Plain::Opened(copy))) => {
                self.managed_path = Some(copy.work_root().to_owned());
                self.report = Zeroizing::new(format!(
                    "Unlocked private working copy: {}. Owner-only permissions verified: {}. This path contains plaintext until a successful lock and cleanup.",
                    copy.work_root().display(),
                    copy.private_permissions()
                ));
                self.copy = Some(*copy);
                self.copy_task(ctx, |copy, c| {
                    Ok(CopyUpdate::Editor(load_page(copy, 0, c)?))
                });
            }
            Reply::Plain(Ok(Plain::Text(text))) => self.report = Zeroizing::new(text),
            Reply::Plain(Err(error)) => self.error = error,
            Reply::Copy(copy, result) => {
                self.copy = Some(*copy);
                match result {
                    Ok(CopyUpdate::Editor(loaded)) => self.install_editor(loaded, ctx),
                    Ok(CopyUpdate::Text(text)) => self.report = Zeroizing::new(text),
                    Ok(CopyUpdate::DraftRetained(path)) => {
                        self.clear_editor(ctx);
                        self.report = Zeroizing::new(format!(
                            "Unsaved editor JSON preserved separately at {}. The page checkpoint is unchanged. This draft will be included by the next encrypted snapshot; reload a page to continue editing.",
                            path.display()
                        ));
                    }
                    Ok(CopyUpdate::Locked(report)) => {
                        self.clear_editor(ctx);
                        self.clear_secrets(ctx);
                        self.original_project.zeroize();
                        self.input.zeroize();
                        self.report=Zeroizing::new(serde_json::to_string_pretty(&report).unwrap_or_else(|_|"Encrypted commit finished; inspect its cleanup report before treating plaintext as removed.".into()));
                        self.copy = None;
                        self.retained_pending = !report.fully_locked;
                        self.close_requested = self.retained_pending;
                        self.retain_confirmed = false;
                        if report.fully_locked {
                            self.managed_path = None;
                        }
                        if !report.fully_locked {
                            self.error="Encrypted snapshot saved, but plaintext remains at the listed paths. The workspace is not fully locked.".into();
                        }
                    }
                    Err(error) => {
                        self.error = format!(
                            "{error}. The unlocked copy remains available; retry or preserve it explicitly."
                        )
                    }
                }
            }
        }
    }
    fn save_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = &self.editor else {
            return;
        };
        let raw = Zeroizing::new(editor.loaded.raw.to_string());
        let expected = editor.loaded.expected.clone();
        let index = editor.loaded.index;
        self.copy_task(ctx, move |copy, c| {
            {
                let _worker = copy.begin_worker()?;
                c.check()?;
                let project = Project::read(copy.project_path())?;
                let number = *project
                    .manifest
                    .pages
                    .get(index)
                    .ok_or_else(|| Fault::new("NG682", "Page selection changed"))?;
                let before = project.load(number)?;
                anyhow::ensure!(
                    fingerprint(&before)? == expected,
                    Fault::new(
                        "NG682",
                        "Page changed; preserve this draft before reloading"
                    )
                );
                let content: PageContent = serde_json::from_str(&raw)?;
                content.validate()?;
                let mut next = before.clone();
                next.content = content;
                if !next.confirmed_redactions.is_empty() {
                    next = crate::redactions::effective_page(&project, &next, c)?.0;
                }
                next.edited = true;
                next.reviewed = false;
                crate::review::commit(&project, &before, &next, "Encrypted workspace edit")?;
            }
            Ok(CopyUpdate::Editor(load_page(copy, index, c)?))
        });
    }
    fn unlock(&mut self, ctx: &egui::Context) {
        let path = PathBuf::from(&self.container);
        let parent = PathBuf::from(&self.work_parent);
        let replacement = if self.use_recovery {
            if self.replacement.as_str() != self.confirmation.as_str() {
                self.error = "Replacement passphrases do not match.".into();
                return;
            }
            Some(SecretString::from(std::mem::take(&mut *self.replacement)))
        } else {
            None
        };
        let credentials = match self.credentials(ctx) {
            Ok(value) => value,
            Err(error) => {
                self.error = error.to_string();
                return;
            }
        };
        self.clear_secrets(ctx);
        let options = vault::Options {
            allow_inherited_permissions: self.inherited_work_permissions,
            ..Default::default()
        };
        self.start(ctx, move |c| {
            Ok(Plain::Opened(Box::new(vault::unlock(
                &path,
                &credentials.borrowed(),
                &parent,
                replacement.as_ref(),
                &options,
                c,
            )?)))
        });
    }
    fn create(&mut self, ctx: &egui::Context) {
        if self.passphrase.as_str() != self.confirmation.as_str() {
            self.error = "New passphrases do not match.".into();
            return;
        }
        let project = PathBuf::from(self.original_project.as_str());
        let output = PathBuf::from(&self.container);
        let key = PathBuf::from(&self.recovery_key);
        let passphrase = SecretString::from(std::mem::take(&mut *self.passphrase));
        self.clear_secrets(ctx);
        self.start(ctx, move |c| {
            Ok(Plain::Text(serde_json::to_string_pretty(&vault::create(
                &project,
                &output,
                &passphrase,
                &key,
                &vault::Options::default(),
                c,
            )?)?))
        });
    }
    fn backup(&mut self, ctx: &egui::Context, restoring: bool) {
        let Some(output) = rfd::FileDialog::new()
            .set_file_name("workspace-backup.ngvault")
            .save_file()
        else {
            return;
        };
        let path = PathBuf::from(&self.container);
        let credentials = match self.credentials(ctx) {
            Ok(value) => value,
            Err(error) => {
                self.error = error.to_string();
                return;
            }
        };
        self.clear_secrets(ctx);
        self.start(ctx, move |c| {
            let receipt = if restoring {
                vault::restore(&path, &output, &credentials.borrowed(), c)?
            } else {
                vault::backup(&path, &output, &credentials.borrowed(), c)?
            };
            Ok(Plain::Text(serde_json::to_string_pretty(&receipt)?))
        });
    }
    fn lock(&mut self, ctx: &egui::Context) {
        if self.editor.as_ref().is_some_and(|editor| editor.dirty) {
            self.error="Save the page draft before saving the encrypted snapshot, or explicitly retain its plaintext working copy.".into();
            return;
        }
        self.copy_task(ctx, |copy, c| {
            Ok(CopyUpdate::Locked(
                copy.commit_and_lock(Duration::from_secs(30), c)?,
            ))
        });
    }
    fn preserve_raw_draft(&mut self, ctx: &egui::Context) {
        let Some(editor) = &self.editor else {
            return;
        };
        let raw = Zeroizing::new(editor.loaded.raw.to_string());
        self.copy_task(ctx, move |copy, c| {
            use std::io::Write;
            let _worker = copy.begin_worker()?;
            c.check()?;
            let parent = copy.work_root().join("work");
            crate::watch::no_links(&parent)?;
            let mut file = tempfile::Builder::new()
                .prefix("unsaved-editor-")
                .suffix(".json")
                .tempfile_in(&parent)?;
            file.write_all(raw.as_bytes())?;
            file.flush()?;
            file.as_file().sync_all()?;
            c.check()?;
            let (_, path) = file.keep().map_err(|error| error.error)?;
            Ok(CopyUpdate::DraftRetained(path))
        });
    }
    pub(super) fn ui(&mut self, ctx: &egui::Context, settings: &Settings) {
        self.poll(ctx);
        if !self.open {
            return;
        }
        let mut open = true;
        let bounds = ctx.content_rect().shrink(8.);
        egui::Window::new("Encrypted workspace").id(egui::Id::unique("notegen-vault-window"))
            .open(&mut open).default_size([950.,730.]).max_height(bounds.height()).constrain_to(bounds).vscroll(true)
            .show(ctx,|ui|{
                ui.label("Opt in to an authenticated encrypted snapshot. Original projects and external exports remain where you put them. Passwords and this editor are never saved in desktop preferences.");
                if !self.error.is_empty(){Desktop::accessible_status(ui,"vault-error",&self.error,true);}
                if self.worker.is_some(){
                    Desktop::accessible_status(ui,"vault-worker-status","Workspace task running. Closing is blocked until it finishes or returns a retained-copy result.",false);
                    if ui.button("Cancel workspace task").clicked(){self.control.cancelled.store(true,Ordering::Relaxed);}
                }
                let idle=self.worker.is_none();
                if self.copy.is_none() && !self.retained_pending {ui.add_enabled_ui(idle,|ui|self.locked_ui(ui,ctx));}
                if let Some(path)=&self.managed_path {ui.colored_label(Color32::LIGHT_YELLOW,format!("MANAGED PLAINTEXT: {}",path.display()));}
                if let Some(copy)=&self.copy {
                    ui.colored_label(Color32::LIGHT_YELLOW,format!("UNLOCKED PLAINTEXT: {}",copy.work_root().display()));
                    ui.label(format!("Owner-only permissions verified: {}",copy.private_permissions()));
                    ui.add_enabled_ui(idle,|ui|self.unlocked_ui(ui,ctx,settings));
                }
                if self.close_requested && self.blocks_close(){
                    ui.separator();ui.strong("Choose how to close this unlocked workspace");
                    ui.label("Save the page draft, then Save encrypted snapshot & lock. If a task is running, cancel it and wait for its result. The previous encrypted commit stays intact until a verified replacement is activated.");
                    if self.copy.is_some() && idle {
                        let path=self.copy.as_ref().unwrap().work_root().to_owned();
                        if ui.button("Save encrypted snapshot & lock").clicked(){self.lock(ctx);}
                        let dirty=self.editor.as_ref().is_some_and(|editor|editor.dirty);
                        if dirty && ui.button("Preserve unsaved editor JSON separately in private work copy").clicked(){self.preserve_raw_draft(ctx);}
                        ui.checkbox(&mut self.retain_confirmed,format!("I explicitly choose to leave plaintext at {}. The previous encrypted commit remains separate; newer edits may be only in this copy.",path.display()));
                        if ui.add_enabled(self.retain_confirmed && !dirty && self.copy.is_some() && self.worker.is_none(),egui::Button::new("Retain plaintext, release key & close dialog")).clicked(){
                            self.report=Zeroizing::new(format!("Plaintext deliberately retained at {}. Reopen the last committed container normally; preserve newer edits here before manual cleanup.",path.display()));
                            self.copy=None;self.clear_editor(ctx);self.clear_secrets(ctx);self.original_project.zeroize();self.input.zeroize();self.managed_path=None;self.close_requested=false;self.retain_confirmed=false;self.open=false;
                        }
                    }
                    if self.retained_pending && idle && self.copy.is_none(){
                        ui.checkbox(&mut self.retain_confirmed,"I acknowledge the retained plaintext paths in the report; encrypted save did not remove this copy.");
                        if ui.add_enabled(self.retain_confirmed,egui::Button::new("Acknowledge retained plaintext & close dialog")).clicked(){self.retained_pending=false;self.managed_path=None;self.retain_confirmed=false;self.close_requested=false;self.open=false;}
                    }
                    if ui.button("Keep workspace open").clicked(){self.close_requested=false;self.retain_confirmed=false;}
                }
                if !self.report.is_empty(){egui::CollapsingHeader::new("Last result / retained paths").default_open(true).show(ui,|ui|{ui.label(self.report.as_str());});}
                egui::CollapsingHeader::new("Encryption and cleanup limits").show(ui,|ui|{ui.label(vault::LIMITS_NOTE);ui.label("The default passphrase KDF uses 256 MiB; plaintext work copies default to a 2 GiB snapshot limit, 512 MiB per file and 50,000 entries. Editing is limited to 200,000 characters per page. These are admission/commit bounds, not an OS disk quota. Native readers and OS temporary files may exist while unlocked.");});
            });
        if !open {
            if self.blocks_close() {
                self.request_close();
            } else {
                self.open = false;
                self.clear_editor(ctx);
                self.clear_secrets(ctx);
                self.original_project.zeroize();
                self.input.zeroize();
            }
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn locked_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        path_edit(ui, "Encrypted container", &mut self.container, |dialog| {
            dialog
                .add_filter("Encrypted workspace", &["ngvault"])
                .pick_file()
        });
        if ui
            .button("Choose new encrypted container filename")
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("workspace.ngvault")
                .save_file()
        {
            self.container = path.display().to_string();
        }
        path_edit(
            ui,
            "Separate recovery key",
            &mut self.recovery_key,
            |dialog| dialog.pick_file(),
        );
        ui.checkbox(&mut self.offline_key_permissions,"Explicit offline/export fallback: allow inherited permissions for a NEW recovery key if owner-only protection is unavailable");
        if ui.button("Generate separate recovery key file").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("workspace-recovery.agekey")
                .save_file()
        {
            self.recovery_key = path.display().to_string();
            let offline = self.offline_key_permissions;
            self.start(ctx, move |c| {
                let receipt = if offline {
                    vault::generate_recovery_key_offline_export(&path, c)?
                } else {
                    vault::generate_recovery_key(&path, c)?
                };
                Ok(Plain::Text(serde_json::to_string_pretty(&receipt)?))
            });
        }
        ui.small("Keep this unencrypted high-entropy key apart from the encrypted backups. NoteGen cannot recover a lost key and passphrase. Never send the key with a shared container.");
        path_edit(
            ui,
            "Original project to snapshot",
            &mut self.original_project,
            |dialog| dialog.pick_folder(),
        );
        path_edit(
            ui,
            "Private working-copy parent",
            &mut self.work_parent,
            |dialog| dialog.pick_folder(),
        );
        ui.checkbox(
            &mut self.use_recovery,
            "Unlock / verify backup with recovery key instead of passphrase",
        );
        password(ui, "Passphrase", "vault-passphrase", &mut self.passphrase);
        if self.use_recovery {
            password(
                ui,
                "Replacement passphrase for recovered workspace",
                "vault-replacement",
                &mut self.replacement,
            );
        }
        password(
            ui,
            "Confirm new/replacement passphrase",
            "vault-confirmation",
            &mut self.confirmation,
        );
        ui.small("Use at least 12 UTF-8 bytes; a longer unique passphrase is preferable. A recovery unlock requires a replacement passphrase, activated only when the new snapshot commits.");
        ui.checkbox(&mut self.inherited_work_permissions,"Explicit fallback: allow a plaintext work copy with inherited permissions when private permissions cannot be enforced");
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Create encrypted snapshot (original untouched)")
                .clicked()
            {
                self.create(ctx);
            }
            if ui.button("Unlock managed working copy").clicked() {
                self.unlock(ctx);
            }
            if ui.button("Verified encrypted backup").clicked() {
                self.backup(ctx, false);
            }
            if ui
                .button("Restore selected backup into a new container")
                .clicked()
            {
                self.backup(ctx, true);
            }
        });
        egui::CollapsingHeader::new("Inspect retained plaintext after interruption").show(ui,|ui|{
            path_edit(ui,"Retained plaintext folder (inspect only)",&mut self.retained_folder,|dialog|dialog.pick_folder());
            ui.small("This never adopts or deletes a previous working copy. Reopen the last committed container normally; preserve newer plaintext edits separately before manual cleanup.");
            if ui.button("Inspect marked retained copy").clicked(){let root=PathBuf::from(&self.retained_folder);self.start(ctx,move|c|Ok(Plain::Text(serde_json::to_string_pretty(&vault::inspect_retained(&root,c)?)?)));}
        });
    }
    fn unlocked_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, settings: &Settings) {
        let dirty = self.editor.as_ref().is_some_and(|editor| editor.dirty);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !dirty,
                    egui::Button::new("Reload current encrypted-workspace page"),
                )
                .clicked()
            {
                let index = self.editor.as_ref().map_or(0, |editor| editor.loaded.index);
                self.copy_task(ctx, move |copy, c| {
                    Ok(CopyUpdate::Editor(load_page(copy, index, c)?))
                });
            }
            if ui
                .add_enabled(
                    dirty,
                    egui::Button::new("Save page draft in private work copy"),
                )
                .clicked()
            {
                self.save_editor(ctx);
            }
            if ui
                .add_enabled(!dirty, egui::Button::new("Save encrypted snapshot & lock"))
                .clicked()
            {
                self.lock(ctx);
            }
        });
        if let Some(editor) = &mut self.editor {
            ui.label(format!(
                "Page {} · {} of {}",
                editor.loaded.page,
                editor.loaded.index + 1,
                editor.loaded.count
            ));
            let mut next = None;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !editor.dirty && editor.loaded.index > 0,
                        egui::Button::new("Previous private page"),
                    )
                    .clicked()
                {
                    next = Some(editor.loaded.index - 1);
                }
                if ui
                    .add_enabled(
                        !editor.dirty && editor.loaded.index + 1 < editor.loaded.count,
                        egui::Button::new("Next private page"),
                    )
                    .clicked()
                {
                    next = Some(editor.loaded.index + 1);
                }
            });
            ui.add(
                egui::Image::new(&editor.texture)
                    .alt_text("Private workspace source; confirmed masks remain opaque")
                    .max_height(230.)
                    .max_width(ui.available_width()),
            );
            ui.label("Page content JSON · validates before a guarded save · private correction history stays inside the encrypted snapshot");
            editor.dirty |= ui
                .add(
                    egui::TextEdit::multiline(&mut *editor.loaded.raw)
                        .id(egui::Id::unique(EDITOR_ID))
                        .code_editor()
                        .desired_rows(12)
                        .desired_width(f32::INFINITY)
                        .char_limit(EDITOR_CHARS),
                )
                .accessible_name("Encrypted workspace page content JSON")
                .changed();
            if editor.dirty {
                ui.colored_label(Color32::LIGHT_YELLOW,"Unsaved page draft · save before navigating, converting, exporting or locking.");
            }
            if let Some(index) = next {
                self.copy_task(ctx, move |copy, c| {
                    Ok(CopyUpdate::Editor(load_page(copy, index, c)?))
                });
            }
        }
        path_edit(
            ui,
            "Input for a fresh private conversion",
            &mut self.input,
            |dialog| dialog.pick_file(),
        );
        ui.horizontal_wrapped(|ui|{
            if ui.add_enabled(!dirty && !self.input.trim().is_empty(),egui::Button::new("Convert inside private workspace")).clicked(){
                let input=PathBuf::from(self.input.as_str());let settings=settings.clone();self.clear_editor(ctx);
                self.copy_task(ctx,move|copy,c|{let report=copy.run_session(&input,&settings,c,&Activity::default())?;
                    if report.success{Ok(CopyUpdate::Editor(load_page(copy,0,c)?))}else{Ok(CopyUpdate::Text(serde_json::to_string_pretty(&report)?))}
                });
            }
            if ui.add_enabled(!dirty,egui::Button::new("Export external Word copy")).clicked() && let Some(output)=rfd::FileDialog::new().set_file_name("workspace-export.docx").add_filter("Word",&["docx"]).save_file(){
                self.copy_task(ctx,move|copy,c|{let _worker=copy.begin_worker()?;let output=external_output(copy,&output)?;let project=Project::read(copy.project_path())?;word::export(&project,&output,c)?;
                    Ok(CopyUpdate::Text(format!("External plaintext Word copy exported to {}. It remains outside encrypted-workspace cleanup and protection.",output.display()))) });
            }
        });
    }
}
fn password(ui: &mut egui::Ui, label: &str, id: &str, value: &mut String) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .password(true)
            .id(egui::Id::unique(id))
            .char_limit(1024),
    )
    .accessible_name(label);
}
fn path_edit(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    pick: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>,
) {
    ui.label(label);
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(530.)
                .char_limit(4096),
        )
        .accessible_name(label);
        if ui.button("Choose…").clicked()
            && let Some(path) = pick(rfd::FileDialog::new())
        {
            *value = path.display().to_string();
        }
    });
}
impl Drop for VaultUi {
    fn drop(&mut self) {
        self.control.cancelled.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // Forced teardown still joins native workers. WorkCopy::drop retains its
        // marked plaintext copy; ordinary close is intercepted by blocks_close.
        self.passphrase.zeroize();
        self.replacement.zeroize();
        self.confirmation.zeroize();
        self.original_project.zeroize();
        self.input.zeroize();
        self.editor = None;
        self.report.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TextureDrain(egui::Context);
    impl Drop for TextureDrain {
        fn drop(&mut self) {
            self.0.tex_manager().write().take_delta().clear();
        }
    }
    fn wait(ui: &mut VaultUi, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while ui.worker.is_some() {
            ui.poll(ctx);
            assert!(
                Instant::now() < deadline,
                "workspace task deadline: {}",
                ui.error
            );
            thread::sleep(Duration::from_millis(10));
        }
        ctx.tex_manager().write().take_delta().clear();
    }
    #[test]
    fn credentials_and_editor_are_ephemeral_and_cleared_without_changing_workspace_settings() {
        let ctx = egui::Context::default();
        let _textures = TextureDrain(ctx.clone());
        let mut app = Desktop::new(Workspace::default());
        app.vault.passphrase = Zeroizing::new("PRIVATE VAULT PASSWORD".into());
        app.vault.replacement = Zeroizing::new("PRIVATE REPLACEMENT".into());
        app.vault.confirmation = Zeroizing::new("PRIVATE CONFIRMATION".into());
        app.vault.original_project = Zeroizing::new("PRIVATE SOURCE PATH".into());
        app.vault.report = Zeroizing::new("PRIVATE WORKSPACE RESULT".into());
        app.vault.install_editor(
            Loaded {
                raw: Zeroizing::new("PRIVATE EDITOR DRAFT".into()),
                expected: String::new(),
                page: 1,
                index: 0,
                count: 1,
                image: egui::ColorImage::from_rgba_unmultiplied([1, 1], &[0, 0, 0, 255]),
            },
            &ctx,
        );
        let stored = serde_json::to_string(&app.state).unwrap();
        assert!(!stored.contains("PRIVATE"));
        app.vault.clear_secrets(&ctx);
        app.vault.clear_editor(&ctx);
        assert!(
            app.vault.passphrase.is_empty()
                && app.vault.replacement.is_empty()
                && app.vault.confirmation.is_empty()
        );
        assert!(app.vault.editor.is_none());
        assert!(!app.vault.blocks_close());
    }

    #[test]
    fn failed_private_edit_and_worker_panic_keep_handle_and_raw_draft_can_be_encrypted_before_close()
     {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let original = Project::open(
            root.join("original"),
            Manifest {
                schema: 2,
                input_sha256: "a".repeat(64),
                fingerprint: "b".repeat(64),
                pages: vec![1],
                models: vec![],
                settings: Settings::default(),
            },
        )
        .unwrap();
        image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]))
            .save(original.image(1))
            .unwrap();
        original
            .save(&Page {
                page: 1,
                image: "page-00001.png".into(),
                content: PageContent {
                    blocks: vec![Block::text("Original private content")],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
        let original_checkpoint = fs::read(original.page_path(1)).unwrap();
        let key = root.join("recovery.agekey");
        vault::generate_recovery_key(&key, &Control::default()).unwrap();
        let pass = SecretString::from("private UI fixture passphrase".to_string());
        let path = root.join("snapshot.ngvault");
        let options = vault::Options {
            scrypt_log_n: 16,
            ..Default::default()
        };
        vault::create(
            &original.root,
            &path,
            &pass,
            &key,
            &options,
            &Control::default(),
        )
        .unwrap();
        let copy = vault::unlock(
            &path,
            &Credentials::Passphrase(&pass),
            &root,
            None,
            &options,
            &Control::default(),
        )
        .unwrap();
        let managed = copy.work_root().to_owned();
        let ctx = egui::Context::default();
        let _textures = TextureDrain(ctx.clone());
        let mut ui = VaultUi::default();
        ui.open = true;
        ui.managed_path = Some(managed.clone());
        ui.copy = Some(copy);
        ui.install_editor(
            load_page(ui.copy.as_ref().unwrap(), 0, &Control::default()).unwrap(),
            &ctx,
        );
        ui.request_close();
        assert!(ui.blocks_close());
        ui.copy_task(&ctx, |_, _| panic!("controlled workspace operation panic"));
        wait(&mut ui, &ctx);
        assert!(ui.copy.is_some());
        assert!(ui.error.contains("Plaintext is retained"));
        let invalid = "{ private unfinished JSON draft";
        ui.editor.as_mut().unwrap().loaded.raw = Zeroizing::new(invalid.into());
        ui.editor.as_mut().unwrap().dirty = true;
        ui.lock(&ctx);
        assert!(ui.worker.is_none());
        assert!(ui.copy.is_some());
        assert!(ui.blocks_close());
        ui.save_editor(&ctx);
        wait(&mut ui, &ctx);
        assert!(ui.copy.is_some());
        assert!(ui.editor.as_ref().unwrap().dirty);
        assert_eq!(ui.editor.as_ref().unwrap().loaded.raw.as_str(), invalid);
        ui.preserve_raw_draft(&ctx);
        wait(&mut ui, &ctx);
        assert!(ui.editor.is_none());
        let draft = fs::read_dir(managed.join("work"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("unsaved-editor-")
            })
            .unwrap();
        assert_eq!(fs::read_to_string(&draft).unwrap(), invalid);
        ui.lock(&ctx);
        wait(&mut ui, &ctx);
        assert!(ui.copy.is_none(), "Encrypted lock failed: {}", ui.error);
        assert!(!ui.blocks_close(), "{}", ui.error);
        assert!(!managed.exists());
        assert_eq!(
            fs::read(original.page_path(1)).unwrap(),
            original_checkpoint
        );
        let checked =
            vault::verify(&path, &Credentials::RecoveryKey(&key), &Control::default()).unwrap();
        assert!(
            checked
                .files
                .iter()
                .any(|file| file.path.contains("unsaved-editor-"))
        );
        // A saved cipher with unknown retained plaintext still blocks ordinary app
        // close until the user explicitly acknowledges its exact report paths.
        let copy = vault::unlock(
            &path,
            &Credentials::RecoveryKey(&key),
            &root,
            Some(&pass),
            &options,
            &Control::default(),
        )
        .unwrap();
        let retained = copy.work_root().to_owned();
        fs::write(retained.join("manual note.txt"), b"retain this plaintext").unwrap();
        ui.managed_path = Some(retained.clone());
        ui.copy = Some(copy);
        ui.lock(&ctx);
        wait(&mut ui, &ctx);
        assert!(ui.copy.is_none());
        assert!(ui.retained_pending && ui.blocks_close());
        assert!(ui.close_requested);
        assert!(ui.report.contains("manual note.txt"));
        assert_eq!(
            fs::read(retained.join("manual note.txt")).unwrap(),
            b"retain this plaintext"
        );
        assert_eq!(
            fs::read(original.page_path(1)).unwrap(),
            original_checkpoint
        );
    }
}
