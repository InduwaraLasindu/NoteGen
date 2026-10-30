//! Native startup and visual smoke test. Capture the app's rendered window, then exit.
use eframe::egui;
use notegen::{desktop::{Desktop,Workspace},hardware,types::Preset};
use std::{path::PathBuf,sync::{Arc,atomic::{AtomicBool,Ordering}}};
struct Capture {app:Desktop,frames:usize,path:PathBuf,done:Arc<AtomicBool>}
impl eframe::App for Capture {
    fn ui(&mut self,ui:&mut egui::Ui,_:&mut eframe::Frame) {
        self.app.render(ui);self.frames+=1;
        if self.frames==12 {
            let path=self.path.clone();let done=self.done.clone();
            ui.ctx().request_screenshot(move|im|{
                let bytes=im.pixels.iter().flat_map(|p|p.to_array()).collect::<Vec<_>>();
                image::save_buffer(&path,&bytes,im.width() as u32,im.height() as u32,image::ColorType::Rgba8).expect("save screenshot");
                done.store(true,Ordering::Relaxed);
            });
        }
        if self.done.load(Ordering::Relaxed){ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);}
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }
}
fn main()->eframe::Result {
    let args=std::env::args().collect::<Vec<_>>();
    let output=PathBuf::from(args.get(1).expect("ui_capture OUTPUT.png [workspace|advanced|review] [PROJECT]"));
    if let Some(p)=output.parent(){std::fs::create_dir_all(p).expect("screenshot directory");}
    let mode=args.get(2).map_or("workspace",String::as_str).to_owned();let project=args.get(3).map(PathBuf::from);
    eframe::run_native("NoteGen visual acceptance",eframe::NativeOptions{
        renderer:if cfg!(windows){eframe::Renderer::Wgpu}else{eframe::Renderer::Glow},
        viewport:egui::ViewportBuilder::default().with_inner_size([1140.,890.]),..Default::default()
    },Box::new(move|cc|{
        let mut state=Workspace::default();state.settings.preset=Preset::Fast;state.settings.ui.reduced_motion=true;
        let mut app=Desktop::new(state);app.hardware=Some(hardware::detect());
        if mode=="advanced"{app.advanced_open();}
        if mode=="review"{app.open_review(project.expect("review requires PROJECT directory"),&cc.egui_ctx);}
        Ok(Box::new(Capture{app,frames:0,path:output,done:Arc::new(AtomicBool::new(false))}))
    }))
}
