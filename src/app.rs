use crate::backend::local::LocalTerminal;
use crate::backend::sftp::{Protocol, SftpBackend};
use crate::backend::ssh::{SshConfig, SshTerminal};
use crate::backend::tftp;
use crate::config::{Appearance, QuickCommand, Session, Store};
use crate::tabs::{LocalTab, SftpPanel, SshState, SshTab, Tab, TftpPanel, TftpResult};
use crate::term::theme::Palette;
use crate::term::{render, TerminalSurface};
use alacritty_terminal::vte::ansi::CursorShape;
use std::thread;

fn ssh_config_for(session: &Session) -> SshConfig {
    SshConfig {
        host: session.host.clone(),
        port: session.port,
        username: session.username.clone(),
        password: session.password.clone(),
        key_path: session.key_path.clone(),
        passphrase: session.passphrase.clone(),
        use_key: session.auth_type == "key",
        cols: 80,
        rows: 24,
    }
}

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

struct ConfirmDeleteSftpEntry {
    tab_index: usize,
    path: String,
    name: String,
    is_dir: bool,
}

struct MkdirDraft {
    tab_index: usize,
    name: String,
}

struct QuickCommandDraft {
    editing_id: Option<String>,
    name: String,
    command: String,
    run_enter: bool,
}

impl QuickCommandDraft {
    fn new() -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            command: String::new(),
            run_enter: true,
        }
    }

    fn from_command(c: &QuickCommand) -> Self {
        Self {
            editing_id: Some(c.id.clone()),
            name: c.name.clone(),
            command: c.command.clone(),
            run_enter: c.run_enter,
        }
    }
}

struct ConfirmDeleteCommand {
    id: String,
    name: String,
}

pub struct DTerminalApp {
    store: Store,
    sessions: Vec<Session>,
    appearance: Appearance,
    palette: Palette,
    quick_commands: Vec<QuickCommand>,
    cmd_panel_open: bool,

    tabs: Vec<Tab>,
    active_tab: Option<usize>,
    tab_seq: usize,

    session_modal: Option<SessionDraft>,
    settings_modal: Option<Appearance>,
    confirm_delete: Option<ConfirmDeleteSession>,
    confirm_delete_sftp: Option<ConfirmDeleteSftpEntry>,
    mkdir_modal: Option<MkdirDraft>,
    cmd_modal: Option<QuickCommandDraft>,
    confirm_delete_cmd: Option<ConfirmDeleteCommand>,
}

impl Default for DTerminalApp {
    fn default() -> Self {
        let store = Store::load();
        let sessions = store.list_sessions();
        let appearance = store.appearance();
        let palette = Palette::by_name(&appearance.theme);
        let quick_commands = store.quick_commands();
        Self {
            store,
            sessions,
            appearance,
            palette,
            quick_commands,
            cmd_panel_open: false,
            tabs: Vec::new(),
            active_tab: None,
            tab_seq: 0,
            session_modal: None,
            settings_modal: None,
            confirm_delete: None,
            confirm_delete_sftp: None,
            mkdir_modal: None,
            cmd_modal: None,
            confirm_delete_cmd: None,
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
        let config = ssh_config_for(&session);
        let backend = SshTerminal::connect(config);
        let tab = SshTab {
            title: session.name.clone(),
            session_id: session.id.clone(),
            host: session.host.clone(),
            port: session.port,
            username: session.username.clone(),
            transfer_protocol: session.transfer_protocol.clone(),
            tftp_port: session.tftp_port,
            surface: TerminalSurface::new(80, 24, cursor_shape_from_str(&self.appearance.cursor_style)),
            backend,
            state: SshState::Connecting,
            just_created: true,
            sftp: SftpPanel::default(),
            tftp: TftpPanel::default(),
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
        let config = ssh_config_for(&session);
        let backend = SshTerminal::connect(config);
        if let Tab::Ssh(tab) = &mut self.tabs[index] {
            tab.surface = TerminalSurface::new(80, 24, cursor_shape_from_str(&self.appearance.cursor_style));
            tab.backend = backend;
            tab.state = SshState::Connecting;
            tab.just_created = true;
        }
    }

    fn toggle_sftp_panel(&mut self, index: usize) {
        let Some(tab) = self.tabs[index].as_ssh_mut() else {
            return;
        };
        tab.sftp.open = !tab.sftp.open;
        if tab.sftp.open && tab.sftp.backend.is_none() {
            let Some(session) = self.store.get_session_with_secrets(&tab.session_id) else {
                return;
            };
            let protocol = if session.transfer_protocol == "scp" {
                Protocol::Scp
            } else {
                Protocol::Sftp
            };
            let backend = SftpBackend::connect(ssh_config_for(&session), protocol);
            backend.list(&tab.sftp.path.clone());
            tab.sftp.backend = Some(backend);
            tab.sftp.status = "Loading…".to_string();
        }
    }

    fn toggle_tftp_panel(&mut self, index: usize) {
        if let Some(tab) = self.tabs[index].as_ssh_mut() {
            tab.tftp.open = !tab.tftp.open;
        }
    }

    fn start_tftp_upload(&mut self, index: usize) {
        let Some(tab) = self.tabs[index].as_ssh_mut() else {
            return;
        };
        let Some(local_path) = tab.tftp.local_path.clone() else {
            tab.tftp.status = "Choose a local file and enter a remote filename.".to_string();
            return;
        };
        let remote_file_name = tab.tftp.remote_file_name.trim().to_string();
        if remote_file_name.is_empty() {
            tab.tftp.status = "Choose a local file and enter a remote filename.".to_string();
            return;
        }
        let host = tab.host.clone();
        let port = tab.tftp_port;
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let result = tftp::upload(&host, port, &local_path, &remote_file_name);
            let _ = tx.send(TftpResult::Upload(result));
        });
        tab.tftp.pending = Some(rx);
        tab.tftp.status = "Uploading via TFTP…".to_string();
    }

