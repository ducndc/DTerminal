use crate::backend::local::LocalTerminal;
use crate::backend::ssh::{SshConfig, SshTerminal};
use crate::config::{Appearance, Session, Store};
use crate::tabs::{LocalTab, SshState, SshTab, Tab};
use crate::term::theme::Palette;
use crate::term::{render, TerminalSurface};
use alacritty_terminal::vte::ansi::CursorShape;

#[derive(PartialEq, Clone, Copy)]
enum AuthType {
    Password,
    Key,
}

#[derive(PartialEq, Clone, Copy)]
enum TransferProtocol {
    Sftp,
    Scp,
    Tftp,
}

impl TransferProtocol {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sftp => "sftp",
            Self::Scp => "scp",
            Self::Tftp => "tftp",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "scp" => Self::Scp,
            "tftp" => Self::Tftp,
            _ => Self::Sftp,
        }
    }
}

struct SessionDraft {
    editing_id: Option<String>,
    name: String,
    host: String,
    port: u16,
    username: String,
    transfer_protocol: TransferProtocol,
    tftp_port: u16,
    auth_type: AuthType,
    /// Typed-in plaintext; blank while editing means "keep the existing secret".
    password: String,
    key_path: String,
    passphrase: String,
}

impl SessionDraft {
    fn new() -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            transfer_protocol: TransferProtocol::Sftp,
            tftp_port: 69,
            auth_type: AuthType::Password,
            password: String::new(),
            key_path: String::new(),
            passphrase: String::new(),
        }
    }

    fn from_session(s: &Session) -> Self {
        Self {
            editing_id: Some(s.id.clone()),
            name: s.name.clone(),
            host: s.host.clone(),
            port: s.port,
            username: s.username.clone(),
            transfer_protocol: TransferProtocol::from_str(&s.transfer_protocol),
            tftp_port: s.tftp_port,
            auth_type: if s.auth_type == "key" {
                AuthType::Key
            } else {
                AuthType::Password
            },
            password: String::new(),
            key_path: s.key_path.clone(),
            passphrase: String::new(),
        }
    }

    fn into_session(&self) -> Session {
        Session {
            id: self.editing_id.clone().unwrap_or_default(),
            name: if self.name.trim().is_empty() {
                self.host.clone()
            } else {
                self.name.clone()
            },
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            auth_type: match self.auth_type {
                AuthType::Password => "password".to_string(),
                AuthType::Key => "key".to_string(),
            },
            key_path: self.key_path.clone(),
            transfer_protocol: self.transfer_protocol.as_str().to_string(),
            tftp_port: self.tftp_port,
            password: String::new(),
            passphrase: String::new(),
        }
    }
}

struct ConfirmDeleteSession {
    id: String,
    name: String,
}

pub struct DTerminalApp {
    store: Store,
    sessions: Vec<Session>,
    appearance: Appearance,
    palette: Palette,

    tabs: Vec<Tab>,
    active_tab: Option<usize>,
    tab_seq: usize,

    session_modal: Option<SessionDraft>,
    settings_modal: Option<Appearance>,
    confirm_delete: Option<ConfirmDeleteSession>,
}

impl Default for DTerminalApp {
    fn default() -> Self {
        let store = Store::load();
        let sessions = store.list_sessions();
        let appearance = store.appearance();
        let palette = Palette::by_name(&appearance.theme);
        Self {
            store,
            sessions,
            appearance,
            palette,
            tabs: Vec::new(),
            active_tab: None,
            tab_seq: 0,
            session_modal: None,
            settings_modal: None,
            confirm_delete: None,
        }
    }
}

fn cursor_shape_from_str(s: &str) -> CursorShape {
    match s {
        "bar" => CursorShape::Beam,
        "underline" => CursorShape::Underline,
        _ => CursorShape::Block,
    }
}

