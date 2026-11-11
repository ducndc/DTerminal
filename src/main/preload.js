const { contextBridge, ipcRenderer, webUtils } = require('electron');

contextBridge.exposeInMainWorld('api', {
  getPathForFile: (file) => webUtils.getPathForFile(file),
  sessions: {
    list: () => ipcRenderer.invoke('sessions:list'),
    save: (session) => ipcRenderer.invoke('sessions:save', session),
    delete: (id) => ipcRenderer.invoke('sessions:delete', id)
  },
  appearance: {
    get: () => ipcRenderer.invoke('appearance:get'),
    save: (appearance) => ipcRenderer.invoke('appearance:save', appearance)
  },
  quickCommands: {
    list: () => ipcRenderer.invoke('quickCommands:list'),
    save: (cmd) => ipcRenderer.invoke('quickCommands:save', cmd),
    delete: (id) => ipcRenderer.invoke('quickCommands:delete', id)
  },
  dialog: {
    pickKeyFile: () => ipcRenderer.invoke('dialog:pickKeyFile'),
    pickUploadFiles: () => ipcRenderer.invoke('dialog:pickUploadFiles'),
    pickDownloadDir: () => ipcRenderer.invoke('dialog:pickDownloadDir')
  },
  local: {
    spawn: (opts) => ipcRenderer.invoke('local:spawn', opts),
    write: (id, data) => ipcRenderer.invoke('local:write', { id, data }),
    resize: (id, cols, rows) => ipcRenderer.invoke('local:resize', { id, cols, rows }),
    kill: (id) => ipcRenderer.invoke('local:kill', id),
    onData: (cb) => ipcRenderer.on('local:data', (_e, payload) => cb(payload)),
    onExit: (cb) => ipcRenderer.on('local:exit', (_e, payload) => cb(payload))
  },
  ssh: {
    connect: (config) => ipcRenderer.invoke('ssh:connect', config),
    write: (connId, data) => ipcRenderer.invoke('ssh:write', { connId, data }),
    resize: (connId, cols, rows) => ipcRenderer.invoke('ssh:resize', { connId, cols, rows }),
    disconnect: (connId) => ipcRenderer.invoke('ssh:disconnect', connId),
    onData: (cb) => ipcRenderer.on('ssh:data', (_e, payload) => cb(payload)),
    onClosed: (cb) => ipcRenderer.on('ssh:closed', (_e, payload) => cb(payload)),
    onError: (cb) => ipcRenderer.on('ssh:error', (_e, payload) => cb(payload))
  },
  sftp: {
    list: (connId, remotePath) => ipcRenderer.invoke('sftp:list', { connId, remotePath }),
    mkdir: (connId, remotePath) => ipcRenderer.invoke('sftp:mkdir', { connId, remotePath }),
    delete: (connId, remotePath, isDir) => ipcRenderer.invoke('sftp:delete', { connId, remotePath, isDir }),
    upload: (connId, localPaths, remoteDir) => ipcRenderer.invoke('sftp:upload', { connId, localPaths, remoteDir }),
    download: (connId, remotePaths, localDir) => ipcRenderer.invoke('sftp:download', { connId, remotePaths, localDir }),
    onProgress: (cb) => ipcRenderer.on('sftp:progress', (_e, payload) => cb(payload))
  },
  tftp: {
    upload: (host, port, localPath, remoteFileName) =>
      ipcRenderer.invoke('tftp:upload', { host, port, localPath, remoteFileName }),
    download: (host, port, remoteFileName, localDir) =>
      ipcRenderer.invoke('tftp:download', { host, port, remoteFileName, localDir })
  }
});
