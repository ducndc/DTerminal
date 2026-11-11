mod app;
mod backend;
mod term;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DTerminal",
        options,
        Box::new(|_cc| Ok(Box::new(app::DTerminalApp::default()))),
    )
}
