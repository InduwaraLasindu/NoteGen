//! Native startup and visual smoke test. Capture the app's rendered window, then exit.
use eframe::egui;
use notegen::{
    desktop::{Desktop, Workspace},
    hardware,
    types::Preset,
};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
struct Capture {
    app: Desktop,
    frames: usize,
    path: PathBuf,
    done: Arc<AtomicBool>,
    motion_preview: bool,
    began: std::time::Instant,
}
impl eframe::App for Capture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.app.render(ui);
        self.frames += 1;
        if self.frames == 12 {
            let path = self.path.clone();
            let done = self.done.clone();
            ui.ctx().request_screenshot(move |im| {
                let bytes = im
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                let mut encoded = std::io::Cursor::new(Vec::new());
                image::write_buffer_with_format(
                    &mut encoded,
                    &bytes,
                    im.width() as u32,
                    im.height() as u32,
                    image::ColorType::Rgba8,
                    image::ImageFormat::Png,
                )
                .expect("encode screenshot");
                let mut file = std::fs::File::create(&path).expect("create screenshot");
                file.write_all(encoded.get_ref()).expect("save screenshot");
                file.sync_all().expect("flush screenshot");
                done.store(true, Ordering::Relaxed);
            });
        }
        if self.done.load(Ordering::Relaxed)
            && (!self.motion_preview || self.began.elapsed().as_secs_f32() > 6.)
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(30));
    }
}
fn main() -> eframe::Result {
    let args = std::env::args().collect::<Vec<_>>();
    let output = PathBuf::from(
        args.get(1)
            .expect("ui_capture OUTPUT.png [workspace|advanced|review] [PROJECT]"),
    );
    if let Some(p) = output.parent() {
        std::fs::create_dir_all(p).expect("screenshot directory");
    }
    let mode = args.get(2).map_or("workspace", String::as_str).to_owned();
    let project = args.get(3).map(PathBuf::from);
    let verify_path = output.clone();
    eframe::run_native(
        "NoteGen visual acceptance",
        eframe::NativeOptions {
            renderer: if cfg!(windows) {
                eframe::Renderer::Wgpu
            } else {
                eframe::Renderer::Glow
            },
            viewport: egui::ViewportBuilder::default().with_inner_size([1140., 890.]),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut state = Workspace::default();
            state.settings.preset = Preset::Fast;
            state.settings.ui.reduced_motion = mode != "motion";
            let mut app = Desktop::new(state);
            app.hardware = Some(hardware::detect());
            if mode == "advanced" {
                app.advanced_open();
            }
            if mode == "review" {
                app.open_review(
                    project.expect("review requires PROJECT directory"),
                    &cc.egui_ctx,
                );
            }
            Ok(Box::new(Capture {
                app,
                frames: 0,
                path: output,
                done: Arc::new(AtomicBool::new(false)),
                motion_preview: mode == "motion",
                began: std::time::Instant::now(),
            }))
        }),
    )?;
    let image = image::open(verify_path).expect("native screenshot must be a complete image");
    assert!(image.width() >= 1000 && image.height() >= 700);
    Ok(())
}
