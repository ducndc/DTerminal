use crate::backend::local::{LocalEvent, LocalTerminal};
use crate::backend::sftp::{SftpBackend, SftpEntry, SftpEvent};
use crate::backend::ssh::{SshEvent, SshTerminal};
use crate::term::TerminalSurface;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

pub enum TftpResult {
    Upload(Result<u64, String>),
    Download(Result<(PathBuf, u64), String>),
}

pub struct TftpPanel {
    pub open: bool,
    pub local_path: Option<PathBuf>,
    pub remote_file_name: String,
    pub status: String,
    pub pending: Option<Receiver<TftpResult>>,
}

impl Default for TftpPanel {
    fn default() -> Self {
        Self {
            open: false,
            local_path: None,
            remote_file_name: String::new(),
            status: String::new(),
            pending: None,
        }
    }
}

pub struct SftpPanel {
    pub open: bool,
    pub backend: Option<SftpBackend>,
    pub path: String,
    pub entries: Vec<SftpEntry>,
    pub selected: Option<usize>,
    pub status: String,
}

impl Default for SftpPanel {
    fn default() -> Self {
        Self {
            open: false,
            backend: None,
            path: ".".to_string(),
            entries: Vec::new(),
            selected: None,
            status: String::new(),
        }
    }
}

pub struct LocalTab {
    pub title: String,
    pub surface: TerminalSurface,
    pub backend: LocalTerminal,
    pub connected: bool,
    pub just_created: bool,
}

pub enum SshState {
    Connecting,
    Connected,
    Error(String),
    Closed,
}

pub struct SshTab {
    pub title: String,
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// "sftp" | "scp" | "tftp" — picks which transfer panel the toolbar shows.
    pub transfer_protocol: String,
    pub tftp_port: u16,
    pub surface: TerminalSurface,
    pub backend: SshTerminal,
    pub state: SshState,
    pub just_created: bool,
    pub sftp: SftpPanel,
    pub tftp: TftpPanel,
}

pub enum Tab {
    Local(LocalTab),
    Ssh(SshTab),
}

impl Tab {
    pub fn title(&self) -> &str {
        match self {
            Tab::Local(t) => &t.title,
            Tab::Ssh(t) => &t.title,
        }
    }

    pub fn surface(&self) -> &TerminalSurface {
        match self {
            Tab::Local(t) => &t.surface,
            Tab::Ssh(t) => &t.surface,
        }
    }

    pub fn surface_mut(&mut self) -> &mut TerminalSurface {
        match self {
            Tab::Local(t) => &mut t.surface,
            Tab::Ssh(t) => &mut t.surface,
        }
    }

    /// Whether writes/resizes can currently reach the backend.
    pub fn is_live(&self) -> bool {
        match self {
            Tab::Local(t) => t.connected,
            Tab::Ssh(t) => matches!(t.state, SshState::Connected),
        }
    }

    /// Short status line shown over the terminal while connecting or after
    /// a disconnect — `None` once a local/SSH tab is live.
    pub fn status_line(&self) -> Option<String> {
        match self {
            Tab::Local(t) if !t.connected => Some("Disconnected".to_string()),
            Tab::Local(_) => None,
            Tab::Ssh(t) => match &t.state {
                SshState::Connecting => Some("Connecting…".to_string()),
                SshState::Connected => None,
                SshState::Error(msg) => Some(format!("Disconnected: {msg}")),
                SshState::Closed => Some("Disconnected".to_string()),
            },
        }
    }

    /// Whether a "Reconnect" button should show in the status overlay —
    /// true for a disconnected/errored SSH tab, never while still connecting.
    pub fn can_reconnect(&self) -> bool {
        matches!(self, Tab::Ssh(t) if !matches!(t.state, SshState::Connecting | SshState::Connected))
    }

    pub fn take_just_created(&mut self) -> bool {
        match self {
            Tab::Local(t) => std::mem::take(&mut t.just_created),
            Tab::Ssh(t) => std::mem::take(&mut t.just_created),
        }
    }

