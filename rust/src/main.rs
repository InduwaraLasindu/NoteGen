#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use clap::Parser;
use eframe::egui;
#[derive(Parser)]
#[command(name = "NoteGen", version, about = "Local document studio")]
struct Args {
    /// Keep UI persistence and initial model/writer storage in a selected folder.
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,
    /// Select a native renderer for graphics compatibility testing.
    #[arg(long,value_parser=["wgpu","glow"])]
    renderer: Option<String>,
}
fn main() -> eframe::Result {
    let args = Args::parse();
    let data_dir = args
        .data_dir
        .map(|path| {
            notegen::watch::no_links(&path)?;
            std::fs::create_dir_all(&path)?;
            notegen::watch::no_links(&path)?;
            path.canonicalize().map_err(anyhow::Error::from)
        })
        .transpose()
        .map_err(|error| eframe::Error::AppCreation(error.into()))?;
    if let Some(parent) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        && parent.join("runtime").is_dir()
    {
        let _ = std::env::set_current_dir(parent);
    }
    let options = eframe::NativeOptions {
        persistence_path: data_dir.as_ref().map(|path| path.join("ui")),
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000., 860.])
            .with_min_inner_size([740., 650.])
            .with_title("NoteGen · Local document studio"),
        renderer: if args.renderer.as_deref() == Some("glow") {
            eframe::Renderer::Glow
        } else if args.renderer.as_deref() == Some("wgpu") || cfg!(windows) {
            eframe::Renderer::Wgpu
        } else {
            eframe::Renderer::Glow
        },
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "NoteGen",
        options,
        Box::new(move |cc| {
            Ok(Box::new(
                notegen::desktop::Desktop::from_context_with_data_dir(cc, data_dir.as_deref()),
            ))
        }),
    )
}