    fn start_tftp_download(&mut self, index: usize) {
        let Some(tab) = self.tabs[index].as_ssh_mut() else {
            return;
        };
        let remote_file_name = tab.tftp.remote_file_name.trim().to_string();
        if remote_file_name.is_empty() {
            tab.tftp.status = "Enter the remote filename to fetch.".to_string();
            return;
        }
        let Some(dir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let host = tab.host.clone();
        let port = tab.tftp_port;
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let result = tftp::download(&host, port, &remote_file_name, &dir);
            let _ = tx.send(TftpResult::Download(result));
        });
        tab.tftp.pending = Some(rx);
        tab.tftp.status = "Downloading via TFTP…".to_string();
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

    fn apply_sftp_action(&mut self, tab_index: usize, action: SftpAction) {
        let Some(tab) = self.tabs[tab_index].as_ssh_mut() else {
            return;
        };
        match action {
            SftpAction::Refresh => {
                if let Some(b) = &tab.sftp.backend {
                    b.list(&tab.sftp.path.clone());
                    tab.sftp.status = "Loading…".to_string();
                }
            }
            SftpAction::Upload => {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    if let Some(b) = &tab.sftp.backend {
                        b.upload(files, &tab.sftp.path.clone());
                        tab.sftp.status = "Uploading…".to_string();
                    }
                }
            }
            SftpAction::Download => {
                let Some(idx) = tab.sftp.selected else {
                    tab.sftp.status = "Select a file first.".to_string();
                    return;
                };
                let Some(entry) = tab.sftp.entries.get(idx) else {
                    return;
                };
                if entry.is_dir {
                    tab.sftp.status = "Folder download not supported.".to_string();
                    return;
                }
                let remote_path = join_path(&tab.sftp.path, &entry.name);
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    if let Some(b) = &tab.sftp.backend {
                        b.download(vec![remote_path], dir);
                        tab.sftp.status = "Downloading…".to_string();
                    }
                }
            }
            SftpAction::NewFolder => {
                self.mkdir_modal = Some(MkdirDraft {
                    tab_index,
                    name: String::new(),
                });
            }
            SftpAction::Delete => {
                let Some(idx) = tab.sftp.selected else {
                    tab.sftp.status = "Select an item first.".to_string();
                    return;
                };
                let Some(entry) = tab.sftp.entries.get(idx) else {
                    return;
                };
                self.confirm_delete_sftp = Some(ConfirmDeleteSftpEntry {
                    tab_index,
                    path: join_path(&tab.sftp.path, &entry.name),
                    name: entry.name.clone(),
                    is_dir: entry.is_dir,
                });
            }
            SftpAction::Navigate(path) => {
                tab.sftp.selected = None;
                if let Some(b) = &tab.sftp.backend {
                    b.list(&path);
                    tab.sftp.status = "Loading…".to_string();
                }
            }
            SftpAction::EnterDir(path) => {
                tab.sftp.selected = None;
                if let Some(b) = &tab.sftp.backend {
                    b.list(&path);
                    tab.sftp.status = "Loading…".to_string();
                }
            }
            SftpAction::Select(idx) => {
                tab.sftp.selected = Some(idx);
            }
        }
    }

    fn mkdir_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.mkdir_modal else {
            return;
        };
        let mut close = false;
        let mut create = false;

        egui::Modal::new(egui::Id::unique("mkdir_modal")).show(ctx, |ui| {
            ui.set_min_width(260.0);
            ui.label("New folder name:");
            ui.text_edit_singleline(&mut draft.name);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if ui.add_enabled(!draft.name.trim().is_empty(), egui::Button::new("Create")).clicked() {
                    create = true;
                }
            });
        });

        if create {
            if let Some(draft) = self.mkdir_modal.take() {
                if let Some(tab) = self.tabs[draft.tab_index].as_ssh_mut() {
                    let path = join_path(&tab.sftp.path, draft.name.trim());
                    if let Some(b) = &tab.sftp.backend {
                        b.mkdir(&path);
                    }
                }
            }
        } else if close {
            self.mkdir_modal = None;
        }
    }

    fn confirm_delete_sftp_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.confirm_delete_sftp else {
            return;
        };
        let mut delete = false;
        let mut cancel = false;

        egui::Modal::new(egui::Id::unique("confirm_delete_sftp")).show(ctx, |ui| {
            ui.set_min_width(280.0);
            ui.label(format!("Delete \"{}\"?", pending.name));
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
            if let Some(pending) = self.confirm_delete_sftp.take() {
                if let Some(tab) = self.tabs[pending.tab_index].as_ssh_mut() {
                    if let Some(b) = &tab.sftp.backend {
                        b.delete(&pending.path, pending.is_dir);
                        tab.sftp.status = "Deleting…".to_string();
                    }
                }
            }
        } else if cancel {
            self.confirm_delete_sftp = None;
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

    fn send_quick_command(&mut self, idx: usize) {
        let Some(cmd) = self.quick_commands.get(idx) else {
            return;
        };
        let Some(i) = self.active_tab else {
            return;
        };
        let text = if cmd.run_enter {
            let mut t = cmd.command.trim_end_matches('\n').to_string();
            t.push('\r');
            t
        } else {
            cmd.command.clone()
        };
        self.tabs[i].write(text.as_bytes());
    }

    fn cmd_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.cmd_modal else {
            return;
        };
        let is_new = draft.editing_id.is_none();
        let mut close = false;
        let mut save = false;

        egui::Modal::new(egui::Id::unique("cmd_modal")).show(ctx, |ui| {
            ui.set_min_width(340.0);
            ui.heading(if is_new { "New Command" } else { "Edit Command" });
            ui.separator();
            ui.label("Name");
            ui.text_edit_singleline(&mut draft.name);
            ui.label("Command");
            ui.add(egui::TextEdit::multiline(&mut draft.command).desired_rows(4));
            ui.checkbox(&mut draft.run_enter, "Press Enter after sending");
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                let can_save = !draft.name.trim().is_empty() && !draft.command.is_empty();
                if ui.add_enabled(can_save, egui::Button::new("Save")).clicked() {
                    save = true;
                }
            });
        });

        if save {
            if let Some(draft) = self.cmd_modal.take() {
                let cmd = QuickCommand {
                    id: draft.editing_id.unwrap_or_default(),
                    name: draft.name,
                    command: draft.command,
                    run_enter: draft.run_enter,
                };
                self.store.save_quick_command(cmd);
                self.quick_commands = self.store.quick_commands();
            }
        } else if close {
            self.cmd_modal = None;
        }
    }

    fn confirm_delete_cmd_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.confirm_delete_cmd else {
            return;
        };
        let mut delete = false;
        let mut cancel = false;

        egui::Modal::new(egui::Id::unique("confirm_delete_cmd")).show(ctx, |ui| {
            ui.set_min_width(280.0);
            ui.label(format!("Delete command \"{}\"?", pending.name));
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
            if let Some(pending) = self.confirm_delete_cmd.take() {
                self.store.delete_quick_command(&pending.id);
                self.quick_commands = self.store.quick_commands();
            }
        } else if cancel {
            self.confirm_delete_cmd = None;
        }
    }
}