    pub fn pump_events(&mut self) {
        match self {
            Tab::Local(t) => loop {
                match t.backend.events.try_recv() {
                    Ok(LocalEvent::Data(bytes)) => t.surface.feed(&bytes),
                    Ok(LocalEvent::Exited) => {
                        t.connected = false;
                        break;
                    }
                    Err(_) => break,
                }
            },
            Tab::Ssh(t) => {
                loop {
                    match t.backend.events.try_recv() {
                        Ok(SshEvent::Connected) => t.state = SshState::Connected,
                        Ok(SshEvent::Data(bytes)) => t.surface.feed(&bytes),
                        Ok(SshEvent::Error(msg)) => t.state = SshState::Error(msg),
                        Ok(SshEvent::Closed) => {
                            if !matches!(t.state, SshState::Error(_)) {
                                t.state = SshState::Closed;
                            }
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if let Some(backend) = &t.sftp.backend {
                    let mut connect_failed = false;
                    loop {
                        match backend.events.try_recv() {
                            Ok(SftpEvent::Listed { path, entries }) => {
                                t.sftp.path = path;
                                t.sftp.entries = entries;
                                t.sftp.selected = None;
                                t.sftp.status = format!("{} item(s)", t.sftp.entries.len());
                            }
                            Ok(SftpEvent::Progress { file, done, error }) => {
                                t.sftp.status = match (done, error) {
                                    (true, _) => format!("{file}: done"),
                                    (false, Some(e)) => format!("{file}: failed — {e}"),
                                    (false, None) => format!("{file}: failed"),
                                };
                            }
                            Ok(SftpEvent::Error(msg)) => t.sftp.status = format!("Error: {msg}"),
                            Ok(SftpEvent::ConnectError(msg)) => {
                                t.sftp.status = format!("Connect failed: {msg}");
                                connect_failed = true;
                            }
                            Err(_) => break,
                        }
                    }
                    if connect_failed {
                        t.sftp.backend = None;
                    }
                }
                let mut clear_pending = false;
                if let Some(rx) = &t.tftp.pending {
                    match rx.try_recv() {
                        Ok(TftpResult::Upload(Ok(bytes))) => {
                            t.tftp.status = format!("Upload complete ({bytes} bytes).");
                            clear_pending = true;
                        }
                        Ok(TftpResult::Upload(Err(e))) => {
                            t.tftp.status = format!("Upload failed: {e}");
                            clear_pending = true;
                        }
                        Ok(TftpResult::Download(Ok((path, bytes)))) => {
                            t.tftp.status = format!("Saved to {} ({bytes} bytes).", path.display());
                            clear_pending = true;
                        }
                        Ok(TftpResult::Download(Err(e))) => {
                            t.tftp.status = format!("Download failed: {e}");
                            clear_pending = true;
                        }
                        Err(_) => {}
                    }
                }
                if clear_pending {
                    t.tftp.pending = None;
                }
            }
        }
    }

    pub fn write(&mut self, data: &[u8]) {
        if !self.is_live() {
            return;
        }
        match self {
            Tab::Local(t) => t.backend.write(data),
            Tab::Ssh(t) => t.backend.write(data),
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.surface_mut().resize(cols, rows);
        if !self.is_live() {
            return;
        }
        match self {
            Tab::Local(t) => t.backend.resize(cols, rows),
            Tab::Ssh(t) => t.backend.resize(cols, rows),
        }
    }

    pub fn as_ssh_mut(&mut self) -> Option<&mut SshTab> {
        match self {
            Tab::Ssh(t) => Some(t),
            Tab::Local(_) => None,
        }
    }

    pub fn close(&mut self) {
        if let Tab::Local(t) = self {
            t.backend.kill();
        }
        // SshTerminal disconnects when dropped (dropping the tab drops it).
    }
}
