const { Client } = require('ssh2');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

// connId -> { client, shellStream, sftp, protocol }
const connections = new Map();

function requireConn(connId) {
  const conn = connections.get(connId);
  if (!conn) throw new Error('Connection not found');
  return conn;
}

function shellEscape(str) {
  return "'" + String(str).replace(/'/g, "'\\''") + "'";
}

function buildAuthOptions(config) {
  const opts = {
    host: config.host,
    port: Number(config.port) || 22,
    username: config.username,
    readyTimeout: 15000,
    keepaliveInterval: 15000
  };

  if (config.authType === 'key') {
    if (!config.keyPath) throw new Error('Private key path is required');
    opts.privateKey = fs.readFileSync(config.keyPath);
    if (config.passphrase) opts.passphrase = config.passphrase;
  } else {
    opts.password = config.password || '';
    opts.tryKeyboard = true;
  }

  return opts;
}

function connect(config, emit) {
  return new Promise((resolve, reject) => {
    const client = new Client();
    const connId = crypto.randomUUID();

    client.on('ready', () => {
      client.shell(
        { term: 'xterm-256color', cols: config.cols || 80, rows: config.rows || 24 },
        (err, stream) => {
          if (err) {
            client.end();
            reject(err);
            return;
          }

          connections.set(connId, {
            client,
            shellStream: stream,
            sftp: null,
            protocol: config.transferProtocol || 'sftp'
          });

          stream.on('data', (data) => {
            emit('ssh:data', { connId, data: data.toString('utf-8') });
          });
          stream.stderr.on('data', (data) => {
            emit('ssh:data', { connId, data: data.toString('utf-8') });
          });
          stream.on('close', () => {
            emit('ssh:closed', { connId });
            cleanup(connId);
          });

          resolve({ connId });
        }
      );
    });

    client.on('keyboard-interactive', (_name, _instr, _lang, prompts, finish) => {
      finish(prompts.map(() => config.password || ''));
    });

    client.on('error', (err) => {
      emit('ssh:error', { connId, message: err.message });
      reject(err);
    });

    try {
      client.connect(buildAuthOptions(config));
    } catch (e) {
      reject(e);
    }
  });
}

function cleanup(connId) {
  const conn = connections.get(connId);
  if (!conn) return;
  try {
    if (conn.sftp) conn.sftp.end();
  } catch {}
  try {
    conn.client.end();
  } catch {}
  connections.delete(connId);
}

function write(connId, data) {
  const conn = connections.get(connId);
  if (!conn) return false;
  conn.shellStream.write(data);
  return true;
}

function resize(connId, cols, rows) {
  const conn = connections.get(connId);
  if (!conn) return false;
  conn.shellStream.setWindow(rows, cols, 0, 0);
  return true;
}

function disconnect(connId) {
  cleanup(connId);
  return true;
}

function disconnectAll() {
  for (const connId of Array.from(connections.keys())) cleanup(connId);
}

// ============================================================
// SFTP (real SFTP subsystem) — used when protocol === 'sftp'
// ============================================================

function getSftp(connId) {
  return new Promise((resolve, reject) => {
    const conn = requireConn(connId);
    if (conn.sftp) return resolve(conn.sftp);
    conn.client.sftp((err, sftp) => {
      if (err) return reject(err);
      conn.sftp = sftp;
      resolve(sftp);
    });
  });
}

function modeToType(mode) {
  if ((mode & 0o170000) === 0o040000) return 'dir';
  if ((mode & 0o170000) === 0o120000) return 'symlink';
  return 'file';
}

function sortEntries(entries) {
  return entries.sort((a, b) => {
    if (a.type === 'dir' && b.type !== 'dir') return -1;
    if (a.type !== 'dir' && b.type === 'dir') return 1;
    return a.name.localeCompare(b.name);
  });
}

async function sftpListRaw(connId, remotePath) {
  const sftp = await getSftp(connId);
  const targetPath = remotePath || '.';
  return new Promise((resolve, reject) => {
    sftp.readdir(targetPath, (err, list) => {
      if (err) return reject(err);
      const entries = sortEntries(
        list.map((item) => ({
          name: item.filename,
          type: modeToType(item.attrs.mode),
          size: item.attrs.size,
          mtime: item.attrs.mtime * 1000
        }))
      );
      resolve({ path: targetPath, entries });
    });
  });
}

async function sftpMkdirRaw(connId, remotePath) {
  const sftp = await getSftp(connId);
  return new Promise((resolve, reject) => {
    sftp.mkdir(remotePath, (err) => (err ? reject(err) : resolve(true)));
  });
}

async function sftpDeleteRaw(connId, remotePath, isDir) {
  const sftp = await getSftp(connId);
  return new Promise((resolve, reject) => {
    const cb = (err) => (err ? reject(err) : resolve(true));
    if (isDir) sftp.rmdir(remotePath, cb);
    else sftp.unlink(remotePath, cb);
  });
}

async function sftpUploadRaw(connId, localPaths, remoteDir, emit) {
  const sftp = await getSftp(connId);
  for (const localPath of localPaths) {
    const fileName = path.basename(localPath);
    const remotePath = remoteDir.endsWith('/') ? remoteDir + fileName : remoteDir + '/' + fileName;
    await new Promise((resolve, reject) => {
      sftp.fastPut(localPath, remotePath, (err) => {
        emit('sftp:progress', { connId, file: fileName, done: !err, error: err ? err.message : null });
        err ? reject(err) : resolve();
      });
    });
  }
  return true;
}

async function sftpDownloadRaw(connId, remotePaths, localDir, emit) {
  const sftp = await getSftp(connId);
  for (const remotePath of remotePaths) {
    const fileName = remotePath.split('/').pop();
    const localPath = path.join(localDir, fileName);
    await new Promise((resolve, reject) => {
      sftp.fastGet(remotePath, localPath, (err) => {
        emit('sftp:progress', { connId, file: fileName, done: !err, error: err ? err.message : null });
        err ? reject(err) : resolve();
      });
    });
  }
  return true;
}

// ============================================================
// Plain-exec fallback (ls/mkdir/rm) — used for SCP directory
// browsing, since scp itself has no "list" operation.
// ============================================================

function execSimple(conn, cmd) {
  return new Promise((resolve, reject) => {
    conn.client.exec(cmd, (err, stream) => {
      if (err) return reject(err);
      let errOut = '';
      stream.stderr.on('data', (d) => (errOut += d.toString('utf-8')));
      stream.on('close', (code) => {
        if (code === 0) resolve(true);
        else reject(new Error(errOut.trim() || `Command failed with exit code ${code}`));
      });
    });
  });
}

function parseLsOutput(out) {
  const lines = out.split('\n').map((l) => l.replace(/\r$/, '')).filter(Boolean);
  // Matches both GNU and BusyBox `ls -la` long-listing lines.
  const re = /^([bcdlsp-][-rwxsStT]{9})\s+\d+\s+\S+\s+\S+\s+(\d+)\s+(\w+\s+\d+\s+[\d:]+)\s+(.+)$/;
  const entries = [];
  for (const line of lines) {
    if (/^total\s+\d+/.test(line)) continue;
    const m = line.match(re);
    if (!m) continue;
    const perms = m[1];
    const size = parseInt(m[2], 10);
    let name = m[4];
    let type = 'file';
    if (perms[0] === 'd') type = 'dir';
    else if (perms[0] === 'l') {
      type = 'symlink';
      if (name.includes(' -> ')) name = name.split(' -> ')[0];
    }
    if (name === '.' || name === '..') continue;
    entries.push({ name, type, size, mtime: null });
  }
  return sortEntries(entries);
}

async function execList(connId, remotePath) {
  const conn = requireConn(connId);
  const targetPath = remotePath || '.';
  return new Promise((resolve, reject) => {
    conn.client.exec(`ls -la ${shellEscape(targetPath)}`, (err, stream) => {
      if (err) return reject(err);
      let out = '';
      let errOut = '';
      stream.on('data', (d) => (out += d.toString('utf-8')));
      stream.stderr.on('data', (d) => (errOut += d.toString('utf-8')));
      stream.on('close', (code) => {
        if (code !== 0 && !out.trim()) {
          return reject(new Error(errOut.trim() || `ls exited with code ${code}`));
        }
        resolve({ path: targetPath, entries: parseLsOutput(out) });
      });
    });
  });
}

// ============================================================
// SCP (raw protocol over an exec channel) — used when the
// remote SSH server has no SFTP subsystem but has /bin/scp.
// ============================================================

class ByteStreamReader {
  constructor(stream) {
    this.buffer = Buffer.alloc(0);
    this.pending = null;
    stream.on('data', (chunk) => this._onData(chunk));
  }

  _onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    this._tryResolve();
  }

  _tryResolve() {
    if (!this.pending) return;
    if (this.pending.type === 'bytes') {
      if (this.buffer.length >= this.pending.need) {
        const out = this.buffer.slice(0, this.pending.need);
        this.buffer = this.buffer.slice(this.pending.need);
        const { resolve } = this.pending;
        this.pending = null;
        resolve(out);
      }
    } else if (this.pending.type === 'line') {
      const idx = this.buffer.indexOf(0x0a);
      if (idx !== -1) {
        const out = this.buffer.slice(0, idx);
        this.buffer = this.buffer.slice(idx + 1);
        const { resolve } = this.pending;
        this.pending = null;
        resolve(out.toString('utf-8'));
      }
    }
  }

  readBytes(n) {
    return new Promise((resolve) => {
      this.pending = { type: 'bytes', need: n, resolve };
      this._tryResolve();
    });
  }

  readLine() {
    return new Promise((resolve) => {
      this.pending = { type: 'line', resolve };
      this._tryResolve();
    });
  }

  async readByte() {
    const b = await this.readBytes(1);
    return b[0];
  }
}