impl DTerminalApp {
    fn open_local_tab(&mut self) {
        self.tab_seq += 1;
        match LocalTerminal::spawn(80, 24) {
            Ok(backend) => {
                let tab = LocalTab {
                    title: format!("Local Terminal {}", self.tab_seq),
                    surface: TerminalSurface::new(
                        80,
                        24,
                        cursor_shape_from_str(&self.appearance.cursor_style),
                    ),
                    backend,
                    connected: true,
                    just_created: true,
                };
                self.tabs.push(Tab::Local(tab));
                self.active_tab = Some(self.tabs.len() - 1);
            }
            Err(err) => {
                eprintln!("failed to spawn local terminal: {err}");
            }
        }
    }

    fn open_ssh_tab(&mut self, session_id: &str) {
        let Some(session) = self.store.get_session_with_secrets(session_id) else {
            return;
        };
        let config = SshConfig {
            host: session.host.clone(),
            port: session.port,
            username: session.username.clone(),
            password: session.password,
            key_path: session.key_path,
            passphrase: session.passphrase,
            use_key: session.auth_type == "key",
            cols: 80,
            rows: 24,
        };
        let backend = SshTerminal::connect(config);
        let tab = SshTab {
            title: session.name.clone(),
            session_id: session.id.clone(),
            surface: TerminalSurface::new(80, 24, cursor_shape_from_str(&self.appearance.cursor_style)),
            backend,
            state: SshState::Connecting,
            just_created: true,
        };
        self.tabs.push(Tab::Ssh(tab));
        self.active_tab = Some(self.tabs.len() - 1);
    }

    fn reconnect_ssh_tab(&mut self, index: usize) {
        let Tab::Ssh(tab) = &self.tabs[index] else {
            return;
        };
        let Some(session) = self.store.get_session_with_secrets(&tab.session_id) else {
            return;
        };
        let config = SshConfig {
            host: session.host.clone(),
            port: session.port,
            username: session.username.clone(),
            password: session.password,
            key_path: session.key_path,
            passphrase: session.passphrase,
            use_key: session.auth_type == "key",
            cols: 80,
            rows: 24,
        };
        let backend = SshTerminal::connect(config);
        if let Tab::Ssh(tab) = &mut self.tabs[index] {
            tab.surface = TerminalSurface::new(80, 24, cursor_shape_from_str(&self.appearance.cursor_style));
            tab.backend = backend;
            tab.state = SshState::Connecting;
            tab.just_created = true;
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs[index].close();
        self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            Some(i) if i == index => self.tabs.len().checked_sub(1),
            Some(i) if i > index => Some(i - 1),
            other => other,
        };
    }

    fn save_session_draft(&mut self) {
        let Some(draft) = &self.session_modal else {
            return;
        };
        if draft.host.trim().is_empty() || draft.username.trim().is_empty() {
            return;
        }
        let session = draft.into_session();
        let new_password = (!draft.password.is_empty()).then(|| draft.password.clone());
        let new_passphrase = (!draft.passphrase.is_empty()).then(|| draft.passphrase.clone());
        self.store.save_session(session, new_password, new_passphrase);
        self.sessions = self.store.list_sessions();
        self.session_modal = None;
    }

    fn session_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.session_modal else {
            return;
        };
        let is_new = draft.editing_id.is_none();
        let mut close = false;
        let mut save = false;

        egui::Modal::new(egui::Id::unique("session_modal")).show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.heading(if is_new { "New Session" } else { "Edit Session" });
            ui.separator();

