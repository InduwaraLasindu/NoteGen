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
            state.settings.experience.check_updates = false;
            state.settings.ui.reduced_motion = mode != "motion";
            if mode == "writers" {
                state.settings.writer.root = "build/ui-demo-writers".into();
                let profile = notegen::writers::create(&state.settings.writer.root, "Avery (demo)")
                    .expect("create demo writer");
                let bytes = [
                    "C:/Windows/Fonts/arial.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                ]
                .into_iter()
                .find_map(|p| std::fs::read(p).ok())
                .expect("QA system font");
                let font = ab_glyph::FontArc::try_from_vec(bytes).unwrap();
                for ch in "Aaersti025".chars() {
                    let mut im = image::RgbImage::from_pixel(100, 120, image::Rgb([255, 255, 255]));
                    imageproc::drawing::draw_text_mut(
                        &mut im,
                        image::Rgb([0, 0, 0]),
                        10,
                        10,
                        90.,
                        &font,
                        &ch.to_string(),
                    );
                    notegen::writers::add_example(
                        &state.settings.writer.root,
                        &profile.id,
                        &image::DynamicImage::ImageRgb8(im),
                        notegen::types::BoxRect::full(),
                        &ch.to_string(),
                        true,
                        "Synthetic UI demonstration",
                    )
                    .expect("demo glyph");
                }
                state.settings.writer.id = Some(profile.id);
            }
            let mut app = Desktop::new(state);
            if mode == "settings" {
                app.settings_open(0);
            }
            if mode == "updates" {
                app.settings_open(1);
            }
            if mode == "hardware" {
                app.settings_open(2);
            }
            app.hardware = Some(hardware::detect());
            if mode == "studio" {
                app.studio_open(0);
            }
            if mode == "writers" {
                app.studio_open(1);
            }
            if mode == "models" {
                app.studio_open(2);
            }
            if mode == "advanced" {
                app.advanced_open();
            }
            if mode == "review" {
                let project = project.clone().expect("review requires PROJECT directory");
                assert!(
                    app.open_review(project.clone(), &cc.egui_ctx),
                    "Review capture could not open {}: {}",
                    project.display(),
                    app.notice
                );
            }
            if let Some(tab) = [
                "workbench-projects",
                "workbench-export",
                "workbench-recovery",
                "workbench-privacy",
            ]
            .iter()
            .position(|m| *m == mode)
            {
                if let Some(project) = project.as_ref() {
                    assert!(
                        app.open_review(project.clone(), &cc.egui_ctx),
                        "Workbench capture could not open {}: {}",
                        project.display(),
                        app.notice
                    );
                }
                app.tools_open(tab);
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
