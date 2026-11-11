(() => {
  'use strict';

  // ---------------- State ----------------
  const state = {
    sessions: [],
    appearance: null,
    quickCommands: [],
    tabs: new Map(),      // tabId -> tab object
    connToTab: new Map(), // ssh connId -> tabId
    localToTab: new Map(), // local pty id -> tabId
    activeTabId: null,
    editingSessionId: null,
    editingCommandId: null,
    cmdPanelOpen: false,
    tabSeq: 0
  };

  // ---------------- DOM refs ----------------
  const el = {
    sessionList: document.getElementById('sessionList'),
    tabBar: document.getElementById('tabBar'),
    panes: document.getElementById('panes'),
    newSessionBtn: document.getElementById('newSessionBtn'),
    newLocalBtn: document.getElementById('newLocalBtn'),
    settingsBtn: document.getElementById('settingsBtn'),

    sessionModal: document.getElementById('sessionModal'),
    sessionModalTitle: document.getElementById('sessionModalTitle'),
    f_name: document.getElementById('f_name'),
    f_host: document.getElementById('f_host'),
    f_port: document.getElementById('f_port'),
    f_username: document.getElementById('f_username'),
    f_transferProtocol: document.getElementById('f_transferProtocol'),
    tftpPortRow: document.getElementById('tftpPortRow'),
    f_tftpPort: document.getElementById('f_tftpPort'),
    f_password: document.getElementById('f_password'),
    f_keyPath: document.getElementById('f_keyPath'),
    f_passphrase: document.getElementById('f_passphrase'),
    authPassword: document.getElementById('authPassword'),
    authKey: document.getElementById('authKey'),
    browseKeyBtn: document.getElementById('browseKeyBtn'),
    sessionCancelBtn: document.getElementById('sessionCancelBtn'),
    sessionSaveBtn: document.getElementById('sessionSaveBtn'),
    sessionSaveConnectBtn: document.getElementById('sessionSaveConnectBtn'),

    settingsModal: document.getElementById('settingsModal'),
    s_theme: document.getElementById('s_theme'),
    s_fontFamily: document.getElementById('s_fontFamily'),
    s_fontSize: document.getElementById('s_fontSize'),
    s_cursorStyle: document.getElementById('s_cursorStyle'),
    settingsCancelBtn: document.getElementById('settingsCancelBtn'),
    settingsSaveBtn: document.getElementById('settingsSaveBtn'),

    confirmModal: document.getElementById('confirmModal'),
    confirmMessage: document.getElementById('confirmMessage'),
    confirmCancelBtn: document.getElementById('confirmCancelBtn'),
    confirmOkBtn: document.getElementById('confirmOkBtn'),

    cmdPanelToggle: document.getElementById('cmdPanelToggle'),
    cmdPanel: document.getElementById('cmdPanel'),
    cmdList: document.getElementById('cmdList'),
    cmdAddBtn: document.getElementById('cmdAddBtn'),
    cmdModal: document.getElementById('cmdModal'),
    cmdModalTitle: document.getElementById('cmdModalTitle'),
    c_name: document.getElementById('c_name'),
    c_command: document.getElementById('c_command'),
    c_runEnter: document.getElementById('c_runEnter'),
    cmdCancelBtn: document.getElementById('cmdCancelBtn'),
    cmdSaveBtn: document.getElementById('cmdSaveBtn')
  };

  let currentAuthType = 'password';

  // ---------------- Init ----------------
  async function init() {
    state.sessions = await window.api.sessions.list();
    state.appearance = await window.api.appearance.get();
    state.quickCommands = await window.api.quickCommands.list();
    renderSessionList();
    renderQuickCommandList();

    window.api.ssh.onData(({ connId, data }) => {
      const tab = tabByConn(connId);
      if (tab) tab.term.write(data);
    });
    window.api.ssh.onClosed(({ connId }) => {
      const tab = tabByConn(connId);
      if (tab) markDisconnected(tab);
    });
    window.api.ssh.onError(({ connId, message }) => {
      const tab = tabByConn(connId);
      if (tab) tab.term.writeln(`\r\n\x1b[31m[error] ${message}\x1b[0m`);
    });

    window.api.local.onData(({ id, data }) => {
      const tab = tabByLocal(id);
      if (tab) tab.term.write(data);
    });
    window.api.local.onExit(({ id }) => {
      const tab = tabByLocal(id);
      if (tab) markDisconnected(tab);
    });

    new ResizeObserver(() => fitActiveTab()).observe(el.panes);

    wireGlobalUI();
  }

  function tabByConn(connId) {
    const tabId = state.connToTab.get(connId);
    return tabId ? state.tabs.get(tabId) : null;
  }

  function tabByLocal(id) {
    const tabId = state.localToTab.get(id);
    return tabId ? state.tabs.get(tabId) : null;
  }

  // ---------------- Session list ----------------
  function renderSessionList() {
    el.sessionList.innerHTML = '';
    state.sessions.forEach((s) => {
      const item = document.createElement('div');
      item.className = 'session-item';
      item.innerHTML = `
        <div class="name"><span class="dot"></span>${escapeHtml(s.name)}</div>
        <div class="actions">
          <button data-act="edit" title="Edit">✎</button>
          <button data-act="delete" title="Delete">🗑</button>
        </div>
      `;
      item.title = `${s.username}@${s.host}:${s.port}`;
      item.addEventListener('click', (e) => {
        const act = e.target.closest('button')?.dataset.act;
        if (act === 'edit') return openSessionModal(s);
        if (act === 'delete') return confirmDeleteSession(s);
        createTabForSession(s);
      });
      el.sessionList.appendChild(item);
    });
  }

  function escapeHtml(str) {
    const d = document.createElement('div');
    d.textContent = str ?? '';
    return d.innerHTML;
  }

  function confirmDeleteSession(session) {
    showConfirm(`Delete session "${session.name}"?`, async () => {
      await window.api.sessions.delete(session.id);
      state.sessions = await window.api.sessions.list();
      renderSessionList();
    });
  }

  // ---------------- Session modal ----------------
  function openSessionModal(session) {
    state.editingSessionId = session ? session.id : null;
    el.sessionModalTitle.textContent = session ? 'Edit Session' : 'New Session';
    el.f_name.value = session?.name || '';
    el.f_host.value = session?.host || '';
    el.f_port.value = session?.port || 22;
    el.f_username.value = session?.username || '';
    el.f_transferProtocol.value = session?.transferProtocol || 'sftp';
    el.f_tftpPort.value = session?.tftpPort || 69;
    el.tftpPortRow.classList.toggle('hidden', el.f_transferProtocol.value !== 'tftp');
    el.f_password.value = '';
    el.f_passphrase.value = '';
    el.f_keyPath.value = session?.keyPath || '';
    setAuthType(session?.authType || 'password');
    el.sessionModal.classList.remove('hidden');
    el.f_name.focus();
  }

  function closeSessionModal() {
    el.sessionModal.classList.add('hidden');
  }

  function setAuthType(type) {
    currentAuthType = type;
    document.querySelectorAll('.auth-tab').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.auth === type);
    });
    el.authPassword.classList.toggle('hidden', type !== 'password');
    el.authKey.classList.toggle('hidden', type !== 'key');
  }

  function collectSessionForm() {
    const session = {
      id: state.editingSessionId || undefined,
      name: el.f_name.value.trim() || el.f_host.value.trim(),
      host: el.f_host.value.trim(),
      port: Number(el.f_port.value) || 22,
      username: el.f_username.value.trim(),
      transferProtocol: el.f_transferProtocol.value,
      tftpPort: Number(el.f_tftpPort.value) || 69,
      authType: currentAuthType,
      keyPath: el.f_keyPath.value
    };
    // Only send secrets if user actually typed something, so editing
    // a session without retyping the password keeps the existing one.
    if (el.f_password.value) session.password = el.f_password.value;
    if (el.f_passphrase.value) session.passphrase = el.f_passphrase.value;
    if (!state.editingSessionId) {
      session.password = session.password || '';
      session.passphrase = session.passphrase || '';
    }
    return session;
  }

  async function saveSessionForm(connectAfter) {
    const form = collectSessionForm();
    if (!form.host || !form.username) {
      alert('Host and username are required.');
      return;
    }
    const saved = await window.api.sessions.save(form);
    state.sessions = await window.api.sessions.list();
    renderSessionList();
    closeSessionModal();
    if (connectAfter) createTabForSession(saved);
  }

  // ---------------- Confirm modal ----------------
  function showConfirm(message, onConfirm) {
    el.confirmMessage.textContent = message;
    el.confirmModal.classList.remove('hidden');
    const cleanup = () => {
      el.confirmModal.classList.add('hidden');
      el.confirmOkBtn.removeEventListener('click', okHandler);
      el.confirmCancelBtn.removeEventListener('click', cancelHandler);
    };
    const okHandler = () => { cleanup(); onConfirm(); };
    const cancelHandler = () => cleanup();
    el.confirmOkBtn.addEventListener('click', okHandler);
    el.confirmCancelBtn.addEventListener('click', cancelHandler);
  }

  // ---------------- Settings modal ----------------
  function openSettingsModal() {
    el.s_theme.value = state.appearance.theme;
    el.s_fontFamily.value = state.appearance.fontFamily;
    el.s_fontSize.value = state.appearance.fontSize;
    el.s_cursorStyle.value = state.appearance.cursorStyle;
    el.settingsModal.classList.remove('hidden');
  }

  async function saveSettings() {
    const appearance = {
      theme: el.s_theme.value,
      fontFamily: el.s_fontFamily.value || "monospace",
      fontSize: Number(el.s_fontSize.value) || 14,
      cursorStyle: el.s_cursorStyle.value
    };
    state.appearance = await window.api.appearance.save(appearance);
    applyAppearanceToAllTabs();
    el.settingsModal.classList.add('hidden');
  }

  function applyAppearanceToAllTabs() {
    const theme = window.TERMINAL_THEMES[state.appearance.theme] || window.TERMINAL_THEMES.dark;
    state.tabs.forEach((tab) => {
      tab.term.options.theme = theme;
      tab.term.options.fontFamily = state.appearance.fontFamily;
      tab.term.options.fontSize = state.appearance.fontSize;
      tab.term.options.cursorStyle = state.appearance.cursorStyle;
      tab.fitAddon.fit();
      if (tab.connId) resizeConn(tab);
    });
  }

  // ---------------- Tabs ----------------
  function createTabForSession(session) {
    const id = `tab-${++state.tabSeq}`;
    const tab = {
      id,
      kind: 'ssh',
      session,
      connId: null,
      connected: false,
      term: null,
      fitAddon: null,
      sftp: { open: false, path: '.', entries: [], selected: null }
    };
    state.tabs.set(id, tab);
    buildTabBarButton(tab);
    buildPane(tab);
    setActiveTab(id);
    connectTab(tab);
    return tab;
  }

  function createLocalTab() {
    const id = `tab-${++state.tabSeq}`;
    const tab = {
      id,
      kind: 'local',
      session: { name: 'Local Terminal' },
      connId: null,
      connected: false,
      term: null,
      fitAddon: null,
      sftp: { open: false, path: '.', entries: [], selected: null }
    };
    state.tabs.set(id, tab);
    buildTabBarButton(tab);
    buildPane(tab);
    setActiveTab(id);
    connectTab(tab);
    return tab;
  }

  function buildTabBarButton(tab) {
    const btn = document.createElement('div');
    btn.className = 'tab';
    btn.id = `tabbtn-${tab.id}`;
    btn.innerHTML = `<span class="status-dot"></span><span class="label">${escapeHtml(tab.session.name)}</span><span class="close">✕</span>`;
    btn.addEventListener('click', (e) => {
      if (e.target.classList.contains('close')) {
        closeTab(tab.id);
      } else {
        setActiveTab(tab.id);
      }
    });
    el.tabBar.appendChild(btn);
  }

  function buildPane(tab) {
    if (tab.kind === 'local') return buildLocalPane(tab);

    const protocol = tab.session.transferProtocol || 'sftp';
    const toggleLabel = protocol === 'tftp' ? 'TFTP' : protocol === 'scp' ? 'SCP' : 'SFTP';

    const transferPanelHtml =
      protocol === 'tftp'
        ? `
      <div class="sftp-panel">
        <div class="cmd-panel-header"><span>TFTP Transfer</span></div>
        <div class="tftp-form">
          <label>Local file
            <div class="row">
              <input class="tftp-local" type="text" readonly placeholder="No file selected" />
              <button class="tftp-browse" type="button">Browse…</button>
            </div>
          </label>
          <label>Remote filename<input class="tftp-remote" type="text" placeholder="firmware.bin" /></label>
          <div class="tftp-actions">
            <button class="tftp-upload" type="button">⬆ Upload to device</button>
            <button class="tftp-download" type="button">⬇ Download from device</button>
          </div>
          <div class="tftp-status sftp-status"></div>
        </div>
      </div>
    `
        : `
      <div class="sftp-panel">
        <div class="sftp-toolbar">
          <button data-act="refresh">⟳ Refresh</button>
          <button data-act="upload">⬆ Upload</button>
          <button data-act="download">⬇ Download</button>
          <button data-act="mkdir">+ Folder</button>
          <button data-act="delete">🗑 Delete</button>
        </div>
        <div class="sftp-breadcrumb"></div>
        <div class="sftp-list"></div>
        <div class="sftp-status"></div>
      </div>
    `;

    const pane = document.createElement('div');
    pane.className = 'pane';
    pane.id = `pane-${tab.id}`;
    pane.innerHTML = `
      <div class="term-area">
        <div class="term-toolbar">
          <span>${escapeHtml(tab.session.username)}@${escapeHtml(tab.session.host)}:${tab.session.port}</span>
          <div class="spacer"></div>
          <button class="sftp-toggle">${toggleLabel}</button>
        </div>
        <div class="term-container"></div>
        <div class="disconnect-overlay">
          <div>Disconnected</div>
          <button class="reconnect-btn">Reconnect</button>
        </div>
      </div>
      ${transferPanelHtml}
    `;
    el.panes.appendChild(pane);

    const theme = window.TERMINAL_THEMES[state.appearance.theme] || window.TERMINAL_THEMES.dark;
    const term = new Terminal({
      theme,
      fontFamily: state.appearance.fontFamily,
      fontSize: state.appearance.fontSize,
      cursorStyle: state.appearance.cursorStyle,
      cursorBlink: true,
      scrollback: 5000
    });
    const fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(pane.querySelector('.term-container'));

    term.onData((data) => {
      if (tab.connId) window.api.ssh.write(tab.connId, data);
    });

    tab.term = term;
    tab.fitAddon = fitAddon;
    tab.dom = pane;

    pane.querySelector('.sftp-toggle').addEventListener('click', () => toggleSftp(tab));
    pane.querySelector('.reconnect-btn').addEventListener('click', () => reconnectTab(tab));

    if (protocol === 'tftp') {
      pane.querySelector('.tftp-browse').addEventListener('click', async () => {
        const files = await window.api.dialog.pickUploadFiles();
        if (files && files.length) {
          pane.querySelector('.tftp-local').value = files[0];
          const remoteInput = pane.querySelector('.tftp-remote');
          if (!remoteInput.value) remoteInput.value = files[0].split('/').pop();
        }
      });
      pane.querySelector('.tftp-upload').addEventListener('click', () => handleTftpUpload(tab));
      pane.querySelector('.tftp-download').addEventListener('click', () => handleTftpDownload(tab));
    } else {
      pane.querySelectorAll('.sftp-toolbar button').forEach((btn) => {
        btn.addEventListener('click', () => handleSftpAction(tab, btn.dataset.act));
      });
    }

    pane.addEventListener('dragover', (e) => {
      e.preventDefault();
      pane.classList.add('drag-over');
    });
    pane.addEventListener('dragleave', (e) => {
      if (!pane.contains(e.relatedTarget)) pane.classList.remove('drag-over');
    });
    pane.addEventListener('drop', (e) => {
      e.preventDefault();
      pane.classList.remove('drag-over');
      if (e.dataTransfer.files.length) uploadDroppedFiles(tab, e.dataTransfer.files);
    });
  }

  function buildLocalPane(tab) {
    const pane = document.createElement('div');
    pane.className = 'pane';
    pane.id = `pane-${tab.id}`;
    pane.innerHTML = `
      <div class="term-area">
        <div class="term-toolbar">
          <span>Local Terminal</span>
          <div class="spacer"></div>
        </div>
        <div class="term-container"></div>
        <div class="disconnect-overlay">
          <div>Disconnected</div>
          <button class="reconnect-btn">Reconnect</button>
        </div>
      </div>
    `;
    el.panes.appendChild(pane);

    const theme = window.TERMINAL_THEMES[state.appearance.theme] || window.TERMINAL_THEMES.dark;
    const term = new Terminal({
      theme,
      fontFamily: state.appearance.fontFamily,
      fontSize: state.appearance.fontSize,
      cursorStyle: state.appearance.cursorStyle,
      cursorBlink: true,
      scrollback: 5000
    });
    const fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
    term.open(pane.querySelector('.term-container'));

    term.onData((data) => {
      if (tab.connId) window.api.local.write(tab.connId, data);
    });

    tab.term = term;
    tab.fitAddon = fitAddon;
    tab.dom = pane;

    pane.querySelector('.reconnect-btn').addEventListener('click', () => reconnectTab(tab));
  }

  async function uploadDroppedFiles(tab, fileList) {
    const protocol = tab.session.transferProtocol || 'sftp';
    const localPaths = Array.from(fileList)
      .map((f) => window.api.getPathForFile(f))
      .filter(Boolean);
    if (!localPaths.length) return;

    if (protocol === 'tftp') {
      if (!tab.sftp.open) toggleSftp(tab);
      const localInput = tab.dom.querySelector('.tftp-local');
      const remoteInput = tab.dom.querySelector('.tftp-remote');
      localInput.value = localPaths[0];
      if (!remoteInput.value) remoteInput.value = localPaths[0].split('/').pop();
      tab.dom.querySelector('.tftp-status').textContent = 'File ready — click "Upload to device" to send.';
      return;
    }

    if (!tab.connId) {
      alert('Connect to a host before uploading files.');
      return;
    }
    if (!tab.sftp.open) toggleSftp(tab);
    const remoteDir = tab.sftp.path || '.';
    setSftpStatus(tab, `Uploading ${localPaths.length} file(s)…`);
    try {
      await window.api.sftp.upload(tab.connId, localPaths, remoteDir);
      setSftpStatus(tab, 'Upload complete.');
      loadSftpDir(tab, remoteDir);
    } catch (err) {
      setSftpStatus(tab, `Upload failed: ${err.message}`);
    }
  }

  async function handleTftpUpload(tab) {
    const localPath = tab.dom.querySelector('.tftp-local').value;
    const remoteFileName = tab.dom.querySelector('.tftp-remote').value.trim();
    const statusEl = tab.dom.querySelector('.tftp-status');
    if (!localPath || !remoteFileName) {
      statusEl.textContent = 'Choose a local file and enter a remote filename.';
      return;
    }
    statusEl.textContent = 'Uploading via TFTP…';
    try {
      await window.api.tftp.upload(tab.session.host, tab.session.tftpPort || 69, localPath, remoteFileName);
      statusEl.textContent = 'Upload complete.';
    } catch (err) {
      statusEl.textContent = `Upload failed: ${err.message}`;
    }
  }

  async function handleTftpDownload(tab) {
    const remoteFileName = tab.dom.querySelector('.tftp-remote').value.trim();
    const statusEl = tab.dom.querySelector('.tftp-status');
    if (!remoteFileName) {
      statusEl.textContent = 'Enter the remote filename to fetch.';
      return;
    }
    const dir = await window.api.dialog.pickDownloadDir();
    if (!dir) return;
    statusEl.textContent = 'Downloading via TFTP…';
    try {
      const result = await window.api.tftp.download(tab.session.host, tab.session.tftpPort || 69, remoteFileName, dir);
      statusEl.textContent = `Saved to ${result.localPath}`;
    } catch (err) {
      statusEl.textContent = `Download failed: ${err.message}`;
    }
  }

  function setActiveTab(tabId) {
    state.activeTabId = tabId;
    state.tabs.forEach((tab) => {
      const isActive = tab.id === tabId;
      tab.dom.classList.toggle('active', isActive);
      document.getElementById(`tabbtn-${tab.id}`).classList.toggle('active', isActive);
    });
    requestAnimationFrame(() => fitActiveTab());
  }

  function fitActiveTab() {
    const tab = state.tabs.get(state.activeTabId);
    if (!tab || !tab.term) return;
    try {
      tab.fitAddon.fit();
      if (tab.connId) resizeConn(tab);
    } catch {}
  }

  function resizeConn(tab) {
    if (tab.kind === 'local') window.api.local.resize(tab.connId, tab.term.cols, tab.term.rows);
    else window.api.ssh.resize(tab.connId, tab.term.cols, tab.term.rows);
  }

  function closeTab(tabId) {
    const tab = state.tabs.get(tabId);
    if (!tab) return;
    if (tab.connId) {
      if (tab.kind === 'local') {
        window.api.local.kill(tab.connId);
        state.localToTab.delete(tab.connId);
      } else {
        window.api.ssh.disconnect(tab.connId);
        state.connToTab.delete(tab.connId);
      }
    }
    tab.term.dispose();
    tab.dom.remove();
    document.getElementById(`tabbtn-${tab.id}`).remove();
    state.tabs.delete(tabId);

    if (state.activeTabId === tabId) {
      const remaining = Array.from(state.tabs.keys());
      if (remaining.length) setActiveTab(remaining[remaining.length - 1]);
      else state.activeTabId = null;
    }
  }

  async function connectTab(tab) {
    try {
      tab.fitAddon.fit();
      const cols = tab.term.cols || 80;
      const rows = tab.term.rows || 24;
      if (tab.kind === 'local') {
        const { id } = await window.api.local.spawn({ cols, rows });
        tab.connId = id;
        tab.connected = true;
        state.localToTab.set(id, tab.id);
      } else {
        const { connId } = await window.api.ssh.connect({ sessionId: tab.session.id, cols, rows });
        tab.connId = connId;
        tab.connected = true;
        state.connToTab.set(connId, tab.id);
      }
      updateTabStatus(tab);
      hideOverlay(tab);
      resizeConn(tab);
    } catch (err) {
      tab.term.writeln(`\x1b[31mConnection failed: ${err.message}\x1b[0m`);
      markDisconnected(tab);
    }
  }

  function reconnectTab(tab) {
    tab.term.reset();
    connectTab(tab);
  }

  function markDisconnected(tab) {
    tab.connected = false;
    if (tab.connId) {
      if (tab.kind === 'local') state.localToTab.delete(tab.connId);
      else state.connToTab.delete(tab.connId);
    }
    tab.connId = null;
    updateTabStatus(tab);
    showOverlay(tab);
  }

  function updateTabStatus(tab) {
    const btn = document.getElementById(`tabbtn-${tab.id}`);
    if (btn) btn.classList.toggle('disconnected', !tab.connected);
  }

  function showOverlay(tab) {
    tab.dom.querySelector('.disconnect-overlay').classList.add('show');
  }
  function hideOverlay(tab) {
    tab.dom.querySelector('.disconnect-overlay').classList.remove('show');
  }

  // ---------------- SFTP panel ----------------
  function toggleSftp(tab) {
    const protocol = tab.session.transferProtocol || 'sftp';
    tab.sftp.open = !tab.sftp.open;
    tab.dom.querySelector('.sftp-panel').classList.toggle('open', tab.sftp.open);
    tab.dom.querySelector('.sftp-toggle').classList.toggle('active-toggle', tab.sftp.open);
    if (tab.sftp.open && protocol !== 'tftp') loadSftpDir(tab, tab.sftp.path);
    requestAnimationFrame(() => fitActiveTab());
  }

  async function loadSftpDir(tab, dirPath) {
    if (!tab.connId) return setSftpStatus(tab, 'Not connected.');
    setSftpStatus(tab, 'Loading…');
    try {
      const result = await window.api.sftp.list(tab.connId, dirPath);
      tab.sftp.path = result.path;
      tab.sftp.entries = result.entries;
      tab.sftp.selected = null;
      renderSftpBreadcrumb(tab);
      renderSftpList(tab);
      setSftpStatus(tab, `${result.entries.length} item(s)`);
    } catch (err) {
      setSftpStatus(tab, `Error: ${err.message}`);
    }
  }

  function renderSftpBreadcrumb(tab) {
    const crumbEl = tab.dom.querySelector('.sftp-breadcrumb');
    const parts = tab.sftp.path.split('/').filter(Boolean);
    const isAbsolute = tab.sftp.path.startsWith('/');
    let acc = isAbsolute ? '' : '.';
    const segments = [{ label: isAbsolute ? '/' : '.', path: isAbsolute ? '/' : '.' }];
    parts.forEach((p) => {
      acc = acc === '/' || acc === '' ? '/' + p : acc + '/' + p;
      segments.push({ label: p, path: acc });
    });
    crumbEl.innerHTML = segments
      .map((s, i) => `<span data-path="${escapeHtml(s.path)}">${escapeHtml(s.label)}</span>${i < segments.length - 1 ? ' / ' : ''}`)
      .join('');
    crumbEl.querySelectorAll('span').forEach((s) => {
      s.addEventListener('click', () => loadSftpDir(tab, s.dataset.path));
    });
  }

  function renderSftpList(tab) {
    const listEl = tab.dom.querySelector('.sftp-list');
    listEl.innerHTML = '';
    if (tab.sftp.path !== '.' && tab.sftp.path !== '/') {
      const up = document.createElement('div');
      up.className = 'sftp-row';
      up.innerHTML = `<span class="icon">📁</span><span class="fname">..</span>`;
      up.addEventListener('click', () => {
        const parent = tab.sftp.path.split('/').slice(0, -1).join('/') || (tab.sftp.path.startsWith('/') ? '/' : '.');
        loadSftpDir(tab, parent);
      });
      listEl.appendChild(up);
    }
    tab.sftp.entries.forEach((entry) => {
      const row = document.createElement('div');
      row.className = 'sftp-row';
      const icon = entry.type === 'dir' ? '📁' : entry.type === 'symlink' ? '🔗' : '📄';
      row.innerHTML = `<span class="icon">${icon}</span><span class="fname">${escapeHtml(entry.name)}</span><span class="fsize">${entry.type === 'dir' ? '' : formatSize(entry.size)}</span>`;
      row.addEventListener('click', () => {
        tab.dom.querySelectorAll('.sftp-row.selected').forEach((r) => r.classList.remove('selected'));
        row.classList.add('selected');
        tab.sftp.selected = entry;
      });
      row.addEventListener('dblclick', () => {
        if (entry.type === 'dir') {
          const next = tab.sftp.path === '/' ? '/' + entry.name : tab.sftp.path + '/' + entry.name;
          loadSftpDir(tab, next);
        }
      });
      listEl.appendChild(row);
    });
  }

  function formatSize(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / 1024 / 1024).toFixed(1) + ' MB';
  }

  function setSftpStatus(tab, msg) {
    tab.dom.querySelector('.sftp-status').textContent = msg;
  }

  async function handleSftpAction(tab, action) {
    if (!tab.connId) return setSftpStatus(tab, 'Not connected.');
    const joinPath = (name) => (tab.sftp.path === '/' ? '/' + name : tab.sftp.path + '/' + name);

    if (action === 'refresh') return loadSftpDir(tab, tab.sftp.path);

    if (action === 'upload') {
      const files = await window.api.dialog.pickUploadFiles();
      if (!files.length) return;
      setSftpStatus(tab, 'Uploading…');
      try {
        await window.api.sftp.upload(tab.connId, files, tab.sftp.path);
        setSftpStatus(tab, 'Upload complete.');
        loadSftpDir(tab, tab.sftp.path);
      } catch (err) {
        setSftpStatus(tab, `Upload failed: ${err.message}`);
      }
      return;
    }

    if (action === 'download') {
      if (!tab.sftp.selected) return setSftpStatus(tab, 'Select a file first.');
      if (tab.sftp.selected.type === 'dir') return setSftpStatus(tab, 'Folder download not supported.');
      const dir = await window.api.dialog.pickDownloadDir();
      if (!dir) return;
      setSftpStatus(tab, 'Downloading…');
      try {
        await window.api.sftp.download(tab.connId, [joinPath(tab.sftp.selected.name)], dir);
        setSftpStatus(tab, 'Download complete.');
      } catch (err) {
        setSftpStatus(tab, `Download failed: ${err.message}`);
      }
      return;
    }

    if (action === 'mkdir') {
      const name = prompt('New folder name:');
      if (!name) return;
      try {
        await window.api.sftp.mkdir(tab.connId, joinPath(name));
        loadSftpDir(tab, tab.sftp.path);
      } catch (err) {
        setSftpStatus(tab, `Error: ${err.message}`);
      }
      return;
    }

    if (action === 'delete') {
      if (!tab.sftp.selected) return setSftpStatus(tab, 'Select an item first.');
      const target = tab.sftp.selected;
      showConfirm(`Delete "${target.name}"?`, async () => {
        try {
          await window.api.sftp.delete(tab.connId, joinPath(target.name), target.type === 'dir');
          loadSftpDir(tab, tab.sftp.path);
        } catch (err) {
          setSftpStatus(tab, `Error: ${err.message}`);
        }
      });
    }
  }

  // ---------------- Quick Commands ----------------
  function renderQuickCommandList() {
    el.cmdList.innerHTML = '';
    if (!state.quickCommands.length) {
      el.cmdList.innerHTML = '<div class="cmd-empty">No quick commands yet. Click + to add one.</div>';
      return;
    }
    state.quickCommands.forEach((cmd) => {
      const item = document.createElement('div');
      item.className = 'cmd-item';
      item.innerHTML = `
        <div class="cmd-text">
          <div class="cmd-name">${escapeHtml(cmd.name)}</div>
          <div class="cmd-preview">${escapeHtml(cmd.command.split('\n')[0])}</div>
        </div>
        <div class="actions">
          <button data-act="edit" title="Edit">✎</button>
          <button data-act="delete" title="Delete">🗑</button>
        </div>
      `;
      item.title = cmd.command;
      item.addEventListener('click', (e) => {
        const act = e.target.closest('button')?.dataset.act;
        if (act === 'edit') return openCommandModal(cmd);
        if (act === 'delete') return confirmDeleteCommand(cmd);
        sendQuickCommand(cmd);
      });
      el.cmdList.appendChild(item);
    });
  }

  function toggleCommandPanel() {
    state.cmdPanelOpen = !state.cmdPanelOpen;
    el.cmdPanel.classList.toggle('open', state.cmdPanelOpen);
    el.cmdPanelToggle.classList.toggle('active-toggle', state.cmdPanelOpen);
    requestAnimationFrame(() => fitActiveTab());
  }

  function sendQuickCommand(cmd) {
    const tab = state.tabs.get(state.activeTabId);
    if (!tab || !tab.connId) {
      alert('No active connected session to send this command to.');
      return;
    }
    const text = cmd.runEnter ? cmd.command.replace(/\n?$/, '\r') : cmd.command;
    if (tab.kind === 'local') window.api.local.write(tab.connId, text);
    else window.api.ssh.write(tab.connId, text);
    tab.term.focus();
  }

  function openCommandModal(cmd) {
    state.editingCommandId = cmd ? cmd.id : null;
    el.cmdModalTitle.textContent = cmd ? 'Edit Command' : 'New Command';
    el.c_name.value = cmd?.name || '';
    el.c_command.value = cmd?.command || '';
    el.c_runEnter.checked = cmd ? cmd.runEnter !== false : true;
    el.cmdModal.classList.remove('hidden');
    el.c_name.focus();
  }

  async function saveCommandForm() {
    const name = el.c_name.value.trim();
    const command = el.c_command.value;
    if (!name || !command) {
      alert('Name and command are required.');
      return;
    }
    await window.api.quickCommands.save({
      id: state.editingCommandId || undefined,
      name,
      command,
      runEnter: el.c_runEnter.checked
    });
    state.quickCommands = await window.api.quickCommands.list();
    renderQuickCommandList();
    el.cmdModal.classList.add('hidden');
  }

  function confirmDeleteCommand(cmd) {
    showConfirm(`Delete command "${cmd.name}"?`, async () => {
      await window.api.quickCommands.delete(cmd.id);
      state.quickCommands = await window.api.quickCommands.list();
      renderQuickCommandList();
    });
  }

  // ---------------- Global UI wiring ----------------
  function wireGlobalUI() {
    el.newSessionBtn.addEventListener('click', () => openSessionModal(null));
    el.newLocalBtn.addEventListener('click', () => createLocalTab());
    el.settingsBtn.addEventListener('click', openSettingsModal);

    document.querySelectorAll('.auth-tab').forEach((btn) => {
      btn.addEventListener('click', () => setAuthType(btn.dataset.auth));
    });
    el.f_transferProtocol.addEventListener('change', () => {
      el.tftpPortRow.classList.toggle('hidden', el.f_transferProtocol.value !== 'tftp');
    });
    el.browseKeyBtn.addEventListener('click', async () => {
      const filePath = await window.api.dialog.pickKeyFile();
      if (filePath) el.f_keyPath.value = filePath;
    });
    el.sessionCancelBtn.addEventListener('click', closeSessionModal);
    el.sessionSaveBtn.addEventListener('click', () => saveSessionForm(false));
    el.sessionSaveConnectBtn.addEventListener('click', () => saveSessionForm(true));

    el.settingsCancelBtn.addEventListener('click', () => el.settingsModal.classList.add('hidden'));
    el.settingsSaveBtn.addEventListener('click', saveSettings);

    el.cmdPanelToggle.addEventListener('click', toggleCommandPanel);
    el.cmdAddBtn.addEventListener('click', () => openCommandModal(null));
    el.cmdCancelBtn.addEventListener('click', () => el.cmdModal.classList.add('hidden'));
    el.cmdSaveBtn.addEventListener('click', saveCommandForm);

    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        el.sessionModal.classList.add('hidden');
        el.settingsModal.classList.add('hidden');
        el.confirmModal.classList.add('hidden');
        el.cmdModal.classList.add('hidden');
      }
    });
  }

  init();
})();