            egui::Grid::new("session_form_grid")
                .num_columns(2)
                .spacing([8.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.end_row();

                    ui.label("Host");
                    ui.text_edit_singleline(&mut draft.host);
                    ui.end_row();

                    ui.label("Port");
                    ui.add(egui::DragValue::new(&mut draft.port).range(1..=65535));
                    ui.end_row();

                    ui.label("Username");
                    ui.text_edit_singleline(&mut draft.username);
                    ui.end_row();

                    ui.label("Transfer protocol");
                    egui::ComboBox::from_id_salt("transfer_protocol")
                        .selected_text(match draft.transfer_protocol {
                            TransferProtocol::Sftp => "SFTP (default)",
                            TransferProtocol::Scp => "SCP (no SFTP subsystem)",
                            TransferProtocol::Tftp => "TFTP (no auth, UDP)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.transfer_protocol,
                                TransferProtocol::Sftp,
                                "SFTP (default)",
                            );
                            ui.selectable_value(
                                &mut draft.transfer_protocol,
                                TransferProtocol::Scp,
                                "SCP (no SFTP subsystem)",
                            );
                            ui.selectable_value(
                                &mut draft.transfer_protocol,
                                TransferProtocol::Tftp,
                                "TFTP (no auth, UDP)",
                            );
                        });
                    ui.end_row();

                    if draft.transfer_protocol == TransferProtocol::Tftp {
                        ui.label("TFTP port");
                        ui.add(egui::DragValue::new(&mut draft.tftp_port).range(1..=65535));
                        ui.end_row();
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut draft.auth_type, AuthType::Password, "Password");
                ui.selectable_value(&mut draft.auth_type, AuthType::Key, "Private Key");
            });

            match draft.auth_type {
                AuthType::Password => {
                    ui.horizontal(|ui| {
                        ui.label("Password");
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.password)
                                .password(true)
                                .hint_text("(leave blank to keep existing)"),
                        );
                    });
                }
                AuthType::Key => {
                    ui.horizontal(|ui| {
                        ui.label("Key file");
                        ui.text_edit_singleline(&mut draft.key_path);
                        if ui.button("Browse…").clicked() {
                            if let Some(path) = rfd_pick_file() {
                                draft.key_path = path;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Passphrase");
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.passphrase)
                                .password(true)
                                .hint_text("(optional)"),
                        );
                    });
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                let can_save = !draft.host.trim().is_empty() && !draft.username.trim().is_empty();
                if ui.add_enabled(can_save, egui::Button::new("Save")).clicked() {
                    save = true;
                }
            });
        });

        if save {
            self.save_session_draft();
        } else if close {
            self.session_modal = None;
        }
    }

    fn settings_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.settings_modal else {
            return;
        };
        let mut close = false;
        let mut save = false;

        egui::Modal::new(egui::Id::unique("settings_modal")).show(ctx, |ui| {
            ui.set_min_width(320.0);
            ui.heading("Appearance");
            ui.separator();

            egui::Grid::new("settings_form_grid")
                .num_columns(2)
                .spacing([8.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Theme");
                    egui::ComboBox::from_id_salt("theme")
                        .selected_text(theme_label(&draft.theme))
                        .show_ui(ui, |ui| {
                            for name in ["dark", "light", "solarizedDark", "monokai", "dracula"] {
                                ui.selectable_value(&mut draft.theme, name.to_string(), theme_label(name));
                            }
                        });
                    ui.end_row();

                    ui.label("Font family");
                    ui.text_edit_singleline(&mut draft.font_family);
                    ui.end_row();

                    ui.label("Font size");
                    ui.add(egui::DragValue::new(&mut draft.font_size).range(8.0..=32.0));
                    ui.end_row();

                    ui.label("Cursor style");
                    egui::ComboBox::from_id_salt("cursor_style")
                        .selected_text(draft.cursor_style.clone())
                        .show_ui(ui, |ui| {
                            for name in ["block", "bar", "underline"] {
                                ui.selectable_value(&mut draft.cursor_style, name.to_string(), name);
                            }
                        });
                    ui.end_row();
                });

            ui.label("Font family is saved but not yet applied to the terminal — egui renders with its built-in monospace font for now.");

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if ui.button("Save").clicked() {
                    save = true;
                }
            });
        });

        if save {
            let appearance = draft.clone();
            self.palette = Palette::by_name(&appearance.theme);
            self.store.save_appearance(appearance.clone());
            self.appearance = appearance;
            self.settings_modal = None;
        } else if close {
            self.settings_modal = None;
        }
    }

    fn confirm_delete_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.confirm_delete else {
            return;
        };
        let mut delete = false;
        let mut cancel = false;

        egui::Modal::new(egui::Id::unique("confirm_delete_session")).show(ctx, |ui| {
            ui.set_min_width(280.0);
            ui.label(format!("Delete session \"{}\"?", pending.name));
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if ui.button("Delete").clicked() {
                    delete = true;
                }
            });
        });

        if delete {
            if let Some(pending) = self.confirm_delete.take() {
                self.store.delete_session(&pending.id);
                self.sessions = self.store.list_sessions();
            }
        } else if cancel {
            self.confirm_delete = None;
        }
    }
}

