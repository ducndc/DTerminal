use crate::backend::local::{LocalEvent, LocalTerminal};
use crate::term::theme::Palette;
use crate::term::{render, TerminalSurface};

struct LocalTab {
    title: String,
    surface: TerminalSurface,
    backend: LocalTerminal,
    connected: bool,
    just_created: bool,
}

pub struct DTerminalApp {
    tabs: Vec<LocalTab>,
    active_tab: Option<usize>,
    palette: Palette,
    tab_seq: usize,
}

impl Default for DTerminalApp {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: None,
            palette: Palette::dark(),
            tab_seq: 0,
        }
    }
}

impl DTerminalApp {
    fn open_local_tab(&mut self) {
        self.tab_seq += 1;
        match LocalTerminal::spawn(80, 24) {
            Ok(backend) => {
                let tab = LocalTab {
                    title: format!("Local Terminal {}", self.tab_seq),
                    surface: TerminalSurface::new(80, 24),
                    backend,
                    connected: true,
                    just_created: true,
                };
                self.tabs.push(tab);
                self.active_tab = Some(self.tabs.len() - 1);
            }
            Err(err) => {
                eprintln!("failed to spawn local terminal: {err}");
            }
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs[index].backend.kill();
        self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            Some(i) if i == index => self.tabs.len().checked_sub(1),
            Some(i) if i > index => Some(i - 1),
            other => other,
        };
    }
}

impl eframe::App for DTerminalApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Drain backend events into each tab's terminal surface before painting.
        for tab in &mut self.tabs {
            loop {
                match tab.backend.events.try_recv() {
                    Ok(LocalEvent::Data(bytes)) => tab.surface.feed(&bytes),
                    Ok(LocalEvent::Exited) => {
                        tab.connected = false;
                        break;
                    }
                    Err(_) => break,
                }
            }
        }

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("DTerminal");
                    if ui.button("+").on_hover_text("New session").clicked() {
                        // milestone 3: open session editor
                    }
                    if ui
                        .button("⌨")
                        .on_hover_text("New local terminal")
                        .clicked()
                    {
                        self.open_local_tab();
                    }
                });
                ui.separator();
                ui.label("Sessions (none saved yet)");
            });

        let mut close_requested: Option<usize> = None;
        egui::Panel::top("tab_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, tab) in self.tabs.iter().enumerate() {
                    let selected = self.active_tab == Some(i);
                    ui.horizontal(|ui| {
                        if ui.selectable_label(selected, &tab.title).clicked() {
                            self.active_tab = Some(i);
                        }
                        if !tab.connected {
                            ui.colored_label(egui::Color32::from_rgb(0xf4, 0x47, 0x47), "●");
                        }
                        if ui.small_button("✕").clicked() {
                            close_requested = Some(i);
                        }
                    });
                }
            });
        });

        if let Some(i) = close_requested {
            self.close_tab(i);
        }

        egui::CentralPanel::default().show(ui, |ui| {
            let Some(i) = self.active_tab else {
                ui.centered_and_justified(|ui| {
                    ui.label("No tabs open — click ⌨ for a local terminal, or + for an SSH session.");
                });
                return;
            };
            let tab = &mut self.tabs[i];
            let focus_now = tab.just_created;
            tab.just_created = false;

            let id = egui::Id::unique(("local_term", i));
            let result = render::show(ui, id, &tab.surface, &self.palette, focus_now);
            if !result.input_bytes.is_empty() && tab.connected {
                tab.backend.write(&result.input_bytes);
            }
            if let Some((cols, rows)) = result.resize_to {
                tab.surface.resize(cols, rows);
                if tab.connected {
                    tab.backend.resize(cols, rows);
                }
            }
        });

        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}
