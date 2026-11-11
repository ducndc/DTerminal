use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

const OP_RRQ: u16 = 1;
const OP_WRQ: u16 = 2;
const OP_DATA: u16 = 3;
const OP_ACK: u16 = 4;
const OP_ERROR: u16 = 5;
const BLOCK_SIZE: usize = 512;
const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RETRIES: u32 = 5;

fn build_request(opcode: u16, filename: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(filename.len() + 10);
    buf.extend_from_slice(&opcode.to_be_bytes());
    buf.extend_from_slice(filename.as_bytes());
    buf.push(0);
    buf.extend_from_slice(b"octet\0");
    buf
}

fn build_ack(block: u16) -> [u8; 4] {
    let mut buf = [0u8; 4];
    buf[0..2].copy_from_slice(&OP_ACK.to_be_bytes());
    buf[2..4].copy_from_slice(&block.to_be_bytes());
    buf
}

fn build_data(block: u16, chunk: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + chunk.len());
    buf.extend_from_slice(&OP_DATA.to_be_bytes());
    buf.extend_from_slice(&block.to_be_bytes());
    buf.extend_from_slice(chunk);
    buf
}

fn parse_error(buf: &[u8]) -> String {
    let code = u16::from_be_bytes([buf[2], buf[3]]);
    let msg = String::from_utf8_lossy(&buf[4..buf.len().saturating_sub(1)]);
    format!("TFTP error {code}: {msg}")
}

/// Downloads a file via TFTP RRQ into `local_dir`, returning the saved path
/// and byte count — ported from `tftpClient.js`'s `download()`.
pub fn download(host: &str, port: u16, remote_file_name: &str, local_dir: &Path) -> Result<(PathBuf, u64), String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(TIMEOUT)).ok();
    let local_path = local_dir.join(
        Path::new(remote_file_name)
            .file_name()
            .unwrap_or_else(|| remote_file_name.as_ref()),
    );
    let mut file = File::create(&local_path).map_err(|e| e.to_string())?;

    let target: SocketAddr = format!("{host}:{port}").parse().map_err(|_| "invalid host/port".to_string())?;
    let request = build_request(OP_RRQ, remote_file_name);
    socket.send_to(&request, target).map_err(|e| e.to_string())?;

    let mut remote_addr: Option<SocketAddr> = None;
    let mut expected_block: u16 = 1;
    let mut bytes_written: u64 = 0;
    let mut retries = 0;
    let mut buf = [0u8; BLOCK_SIZE + 4];

    loop {
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                if remote_addr.is_none() {
                    remote_addr = Some(from);
                }
                let addr = remote_addr.unwrap();
                if n < 4 {
                    continue;
                }
                let opcode = u16::from_be_bytes([buf[0], buf[1]]);
                if opcode == OP_ERROR {
                    let _ = std::fs::remove_file(&local_path);
                    return Err(parse_error(&buf[..n]));
                }
                if opcode != OP_DATA {
                    continue;
                }
                let block = u16::from_be_bytes([buf[2], buf[3]]);
                if block != expected_block {
                    let ack = build_ack(expected_block.wrapping_sub(1));
                    let _ = socket.send_to(&ack, addr);
                    continue;
                }
                let chunk = &buf[4..n];
                file.write_all(chunk).map_err(|e| e.to_string())?;
                bytes_written += chunk.len() as u64;
                retries = 0;
                let ack = build_ack(block);
                socket.send_to(&ack, addr).map_err(|e| e.to_string())?;

                if chunk.len() < BLOCK_SIZE {
                    return Ok((local_path, bytes_written));
                }
                expected_block = expected_block.wrapping_add(1);
            }
            Err(_) => {
                retries += 1;
                if retries > MAX_RETRIES {
                    let _ = std::fs::remove_file(&local_path);
                    return Err("TFTP transfer timed out".to_string());
                }
                if let Some(addr) = remote_addr {
                    let ack = build_ack(expected_block.wrapping_sub(1));
                    let _ = socket.send_to(&ack, addr);
                } else {
                    let _ = socket.send_to(&request, target);
                }
            }
        }
    }
}

/// Uploads `local_path` via TFTP WRQ, writing it to the server under
/// `remote_file_name` — ported from `tftpClient.js`'s `upload()`.
pub fn upload(host: &str, port: u16, local_path: &Path, remote_file_name: &str) -> Result<u64, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(TIMEOUT)).ok();
    let mut file = File::open(local_path).map_err(|e| e.to_string())?;
    let total_size = file.metadata().map_err(|e| e.to_string())?.len();

    let target: SocketAddr = format!("{host}:{port}").parse().map_err(|_| "invalid host/port".to_string())?;
    let request = build_request(OP_WRQ, remote_file_name);
    socket.send_to(&request, target).map_err(|e| e.to_string())?;

    let mut remote_addr: Option<SocketAddr> = None;
    let mut retries = 0;
    let mut awaiting_initial_ack = true;
    let mut last_sent_block: u16 = 0;
    let mut last_sent: Vec<u8> = Vec::new();
    let mut read_buf = [0u8; BLOCK_SIZE];
    let mut buf = [0u8; 4];

    let mut send_next_block = |socket: &UdpSocket,
                                addr: SocketAddr,
                                file: &mut File,
                                last_sent_block: &mut u16,
                                last_sent: &mut Vec<u8>|
     -> Result<(), String> {
        let n = file.read(&mut read_buf).map_err(|e| e.to_string())?;
        let block_num = last_sent_block.wrapping_add(1);
        let packet = build_data(block_num, &read_buf[..n]);
        *last_sent = packet.clone();
        *last_sent_block = block_num;
        socket.send_to(&packet, addr).map_err(|e| e.to_string())?;
        Ok(())
    };

    loop {
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                if remote_addr.is_none() {
                    remote_addr = Some(from);
                }
                let addr = remote_addr.unwrap();
                if n < 4 {
                    continue;
                }
                let opcode = u16::from_be_bytes([buf[0], buf[1]]);
                if opcode == OP_ERROR {
                    return Err("TFTP error during upload".to_string());
                }
                if opcode != OP_ACK {
                    continue;
                }
                let acked_block = u16::from_be_bytes([buf[2], buf[3]]);

                if awaiting_initial_ack {
                    if acked_block != 0 {
                        continue;
                    }
                    awaiting_initial_ack = false;
                    retries = 0;
                    send_next_block(&socket, addr, &mut file, &mut last_sent_block, &mut last_sent)?;
                    continue;
                }

                if acked_block != last_sent_block {
                    continue;
                }
                retries = 0;
                let last_was_short = last_sent.len() - 4 < BLOCK_SIZE;
                if last_was_short {
                    return Ok(total_size);
                }
                send_next_block(&socket, addr, &mut file, &mut last_sent_block, &mut last_sent)?;
            }
            Err(_) => {
                retries += 1;
                if retries > MAX_RETRIES {
                    return Err("TFTP transfer timed out".to_string());
                }
                if awaiting_initial_ack {
                    let _ = socket.send_to(&request, target);
                } else if let Some(addr) = remote_addr {
                    let _ = socket.send_to(&last_sent, addr);
                }
            }
        }
    }
}
