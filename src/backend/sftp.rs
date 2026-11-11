use super::ssh::{connect_session, SshConfig};
use regex::Regex;
use ssh2::{Session, Sftp};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// Which wire protocol backs this file-transfer connection — mirrors the
/// `transferProtocol` session setting (`sftp` default, or `scp` for hosts
/// with no SFTP subsystem but a working `/bin/scp` + shell).
#[derive(Clone, Copy, PartialEq)]
pub enum Protocol {
    Sftp,
    Scp,
}

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

fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------- SFTP mode

fn sftp_list(sftp: &Sftp, path: &str) -> Result<(String, Vec<SftpEntry>), String> {
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

// ----------------------------------------------------------------- SCP mode

fn exec_capture(sess: &Session, command: &str) -> Result<(String, i32), String> {
    let mut channel = sess.channel_session().map_err(|e| e.to_string())?;
    channel.exec(command).map_err(|e| e.to_string())?;
    let mut out = String::new();
    channel.read_to_string(&mut out).map_err(|e| e.to_string())?;
    channel.wait_close().map_err(|e| e.to_string())?;
    let code = channel.exit_status().unwrap_or(0);
    Ok((out, code))
}

fn exec_simple(sess: &Session, command: &str) -> Result<(), String> {
    let (out, code) = exec_capture(sess, command)?;
    if code != 0 {
        return Err(if out.trim().is_empty() {
            format!("Command failed with exit code {code}")
        } else {
            out.trim().to_string()
        });
    }
    Ok(())
}

fn parse_ls_output(out: &str) -> Vec<SftpEntry> {
    // Matches both GNU and BusyBox `ls -la` long-listing lines.
    let re = Regex::new(
        r"^([bcdlsp-][-rwxsStT]{9})\s+\d+\s+\S+\s+\S+\s+(\d+)\s+\w+\s+\d+\s+[\d:]+\s+(.+)$",
    )
    .expect("valid regex");
    let mut entries = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with("total ") {
            continue;
        }
        let Some(caps) = re.captures(line) else {
            continue;
        };
        let perms = &caps[1];
        let size: u64 = caps[2].parse().unwrap_or(0);
        let mut name = caps[3].to_string();
        let is_dir = perms.starts_with('d');
        let is_symlink = perms.starts_with('l');
        if is_symlink {
            if let Some((target, _)) = name.split_once(" -> ") {
                name = target.to_string();
            }
        }
        if name == "." || name == ".." {
            continue;
        }
        entries.push(SftpEntry {
            name,
            is_dir,
            is_symlink,
            size,
        });
    }
    sort_entries(&mut entries);
    entries
}

fn scp_list(sess: &Session, path: &str) -> Result<(String, Vec<SftpEntry>), String> {
    let target = if path.is_empty() { "." } else { path };
    let (out, code) = exec_capture(sess, &format!("ls -la {}", shell_escape(target)))?;
    if code != 0 && out.trim().is_empty() {
        return Err(format!("ls exited with code {code}"));
    }
    Ok((target.to_string(), parse_ls_output(&out)))
}

fn scp_upload_one(sess: &Session, local_path: &Path, remote_path: &str) -> Result<(), String> {
    let metadata = std::fs::metadata(local_path).map_err(|e| e.to_string())?;
    let mut local_file = std::fs::File::open(local_path).map_err(|e| e.to_string())?;
    let mut channel = sess
        .scp_send(Path::new(remote_path), 0o644, metadata.len(), None)
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 65536];
    loop {
        let n = local_file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        channel.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    channel.send_eof().map_err(|e| e.to_string())?;
    channel.wait_eof().map_err(|e| e.to_string())?;
    channel.close().map_err(|e| e.to_string())?;
    channel.wait_close().map_err(|e| e.to_string())?;
    Ok(())
}

fn scp_download_one(sess: &Session, remote_path: &str, local_path: &Path) -> Result<(), String> {
    let (mut channel, stat) = sess.scp_recv(Path::new(remote_path)).map_err(|e| e.to_string())?;
    let mut local_file = std::fs::File::create(local_path).map_err(|e| e.to_string())?;
    let mut remaining = stat.size();
    let mut buf = [0u8; 65536];
    while remaining > 0 {
        let to_read = (buf.len() as u64).min(remaining) as usize;
        let n = channel.read(&mut buf[..to_read]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        local_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        remaining -= n as u64;
    }
    let _ = channel.wait_close();
    Ok(())
}

impl SftpBackend {
    pub fn connect(config: SshConfig, protocol: Protocol, ctx: egui::Context) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SftpCommand>();
        let (event_tx, event_rx) = mpsc::channel::<SftpEvent>();

        thread::spawn(move || {
            let sess = match connect_session(&config) {
                Ok(s) => s,
                Err(msg) => {
                    let _ = event_tx.send(SftpEvent::ConnectError(msg));
                    ctx.request_repaint();
                    return;
                }
            };
            let sftp = if protocol == Protocol::Sftp {
                match sess.sftp() {
                    Ok(s) => Some(s),
                    Err(e) => {
                        let _ = event_tx.send(SftpEvent::ConnectError(e.to_string()));
                        ctx.request_repaint();
                        return;
                    }
                }
            } else {
                None
            };

            for cmd in cmd_rx {
                match cmd {
                    SftpCommand::List(path) => {
                        let result = match &sftp {
                            Some(sftp) => sftp_list(sftp, &path),
                            None => scp_list(&sess, &path),
                        };
                        match result {
                            Ok((path, entries)) => {
                                let _ = event_tx.send(SftpEvent::Listed { path, entries });
                            }
                            Err(msg) => {
                                let _ = event_tx.send(SftpEvent::Error(msg));
                            }
                        }
                    }
                    SftpCommand::Mkdir(path) => {
                        let result = match &sftp {
                            Some(sftp) => sftp.mkdir(Path::new(&path), 0o755).map_err(|e| e.to_string()),
                            None => exec_simple(&sess, &format!("mkdir {}", shell_escape(&path))),
                        };
                        if let Err(msg) = result {
                            let _ = event_tx.send(SftpEvent::Error(msg));
                        }
                    }
                    SftpCommand::Delete(path, is_dir) => {
                        let result = match &sftp {
                            Some(sftp) => {
                                if is_dir {
                                    sftp.rmdir(Path::new(&path))
                                } else {
                                    sftp.unlink(Path::new(&path))
                                }
                                .map_err(|e| e.to_string())
                            }
                            None => {
                                let cmd = if is_dir {
                                    format!("rmdir {}", shell_escape(&path))
                                } else {
                                    format!("rm -f {}", shell_escape(&path))
                                };
                                exec_simple(&sess, &cmd)
                            }
                        };
                        if let Err(msg) = result {
                            let _ = event_tx.send(SftpEvent::Error(msg));
                        }
                    }
                    SftpCommand::Upload(local_paths, remote_dir) => {
                        for local_path in local_paths {
                            let file_name = local_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let remote_path = join_remote(&remote_dir, &file_name);
                            let result = match &sftp {
                                Some(sftp) => (|| -> Result<(), String> {
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
                                })(),
                                None => scp_upload_one(&sess, &local_path, &remote_path),
                            };
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
                            let result = match &sftp {
                                Some(sftp) => (|| -> Result<(), String> {
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
                                })(),
                                None => scp_download_one(&sess, &remote_path, &local_path),
                            };
                            let _ = event_tx.send(SftpEvent::Progress {
                                file: file_name,
                                done: result.is_ok(),
                                error: result.err(),
                            });
                        }
                    }
                }
                ctx.request_repaint();
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
