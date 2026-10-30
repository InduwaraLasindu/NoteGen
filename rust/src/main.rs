#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use eframe::egui;
fn main() -> eframe::Result {
    if let Some(parent) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        && parent.join("runtime").is_dir()
    {
        let _ = std::env::set_current_dir(parent);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000., 860.])
            .with_min_inner_size([740., 650.])
            .with_title("NoteGen · Local document studio"),
        renderer: if cfg!(windows) { eframe::Renderer::Wgpu } else { eframe::Renderer::Glow },
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "NoteGen",
        options,
        Box::new(|cc| Ok(Box::new(notegen::desktop::Desktop::from_context(cc)))),
    )
}
