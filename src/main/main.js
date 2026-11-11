const { app, BrowserWindow, ipcMain, dialog } = require('electron');
const path = require('path');
const sessionStore = require('./sessionStore');
const sshManager = require('./sshManager');
const tftpClient = require('./tftpClient');
const localTerminal = require('./localTerminal');

let mainWindow;

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1200,
    height: 760,
    backgroundColor: '#1e1e1e',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true
    }
  });

  mainWindow.loadFile(path.join(__dirname, '..', 'renderer', 'index.html'));
  mainWindow.removeMenu();

  mainWindow.webContents.on('console-message', (_e, level, message, line, sourceId) => {
    if (level >= 2) console.log(`[renderer] ${message} (${sourceId}:${line})`);
  });

  mainWindow.on('closed', () => {
    sshManager.disconnectAll();
    localTerminal.killAll();
    mainWindow = null;
  });
}

app.whenReady().then(() => {
  createWindow();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  sshManager.disconnectAll();
  localTerminal.killAll();
  if (process.platform !== 'darwin') app.quit();
});

// ---------- Session store IPC ----------
ipcMain.handle('sessions:list', () => sessionStore.listSessions());
ipcMain.handle('sessions:save', (_e, session) => sessionStore.saveSession(session));
ipcMain.handle('sessions:delete', (_e, id) => sessionStore.deleteSession(id));
ipcMain.handle('appearance:get', () => sessionStore.getAppearance());
ipcMain.handle('appearance:save', (_e, appearance) => sessionStore.saveAppearance(appearance));

ipcMain.handle('quickCommands:list', () => sessionStore.listQuickCommands());
ipcMain.handle('quickCommands:save', (_e, cmd) => sessionStore.saveQuickCommand(cmd));
ipcMain.handle('quickCommands:delete', (_e, id) => sessionStore.deleteQuickCommand(id));

// ---------- Dialog IPC ----------
ipcMain.handle('dialog:pickKeyFile', async () => {
  const result = await dialog.showOpenDialog(mainWindow, {
    title: 'Select private key file',
    properties: ['openFile']
  });
  if (result.canceled || result.filePaths.length === 0) return null;
  return result.filePaths[0];
});

ipcMain.handle('dialog:pickUploadFiles', async () => {
  const result = await dialog.showOpenDialog(mainWindow, {
    title: 'Select files to upload',
    properties: ['openFile', 'multiSelections']
  });
  if (result.canceled) return [];
  return result.filePaths;
});

ipcMain.handle('dialog:pickDownloadDir', async () => {
  const result = await dialog.showOpenDialog(mainWindow, {
    title: 'Select download destination',
    properties: ['openDirectory', 'createDirectory']
  });
  if (result.canceled || result.filePaths.length === 0) return null;
  return result.filePaths[0];
});

// ---------- SSH IPC ----------
ipcMain.handle('ssh:connect', async (event, config) => {
  let effective = config;
  if (config.sessionId) {
    const stored = sessionStore.getSessionWithSecrets(config.sessionId);
    if (!stored) throw new Error('Saved session not found');
    effective = { ...stored, cols: config.cols, rows: config.rows };
  }
  return sshManager.connect(effective, (channel, payload) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send(channel, payload);
    }
  });
});

ipcMain.handle('ssh:write', (_e, { connId, data }) => sshManager.write(connId, data));
ipcMain.handle('ssh:resize', (_e, { connId, cols, rows }) => sshManager.resize(connId, cols, rows));
ipcMain.handle('ssh:disconnect', (_e, connId) => sshManager.disconnect(connId));

// ---------- SFTP IPC ----------
ipcMain.handle('sftp:list', (_e, { connId, remotePath }) => sshManager.sftpList(connId, remotePath));
ipcMain.handle('sftp:mkdir', (_e, { connId, remotePath }) => sshManager.sftpMkdir(connId, remotePath));
ipcMain.handle('sftp:delete', (_e, { connId, remotePath, isDir }) => sshManager.sftpDelete(connId, remotePath, isDir));
ipcMain.handle('sftp:upload', (_e, { connId, localPaths, remoteDir }) =>
  sshManager.sftpUpload(connId, localPaths, remoteDir, (channel, payload) => {
    if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send(channel, payload);
  })
);
ipcMain.handle('sftp:download', (_e, { connId, remotePaths, localDir }) =>
  sshManager.sftpDownload(connId, remotePaths, localDir, (channel, payload) => {
    if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send(channel, payload);
  })
);

// ---------- Local terminal IPC (real local shell, like a native terminal) ----------
ipcMain.handle('local:spawn', (_e, opts) =>
  localTerminal.spawn(opts || {}, (channel, payload) => {
    if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send(channel, payload);
  })
);
ipcMain.handle('local:write', (_e, { id, data }) => localTerminal.write(id, data));
ipcMain.handle('local:resize', (_e, { id, cols, rows }) => localTerminal.resize(id, cols, rows));
ipcMain.handle('local:kill', (_e, id) => localTerminal.kill(id));

// ---------- TFTP IPC (plain UDP, independent of the SSH connection) ----------
ipcMain.handle('tftp:upload', (_e, { host, port, localPath, remoteFileName }) =>
  tftpClient.upload(host, port || 69, localPath, remoteFileName)
);
ipcMain.handle('tftp:download', (_e, { host, port, remoteFileName, localDir }) =>
  tftpClient.download(host, port || 69, remoteFileName, localDir)
);
