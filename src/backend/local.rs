use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

pub enum LocalEvent {
    Data(Vec<u8>),
    Exited,
}

/// A real local shell session, backed by an OS PTY (same thing a native
/// terminal emulator spawns) — mirrors what `localTerminal.js` did with
/// `node-pty`.
pub struct LocalTerminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    pub events: Receiver<LocalEvent>,
    cols: u16,
    rows: u16,
}

impl LocalTerminal {
    pub fn spawn(cols: usize, rows: usize) -> anyhow::Result<Self> {
        let pty_system = native_pty_system();
        let size = PtySize {
            rows: rows.max(1) as u16,
            cols: cols.max(1) as u16,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system.openpty(size)?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = CommandBuilder::new(shell);
        if let Some(home) = dirs_home() {
            cmd.cwd(home);
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx): (Sender<LocalEvent>, Receiver<LocalEvent>) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(LocalEvent::Exited);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(LocalEvent::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(LocalEvent::Exited);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            child,
            events: rx,
            cols: size.cols,
            rows: size.rows,
        })
    }

    pub fn write(&mut self, data: &[u8]) {
        let _ = self.writer.write_all(data);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1) as u16;
        let rows = rows.max(1) as u16;
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}
