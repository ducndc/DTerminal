pub mod secret;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// "password" | "key"
    pub auth_type: String,
    #[serde(default)]
    pub key_path: String,
    /// "sftp" | "scp" | "tftp"
    #[serde(default = "default_transfer_protocol")]
    pub transfer_protocol: String,
    #[serde(default = "default_tftp_port")]
    pub tftp_port: u16,
    /// Encrypted at rest ("enc:"/"plain:" tagged) — see `secret.rs`.
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub passphrase: String,
}

fn default_transfer_protocol() -> String {
    "sftp".to_string()
}

fn default_tftp_port() -> u16 {
    69
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Appearance {
    pub theme: String,
    pub font_family: String,
    pub font_size: f32,
    /// "block" | "bar" | "underline"
    pub cursor_style: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            font_family: "DejaVu Sans Mono, monospace".to_string(),
            font_size: 14.0,
            cursor_style: "block".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickCommand {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default = "default_true")]
    pub run_enter: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoreData {
    #[serde(default)]
    sessions: Vec<Session>,
    #[serde(default)]
    appearance: Option<Appearance>,
    #[serde(default)]
    quick_commands: Vec<QuickCommand>,
}

pub struct Store {
    path: PathBuf,
    data: StoreData,
}

impl Store {
    pub fn load() -> Self {
        let path = store_path();
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self { path, data }
    }

    fn persist(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(raw) = serde_json::to_string_pretty(&self.data) else {
            return;
        };
        let _ = fs::write(&self.path, raw);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(&self.path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o600);
                let _ = fs::set_permissions(&self.path, perms);
            }
        }
    }

    pub fn list_sessions(&self) -> Vec<Session> {
        self.data
            .sessions
            .iter()
            .cloned()
            .map(|mut s| {
                s.password.clear();
                s.passphrase.clear();
                s
            })
            .collect()
    }

    pub fn get_session_with_secrets(&self, id: &str) -> Option<Session> {
        self.data.sessions.iter().find(|s| s.id == id).map(|s| {
            let mut s = s.clone();
            s.password = secret::decrypt_secret(&s.password);
            s.passphrase = secret::decrypt_secret(&s.passphrase);
            s
        })
    }

    /// `new_password`/`new_passphrase` are `None` when the editor's field was
    /// left blank — meaning "keep the existing secret", same convention as
    /// `sessionStore.js`'s save path.
    pub fn save_session(
        &mut self,
        mut session: Session,
        new_password: Option<String>,
        new_passphrase: Option<String>,
    ) -> Session {
        if session.id.is_empty() {
            session.id = new_id();
        }
        let idx = self.data.sessions.iter().position(|s| s.id == session.id);

        session.password = match new_password {
            Some(p) => secret::encrypt_secret(&p),
            None => idx
                .map(|i| self.data.sessions[i].password.clone())
                .unwrap_or_default(),
        };
        session.passphrase = match new_passphrase {
            Some(p) => secret::encrypt_secret(&p),
            None => idx
                .map(|i| self.data.sessions[i].passphrase.clone())
                .unwrap_or_default(),
        };

        match idx {
            Some(i) => self.data.sessions[i] = session.clone(),
            None => self.data.sessions.push(session.clone()),
        }
        self.persist();

        session.password.clear();
        session.passphrase.clear();
        session
    }

    pub fn delete_session(&mut self, id: &str) {
        self.data.sessions.retain(|s| s.id != id);
        self.persist();
    }

    pub fn appearance(&self) -> Appearance {
        self.data.appearance.clone().unwrap_or_default()
    }

    pub fn save_appearance(&mut self, appearance: Appearance) {
        self.data.appearance = Some(appearance);
        self.persist();
    }

    pub fn quick_commands(&self) -> Vec<QuickCommand> {
        self.data.quick_commands.clone()
    }

    pub fn save_quick_command(&mut self, mut cmd: QuickCommand) -> QuickCommand {
        if cmd.id.is_empty() {
            cmd.id = new_id();
        }
        match self.data.quick_commands.iter().position(|c| c.id == cmd.id) {
            Some(i) => self.data.quick_commands[i] = cmd.clone(),
            None => self.data.quick_commands.push(cmd.clone()),
        }
        self.persist();
        cmd
    }

    pub fn delete_quick_command(&mut self, id: &str) {
        self.data.quick_commands.retain(|c| c.id != id);
        self.persist();
    }
}

fn new_id() -> String {
    let mut buf = [0u8; 16];
    if getrandom::fill(&mut buf).is_ok() {
        buf.iter().map(|b| format!("{b:02x}")).collect()
    } else {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    }
}

fn store_path() -> PathBuf {
    directories::ProjectDirs::from("com", "dterminal", "DTerminal")
        .map(|d| d.data_dir().join("dterminal-store.json"))
        .unwrap_or_else(|| PathBuf::from("dterminal-store.json"))
}
