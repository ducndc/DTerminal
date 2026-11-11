use ssh2::{KeyboardInteractivePrompt, Prompt, Session};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

pub enum SshEvent {
    Connected,
    Data(Vec<u8>),
    Error(String),
    Closed,
}

#[derive(Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub key_path: String,
    pub passphrase: String,
    pub use_key: bool,
    pub cols: usize,
    pub rows: usize,
}

enum SshCommand {
    Write(Vec<u8>),
    Resize(usize, usize),
}

/// A remote shell session over SSH — mirrors `sshManager.js`'s shell-channel
/// behavior, feeding the exact same `TerminalSurface` the local-pty backend
/// does. libssh2 (via the `ssh2` crate) is not safe to touch from more than
/// one thread, so the whole connection lives on one dedicated thread; this
/// handle only holds a command sender and the event receiver.
pub struct SshTerminal {
    cmd_tx: Sender<SshCommand>,
    pub events: Receiver<SshEvent>,
}

/// Answers every keyboard-interactive prompt with the session's password,
/// the same blanket strategy `sshManager.js`'s `keyboard-interactive` handler
/// used (`finish(prompts.map(() => config.password || ''))`).
struct PasswordPrompter {
    password: String,
}

impl KeyboardInteractivePrompt for PasswordPrompter {
    fn prompt<'a>(&mut self, _username: &str, _instructions: &str, prompts: &[Prompt<'a>]) -> Vec<String> {
        prompts.iter().map(|_| self.password.clone()).collect()
    }
}

/// Connects and authenticates, leaving the session in blocking mode —
/// shared by the shell-channel backend here and the SFTP backend, which
/// each open their own independent connection (libssh2 sessions aren't
/// safe to share across threads).
pub fn connect_session(config: &SshConfig) -> Result<Session, String> {
    let tcp = TcpStream::connect((config.host.as_str(), config.port)).map_err(|e| e.to_string())?;
    tcp.set_nodelay(true).ok();

    let mut sess = Session::new().map_err(|e| e.to_string())?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(|e| format!("Handshake failed: {e}"))?;

    if config.use_key {
        let key_path = PathBuf::from(&config.key_path);
        let passphrase = (!config.passphrase.is_empty()).then(|| config.passphrase.as_str());
        sess.userauth_pubkey_file(&config.username, None, &key_path, passphrase)
            .map_err(|e| format!("Key auth failed: {e}"))?;
    } else {
        if sess.userauth_password(&config.username, &config.password).is_err() {
            let mut prompter = PasswordPrompter {
                password: config.password.clone(),
            };
            sess.userauth_keyboard_interactive(&config.username, &mut prompter)
                .map_err(|e| format!("Authentication failed: {e}"))?;
        }
    }

    if !sess.authenticated() {
        return Err("Authentication failed".to_string());
    }

    Ok(sess)
}

fn connect_and_open_shell(config: &SshConfig) -> Result<(Session, ssh2::Channel), String> {
    let sess = connect_session(config)?;
    let mut channel = sess.channel_session().map_err(|e| e.to_string())?;
    channel
        .request_pty(
            "xterm-256color",
            None,
            Some((config.cols as u32, config.rows as u32, 0, 0)),
        )
        .map_err(|e| e.to_string())?;
    channel.shell().map_err(|e| e.to_string())?;

    Ok((sess, channel))
}

impl SshTerminal {
    pub fn connect(config: SshConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SshCommand>();
        let (event_tx, event_rx) = mpsc::channel::<SshEvent>();

        thread::spawn(move || {
            let (sess, mut channel) = match connect_and_open_shell(&config) {
                Ok(v) => v,
                Err(msg) => {
                    let _ = event_tx.send(SshEvent::Error(msg));
                    let _ = event_tx.send(SshEvent::Closed);
                    return;
                }
            };
            let _ = event_tx.send(SshEvent::Connected);
            sess.set_blocking(false);

            let mut buf = [0u8; 8192];
            loop {
                loop {
                    match cmd_rx.try_recv() {
                        Ok(SshCommand::Write(data)) => {
                            let mut offset = 0;
                            while offset < data.len() {
                                match channel.write(&data[offset..]) {
                                    Ok(0) => break,
                                    Ok(n) => offset += n,
                                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                        thread::sleep(Duration::from_millis(5));
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                        Ok(SshCommand::Resize(cols, rows)) => {
                            let _ = channel.request_pty_size(cols as u32, rows as u32, None, None);
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            let _ = channel.close();
                            let _ = event_tx.send(SshEvent::Closed);
                            return;
                        }
                    }
                }

                match channel.read(&mut buf) {
                    Ok(0) => {
                        if channel.eof() {
                            let _ = event_tx.send(SshEvent::Closed);
                            return;
                        }
                    }
                    Ok(n) => {
                        if event_tx.send(SshEvent::Data(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15));
                    }
                    Err(e) => {
                        let _ = event_tx.send(SshEvent::Error(e.to_string()));
                        let _ = event_tx.send(SshEvent::Closed);
                        return;
                    }
                }
            }
        });

        Self {
            cmd_tx,
            events: event_rx,
        }
    }

    pub fn write(&self, data: &[u8]) {
        let _ = self.cmd_tx.send(SshCommand::Write(data.to_vec()));
    }

    pub fn resize(&self, cols: usize, rows: usize) {
        let _ = self.cmd_tx.send(SshCommand::Resize(cols, rows));
    }
}