function writeAsync(stream, buf) {
  return new Promise((resolve, reject) => {
    stream.write(buf, (err) => (err ? reject(err) : resolve()));
  });
}

async function scpUploadOne(connId, localPath, remoteDir, emit) {
  const conn = requireConn(connId);
  const fileName = path.basename(localPath);
  const stat = fs.statSync(localPath);

  return new Promise((resolve, reject) => {
    conn.client.exec(`scp -t ${shellEscape(remoteDir)}`, (err, stream) => {
      if (err) return reject(err);
      const reader = new ByteStreamReader(stream);
      let settled = false;
      const done = (e) => {
        if (settled) return;
        settled = true;
        e ? reject(e) : resolve(true);
      };
      stream.on('close', () => done(new Error('Remote scp process closed unexpectedly')));

      (async () => {
        try {
          await writeAsync(stream, Buffer.from(`C0644 ${stat.size} ${fileName}\n`));
          let ack = await reader.readByte();
          if (ack !== 0) throw new Error((await reader.readLine().catch(() => '')) || 'Remote rejected file header');

          const fd = fs.openSync(localPath, 'r');
          const buf = Buffer.alloc(65536);
          let sent = 0;
          while (sent < stat.size) {
            const n = fs.readSync(fd, buf, 0, Math.min(buf.length, stat.size - sent), sent);
            await writeAsync(stream, buf.slice(0, n));
            sent += n;
            emit && emit('sftp:progress', { connId, file: fileName, bytesTransferred: sent, totalBytes: stat.size });
          }
          fs.closeSync(fd);

          await writeAsync(stream, Buffer.from([0]));
          ack = await reader.readByte();
          if (ack !== 0) throw new Error((await reader.readLine().catch(() => '')) || 'Remote rejected file data');

          stream.end();
          done(null);
        } catch (e) {
          done(e);
        }
      })();
    });
  });
}

