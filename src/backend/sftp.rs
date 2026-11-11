use super::ssh::{connect_session, SshConfig};
use ssh2::Sftp;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(Clone)]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
}

pub enum SftpCommand {
    List(String),
    Mkdir(String),
    Delete(String, bool),
    Upload(Vec<PathBuf>, String),
    Download(Vec<String>, PathBuf),
}

pub enum SftpEvent {
    Listed { path: String, entries: Vec<SftpEntry> },
    Progress { file: String, done: bool, error: Option<String> },
    Error(String),
    ConnectError(String),
}

pub struct SftpBackend {
    cmd_tx: Sender<SftpCommand>,
    pub events: Receiver<SftpEvent>,
}

fn sort_entries(entries: &mut Vec<SftpEntry>) {
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
}

fn list_dir(sftp: &Sftp, path: &str) -> Result<(String, Vec<SftpEntry>), String> {
    let target = if path.is_empty() { "." } else { path };
    let raw = sftp.readdir(Path::new(target)).map_err(|e| e.to_string())?;
    let mut entries: Vec<SftpEntry> = raw
        .into_iter()
        .map(|(full_path, stat)| {
            let name = full_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            SftpEntry {
                name,
                is_dir: stat.is_dir(),
                is_symlink: stat.file_type().is_symlink(),
                size: stat.size.unwrap_or(0),
            }
        })
        .collect();
    sort_entries(&mut entries);
    Ok((target.to_string(), entries))
}

fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

impl SftpBackend {
    pub fn connect(config: SshConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SftpCommand>();
        let (event_tx, event_rx) = mpsc::channel::<SftpEvent>();

        thread::spawn(move || {
            let sess = match connect_session(&config) {
                Ok(s) => s,
                Err(msg) => {
                    let _ = event_tx.send(SftpEvent::ConnectError(msg));
                    return;
                }
            };
            let sftp = match sess.sftp() {
                Ok(s) => s,
                Err(e) => {
                    let _ = event_tx.send(SftpEvent::ConnectError(e.to_string()));
                    return;
                }
            };

            for cmd in cmd_rx {
                match cmd {
                    SftpCommand::List(path) => match list_dir(&sftp, &path) {
                        Ok((path, entries)) => {
                            let _ = event_tx.send(SftpEvent::Listed { path, entries });
                        }
                        Err(msg) => {
                            let _ = event_tx.send(SftpEvent::Error(msg));
                        }
                    },
                    SftpCommand::Mkdir(path) => {
                        if let Err(e) = sftp.mkdir(Path::new(&path), 0o755) {
                            let _ = event_tx.send(SftpEvent::Error(e.to_string()));
                        }
                    }
                    SftpCommand::Delete(path, is_dir) => {
                        let result = if is_dir {
                            sftp.rmdir(Path::new(&path))
                        } else {
                            sftp.unlink(Path::new(&path))
                        };
                        if let Err(e) = result {
                            let _ = event_tx.send(SftpEvent::Error(e.to_string()));
                        }
                    }
                    SftpCommand::Upload(local_paths, remote_dir) => {
                        for local_path in local_paths {
                            let file_name = local_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let remote_path = join_remote(&remote_dir, &file_name);
                            let result = (|| -> Result<(), String> {
                                let mut remote_file =
                                    sftp.create(Path::new(&remote_path)).map_err(|e| e.to_string())?;
                                let mut local_file =
                                    std::fs::File::open(&local_path).map_err(|e| e.to_string())?;
                                let mut buf = [0u8; 65536];
                                loop {
                                    let n = local_file.read(&mut buf).map_err(|e| e.to_string())?;
                                    if n == 0 {
                                        break;
                                    }
                                    remote_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                                }
                                Ok(())
                            })();
                            let _ = event_tx.send(SftpEvent::Progress {
                                file: file_name,
                                done: result.is_ok(),
                                error: result.err(),
                            });
                        }
                    }
                    SftpCommand::Download(remote_paths, local_dir) => {
                        for remote_path in remote_paths {
                            let file_name = remote_path.rsplit('/').next().unwrap_or(&remote_path).to_string();
                            let local_path = local_dir.join(&file_name);
                            let result = (|| -> Result<(), String> {
                                let mut remote_file =
                                    sftp.open(Path::new(&remote_path)).map_err(|e| e.to_string())?;
                                let mut local_file =
                                    std::fs::File::create(&local_path).map_err(|e| e.to_string())?;
                                let mut buf = [0u8; 65536];
                                loop {
                                    let n = remote_file.read(&mut buf).map_err(|e| e.to_string())?;
                                    if n == 0 {
                                        break;
                                    }
                                    local_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                                }
                                Ok(())
                            })();
                            let _ = event_tx.send(SftpEvent::Progress {
                                file: file_name,
                                done: result.is_ok(),
                                error: result.err(),
                            });
                        }
                    }
                }
            }
        });

        Self {
            cmd_tx,
            events: event_rx,
        }
    }

    pub fn list(&self, path: &str) {
        let _ = self.cmd_tx.send(SftpCommand::List(path.to_string()));
    }
    pub fn mkdir(&self, path: &str) {
        let _ = self.cmd_tx.send(SftpCommand::Mkdir(path.to_string()));
    }
    pub fn delete(&self, path: &str, is_dir: bool) {
        let _ = self.cmd_tx.send(SftpCommand::Delete(path.to_string(), is_dir));
    }
    pub fn upload(&self, local_paths: Vec<PathBuf>, remote_dir: &str) {
        let _ = self
            .cmd_tx
            .send(SftpCommand::Upload(local_paths, remote_dir.to_string()));
    }
    pub fn download(&self, remote_paths: Vec<String>, local_dir: PathBuf) {
        let _ = self.cmd_tx.send(SftpCommand::Download(remote_paths, local_dir));
    }
}
