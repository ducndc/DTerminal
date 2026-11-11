const os = require('os');
const crypto = require('crypto');
const pty = require('node-pty');

// id -> pty process
const terminals = new Map();

function defaultShell() {
  return process.env.SHELL || '/bin/bash';
}

function spawn(opts, emit) {
  const id = crypto.randomUUID();
  const shell = defaultShell();
  const cwd = opts.cwd || os.homedir();

  const term = pty.spawn(shell, [], {
    name: 'xterm-256color',
    cols: opts.cols || 80,
    rows: opts.rows || 24,
    cwd,
    env: process.env
  });

  terminals.set(id, term);

  term.onData((data) => emit('local:data', { id, data }));
  term.onExit(({ exitCode }) => {
    emit('local:exit', { id, exitCode });
    terminals.delete(id);
  });

  return { id, shell, cwd };
}

function write(id, data) {
  const term = terminals.get(id);
  if (!term) return false;
  term.write(data);
  return true;
}

function resize(id, cols, rows) {
  const term = terminals.get(id);
  if (!term) return false;
  if (cols > 0 && rows > 0) term.resize(cols, rows);
  return true;
}

function kill(id) {
  const term = terminals.get(id);
  if (!term) return false;
  term.kill();
  terminals.delete(id);
  return true;
}

function killAll() {
  for (const id of Array.from(terminals.keys())) kill(id);
}

module.exports = { spawn, write, resize, kill, killAll };