async function scpDownloadOne(connId, remotePath, localDir, emit) {
  const conn = requireConn(connId);

  return new Promise((resolve, reject) => {
    conn.client.exec(`scp -f ${shellEscape(remotePath)}`, (err, stream) => {
      if (err) return reject(err);
      const reader = new ByteStreamReader(stream);
      let settled = false;
      const done = (e) => {
        if (settled) return;
        settled = true;
        e ? reject(e) : resolve(true);
      };
      stream.on('close', () => done(new Error('Remote scp process closed unexpectedly')));

      (async () => {
        try {
          await writeAsync(stream, Buffer.from([0])); // ready to receive
          const line = await reader.readLine();
          const m = line.match(/^C\d+\s+(\d+)\s+(.+)$/);
          if (!m) throw new Error('Unexpected SCP response: ' + line);
          const size = parseInt(m[1], 10);
          const fileName = m[2];

          await writeAsync(stream, Buffer.from([0])); // accept header

          const localPath = path.join(localDir, fileName);
          const fd = fs.openSync(localPath, 'w');
          let received = 0;
          const CHUNK = 65536;
          while (received < size) {
            const toRead = Math.min(CHUNK, size - received);
            const data = await reader.readBytes(toRead);
            fs.writeSync(fd, data, 0, data.length, received);
            received += data.length;
            emit && emit('sftp:progress', { connId, file: fileName, bytesTransferred: received, totalBytes: size });
          }
          fs.closeSync(fd);

          await reader.readBytes(1); // trailing null terminator
          await writeAsync(stream, Buffer.from([0])); // final ack

          stream.end();
          done(null);
        } catch (e) {
          done(e);
        }
      })();
    });
  });
}