fn theme_label(name: &str) -> &'static str {
    match name {
        "light" => "Light",
        "solarizedDark" => "Solarized Dark",
        "monokai" => "Monokai",
        "dracula" => "Dracula",
        _ => "Dark",
    }
}

/// Placeholder for a native file picker — wired up properly once the
/// SSH session modal needs it for real (milestone 4).
fn rfd_pick_file() -> Option<String> {
    None
}

impl eframe::App for DTerminalApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("DTerminal");
                    if ui.button("+").on_hover_text("New session").clicked() {
                        self.session_modal = Some(SessionDraft::new());
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

                if self.sessions.is_empty() {
                    ui.label("No sessions saved yet.");
                }
                let mut connect_request: Option<usize> = None;
                let mut edit_request: Option<usize> = None;
                let mut delete_request: Option<usize> = None;
                for (i, session) in self.sessions.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Label::new(&session.name).sense(egui::Sense::click()))
                            .on_hover_text(format!("Connect to {}@{}", session.username, session.host))
                            .clicked()
                        {
                            connect_request = Some(i);
                        }
                        if ui.small_button("✎").clicked() {
                            edit_request = Some(i);
                        }
                        if ui.small_button("🗑").clicked() {
                            delete_request = Some(i);
                        }
                    });
                }
                if let Some(i) = connect_request {
                    let id = self.sessions[i].id.clone();
                    self.open_ssh_tab(&id);
                }
                if let Some(i) = edit_request {
                    self.session_modal = Some(SessionDraft::from_session(&self.sessions[i]));
                }
                if let Some(i) = delete_request {
                    self.confirm_delete = Some(ConfirmDeleteSession {
                        id: self.sessions[i].id.clone(),
                        name: self.sessions[i].name.clone(),
                    });
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if ui.button("⚙ Settings").clicked() {
                        self.settings_modal = Some(self.appearance.clone());
                    }
                });
            });

        for tab in &mut self.tabs {
            tab.pump_events();
        }

        let mut close_requested: Option<usize> = None;
        egui::Panel::top("tab_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, tab) in self.tabs.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let selected = self.active_tab == Some(i);
                        if ui.selectable_label(selected, tab.title()).clicked() {
                            self.active_tab = Some(i);
                        }
                        if !tab.is_live() {
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

        let mut reconnect_requested: Option<usize> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(i) = self.active_tab else {
                ui.centered_and_justified(|ui| {
                    ui.label("No tabs open — click ⌨ for a local terminal, or + for an SSH session.");
                });
                return;
            };
            let tab = &mut self.tabs[i];
            let focus_now = tab.take_just_created();

            let id = egui::Id::unique(("terminal", i));
            let result = render::show(
                ui,
                id,
                tab.surface(),
                &self.palette,
                self.appearance.font_size,
                focus_now,
            );
            if !result.input_bytes.is_empty() {
                tab.write(&result.input_bytes);
            }
            if let Some((cols, rows)) = result.resize_to {
                tab.resize(cols, rows);
            }

            if let Some(status) = tab.status_line() {
                egui::Area::new(egui::Id::unique(("status_overlay", i)))
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .order(egui::Order::Foreground)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.label(&status);
                                if tab.can_reconnect() && ui.button("Reconnect").clicked() {
                                    reconnect_requested = Some(i);
                                }
                            });
                        });
                    });
            }
        });

        if let Some(i) = reconnect_requested {
            self.reconnect_ssh_tab(i);
        }

        let ctx = ui.ctx().clone();
        self.session_modal_ui(&ctx);
        self.settings_modal_ui(&ctx);
        self.confirm_delete_ui(&ctx);

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
