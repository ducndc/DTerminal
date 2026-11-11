const { app, safeStorage } = require('electron');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const STORE_PATH = path.join(app.getPath('userData'), 'dterminal-store.json');

const DEFAULT_APPEARANCE = {
  theme: 'dark',
  fontFamily: "'Cascadia Code', 'DejaVu Sans Mono', monospace",
  fontSize: 14,
  cursorStyle: 'block'
};

function readStore() {
  try {
    const raw = fs.readFileSync(STORE_PATH, 'utf-8');
    const data = JSON.parse(raw);
    if (!data.sessions) data.sessions = [];
    if (!data.appearance) data.appearance = { ...DEFAULT_APPEARANCE };
    if (!data.quickCommands) data.quickCommands = [];
    return data;
  } catch {
    return { sessions: [], appearance: { ...DEFAULT_APPEARANCE }, quickCommands: [] };
  }
}

function writeStore(data) {
  fs.writeFileSync(STORE_PATH, JSON.stringify(data, null, 2), { mode: 0o600 });
}

function encryptSecret(plain) {
  if (!plain) return '';
  if (safeStorage.isEncryptionAvailable()) {
    return 'enc:' + safeStorage.encryptString(plain).toString('base64');
  }
  // Fallback: mark as plaintext if OS keyring isn't available
  return 'plain:' + plain;
}

function decryptSecret(stored) {
  if (!stored) return '';
  if (stored.startsWith('enc:')) {
    try {
      return safeStorage.decryptString(Buffer.from(stored.slice(4), 'base64'));
    } catch {
      return '';
    }
  }
  if (stored.startsWith('plain:')) return stored.slice(6);
  return '';
}

function listSessions() {
  const { sessions } = readStore();
  // Never send decrypted secrets to the renderer for the list view
  return sessions.map(({ password, passphrase, ...rest }) => rest);
}

function getSessionWithSecrets(id) {
  const { sessions } = readStore();
  const session = sessions.find((s) => s.id === id);
  if (!session) return null;
  return {
    ...session,
    password: decryptSecret(session.password),
    passphrase: decryptSecret(session.passphrase)
  };
}

function saveSession(session) {
  const data = readStore();
  const id = session.id || crypto.randomUUID();
  const toStore = {
    id,
    name: session.name,
    host: session.host,
    port: session.port || 22,
    username: session.username,
    authType: session.authType, // 'password' | 'key'
    keyPath: session.keyPath || '',
    folder: session.folder || '',
    transferProtocol: session.transferProtocol || 'sftp', // 'sftp' | 'scp' | 'tftp'
    tftpPort: Number(session.tftpPort) || 69,
    password: session.password !== undefined ? encryptSecret(session.password) : undefined,
    passphrase: session.passphrase !== undefined ? encryptSecret(session.passphrase) : undefined
  };

  const idx = data.sessions.findIndex((s) => s.id === id);
  if (idx >= 0) {
    // Preserve existing secrets if caller didn't resend them (e.g. editing name only)
    if (toStore.password === undefined) toStore.password = data.sessions[idx].password;
    if (toStore.passphrase === undefined) toStore.passphrase = data.sessions[idx].passphrase;
    data.sessions[idx] = toStore;
  } else {
    if (toStore.password === undefined) toStore.password = '';
    if (toStore.passphrase === undefined) toStore.passphrase = '';
    data.sessions.push(toStore);
  }

  writeStore(data);
  const { password, passphrase, ...rest } = toStore;
  return rest;
}

function deleteSession(id) {
  const data = readStore();
  data.sessions = data.sessions.filter((s) => s.id !== id);
  writeStore(data);
  return true;
}

function getAppearance() {
  return readStore().appearance;
}

function saveAppearance(appearance) {
  const data = readStore();
  data.appearance = { ...DEFAULT_APPEARANCE, ...appearance };
  writeStore(data);
  return data.appearance;
}

function listQuickCommands() {
  return readStore().quickCommands;
}

function saveQuickCommand(cmd) {
  const data = readStore();
  const id = cmd.id || crypto.randomUUID();
  const toStore = { id, name: cmd.name, command: cmd.command, runEnter: cmd.runEnter !== false };
  const idx = data.quickCommands.findIndex((c) => c.id === id);
  if (idx >= 0) data.quickCommands[idx] = toStore;
  else data.quickCommands.push(toStore);
  writeStore(data);
  return toStore;
}

function deleteQuickCommand(id) {
  const data = readStore();
  data.quickCommands = data.quickCommands.filter((c) => c.id !== id);
  writeStore(data);
  return true;
}

module.exports = {
  listSessions,
  getSessionWithSecrets,
  saveSession,
  deleteSession,
  getAppearance,
  saveAppearance,
  listQuickCommands,
  saveQuickCommand,
  deleteQuickCommand
};