enum SftpAction {
    Refresh,
    Upload,
    Download,
    NewFolder,
    Delete,
    Navigate(String),
    Select(usize),
    EnterDir(String),
}

fn join_path(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_of(path: &str) -> String {
    if path == "/" || path == "." {
        return path.to_string();
    }
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((head, _)) => head.to_string(),
        None => if path.starts_with('/') { "/".to_string() } else { ".".to_string() },
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.cmd_panel_open, "⌘ Commands").clicked() {
                        self.cmd_panel_open = !self.cmd_panel_open;
                    }
                });
            });
        });

        if let Some(i) = close_requested {
            self.close_tab(i);
        }

        let mut sftp_toggle: Option<usize> = None;
        let mut tftp_toggle: Option<usize> = None;
        if let Some(i) = self.active_tab {
            egui::Panel::top("term_toolbar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &self.tabs[i] {
                        Tab::Ssh(t) => {
                            ui.label(format!("{}@{}:{}", t.username, t.host, t.port));
                        }
                        Tab::Local(_) => {
                            ui.label("Local Terminal");
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Tab::Ssh(t) = &self.tabs[i] {
                            match t.transfer_protocol.as_str() {
                                "tftp" => {
                                    if ui.selectable_label(t.tftp.open, "TFTP").clicked() {
                                        tftp_toggle = Some(i);
                                    }
                                }
                                "scp" => {
                                    if ui.selectable_label(t.sftp.open, "SCP").clicked() {
                                        sftp_toggle = Some(i);
                                    }
                                }
                                _ => {
                                    if ui.selectable_label(t.sftp.open, "SFTP").clicked() {
                                        sftp_toggle = Some(i);
                                    }
                                }
                            }
                        }
                    });
                });
            });
        }
        if let Some(i) = sftp_toggle {
            self.toggle_sftp_panel(i);
        }
        if let Some(i) = tftp_toggle {
            self.toggle_tftp_panel(i);
        }

        let mut sftp_action: Option<SftpAction> = None;
        if let Some(i) = self.active_tab {
            let sftp_open = matches!(&self.tabs[i], Tab::Ssh(t) if t.sftp.open);
            if sftp_open {
                egui::Panel::right("sftp_panel")
                    .resizable(true)
                    .default_size(340.0)
                    .show(ui, |ui| {
                        let Tab::Ssh(tab) = &self.tabs[i] else { return };
                        ui.horizontal_wrapped(|ui| {
                            if ui.small_button("⟳ Refresh").clicked() {
                                sftp_action = Some(SftpAction::Refresh);
                            }
                            if ui.small_button("⬆ Upload").clicked() {
                                sftp_action = Some(SftpAction::Upload);
                            }
                            if ui.small_button("⬇ Download").clicked() {
                                sftp_action = Some(SftpAction::Download);
                            }
                            if ui.small_button("+ Folder").clicked() {
                                sftp_action = Some(SftpAction::NewFolder);
                            }
                            if ui.small_button("🗑 Delete").clicked() {
                                sftp_action = Some(SftpAction::Delete);
                            }
                        });
                        ui.separator();

                        ui.horizontal_wrapped(|ui| {
                            let is_abs = tab.sftp.path.starts_with('/');
                            let root_label = if is_abs { "/" } else { "." };
                            if ui.link(root_label).clicked() {
                                sftp_action = Some(SftpAction::Navigate(root_label.to_string()));
                            }
                            let mut acc = String::new();
                            for part in tab.sftp.path.split('/').filter(|p| !p.is_empty()) {
                                ui.label("/");
                                acc = if acc.is_empty() {
                                    if is_abs {
                                        format!("/{part}")
                                    } else {
                                        part.to_string()
                                    }
                                } else {
                                    format!("{acc}/{part}")
                                };
                                if ui.link(part).clicked() {
                                    sftp_action = Some(SftpAction::Navigate(acc.clone()));
                                }
                            }
                        });
                        ui.separator();

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if tab.sftp.path != "." && tab.sftp.path != "/" {
                                if ui.selectable_label(false, "📁 ..").double_clicked() {
                                    sftp_action = Some(SftpAction::Navigate(parent_of(&tab.sftp.path)));
                                }
                            }
                            for (idx, entry) in tab.sftp.entries.iter().enumerate() {
                                let icon = if entry.is_dir {
                                    "📁"
                                } else if entry.is_symlink {
                                    "🔗"
                                } else {
                                    "📄"
                                };
                                let size = if entry.is_dir {
                                    String::new()
                                } else {
                                    format_size(entry.size)
                                };
                                let selected = tab.sftp.selected == Some(idx);
                                let resp = ui.selectable_label(selected, format!("{icon} {}  {size}", entry.name));
                                if resp.clicked() {
                                    sftp_action = Some(SftpAction::Select(idx));
                                }
                                if resp.double_clicked() && entry.is_dir {
                                    sftp_action =
                                        Some(SftpAction::EnterDir(join_path(&tab.sftp.path, &entry.name)));
                                }
                            }
                        });
                        ui.separator();
                        ui.label(&tab.sftp.status);
                    });
            }
        }
        if let (Some(i), Some(action)) = (self.active_tab, sftp_action) {
            self.apply_sftp_action(i, action);
        }

        let mut tftp_browse: Option<usize> = None;
        let mut tftp_upload: Option<usize> = None;
        let mut tftp_download: Option<usize> = None;
        if let Some(i) = self.active_tab {
            let tftp_open = matches!(&self.tabs[i], Tab::Ssh(t) if t.tftp.open);
            if tftp_open {
                egui::Panel::right("tftp_panel")
                    .resizable(true)
                    .default_size(320.0)
                    .show(ui, |ui| {
                        let Tab::Ssh(tab) = &mut self.tabs[i] else { return };
                        ui.heading("TFTP Transfer");
                        ui.separator();
                        ui.label("Local file");
                        ui.horizontal(|ui| {
                            let label = tab
                                .tftp
                                .local_path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "No file selected".to_string());
                            ui.label(label);
                            if ui.button("Browse…").clicked() {
                                tftp_browse = Some(i);
                            }
                        });
                        ui.label("Remote filename");
                        ui.text_edit_singleline(&mut tab.tftp.remote_file_name);
                        ui.add_space(8.0);
                        if ui.button("⬆ Upload to device").clicked() {
                            tftp_upload = Some(i);
                        }
                        if ui.button("⬇ Download from device").clicked() {
                            tftp_download = Some(i);
                        }
                        ui.separator();
                        ui.label(&tab.tftp.status);
                    });
            }
        }
        if let Some(i) = tftp_browse {
            if let Some(file) = rfd::FileDialog::new().pick_file() {
                if let Some(tab) = self.tabs[i].as_ssh_mut() {
                    if tab.tftp.remote_file_name.is_empty() {
                        if let Some(name) = file.file_name() {
                            tab.tftp.remote_file_name = name.to_string_lossy().to_string();
                        }
                    }
                    tab.tftp.local_path = Some(file);
                }
            }
        }
        if let Some(i) = tftp_upload {
            self.start_tftp_upload(i);
        }
        if let Some(i) = tftp_download {
            self.start_tftp_download(i);
        }

        if self.cmd_panel_open {
            let mut edit_cmd: Option<usize> = None;
            let mut delete_cmd: Option<usize> = None;
            let mut send_cmd: Option<usize> = None;
            egui::Panel::right("cmd_panel")
                .resizable(true)
                .default_size(260.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Quick Commands");
                        if ui.small_button("+").clicked() {
                            self.cmd_modal = Some(QuickCommandDraft::new());
                        }
                    });
                    ui.separator();
                    if self.quick_commands.is_empty() {
                        ui.label("No quick commands yet. Click + to add one.");
                    }
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (idx, cmd) in self.quick_commands.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let preview = cmd.command.lines().next().unwrap_or("");
                                if ui
                                    .add(egui::Label::new(format!("{}\n{preview}", cmd.name)).sense(egui::Sense::click()))
                                    .on_hover_text(&cmd.command)
                                    .clicked()
                                {
                                    send_cmd = Some(idx);
                                }
                                if ui.small_button("✎").clicked() {
                                    edit_cmd = Some(idx);
                                }
                                if ui.small_button("🗑").clicked() {
                                    delete_cmd = Some(idx);
                                }
                            });
                        }
                    });
                });
            if let Some(idx) = send_cmd {
                self.send_quick_command(idx);
            }
            if let Some(idx) = edit_cmd {
                self.cmd_modal = Some(QuickCommandDraft::from_command(&self.quick_commands[idx]));
            }
            if let Some(idx) = delete_cmd {
                self.confirm_delete_cmd = Some(ConfirmDeleteCommand {
                    id: self.quick_commands[idx].id.clone(),
                    name: self.quick_commands[idx].name.clone(),
                });
            }
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
        self.mkdir_modal_ui(&ctx);
        self.confirm_delete_sftp_ui(&ctx);
        self.cmd_modal_ui(&ctx);
        self.confirm_delete_cmd_ui(&ctx);

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