// ============================================================
// Protocol-aware dispatchers — these are what main.js calls.
// They pick real SFTP, exec+scp, or reject (TFTP has its own
// dedicated IPC path since it doesn't use the SSH connection).
// ============================================================

async function sftpList(connId, remotePath) {
  const conn = requireConn(connId);
  if (conn.protocol === 'scp') return execList(connId, remotePath);
  if (conn.protocol === 'tftp') {
    throw new Error('Directory browsing is not available over TFTP — use the TFTP box to send/fetch a file by exact name.');
  }
  return sftpListRaw(connId, remotePath);
}

async function sftpMkdir(connId, remotePath) {
  const conn = requireConn(connId);
  if (conn.protocol === 'scp') return execSimple(conn, `mkdir ${shellEscape(remotePath)}`);
  if (conn.protocol === 'tftp') throw new Error('Not supported over TFTP.');
  return sftpMkdirRaw(connId, remotePath);
}

async function sftpDelete(connId, remotePath, isDir) {
  const conn = requireConn(connId);
  if (conn.protocol === 'scp') {
    return execSimple(conn, isDir ? `rmdir ${shellEscape(remotePath)}` : `rm -f ${shellEscape(remotePath)}`);
  }
  if (conn.protocol === 'tftp') throw new Error('Not supported over TFTP.');
  return sftpDeleteRaw(connId, remotePath, isDir);
}

async function sftpUpload(connId, localPaths, remoteDir, emit) {
  const conn = requireConn(connId);
  if (conn.protocol === 'tftp') throw new Error('Use the TFTP box for TFTP transfers.');
  if (conn.protocol === 'scp') {
    for (const localPath of localPaths) await scpUploadOne(connId, localPath, remoteDir, emit);
    return true;
  }
  return sftpUploadRaw(connId, localPaths, remoteDir, emit);
}

async function sftpDownload(connId, remotePaths, localDir, emit) {
  const conn = requireConn(connId);
  if (conn.protocol === 'tftp') throw new Error('Use the TFTP box for TFTP transfers.');
  if (conn.protocol === 'scp') {
    for (const remotePath of remotePaths) await scpDownloadOne(connId, remotePath, localDir, emit);
    return true;
  }
  return sftpDownloadRaw(connId, remotePaths, localDir, emit);
}

module.exports = {
  connect,
  write,
  resize,
  disconnect,
  disconnectAll,
  sftpList,
  sftpMkdir,
  sftpDelete,
  sftpUpload,
  sftpDownload
};
