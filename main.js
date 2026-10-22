'use strict';
const { app, BrowserWindow, ipcMain, dialog, session, shell } = require('electron');
const { execFile, spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const AUDIO_EXT = ['mp3', 'wav', 'ogg', 'oga', 'opus', 'm4a', 'aac', 'flac', 'webm'];
const MAX_AUDIO_BYTES = 512 * 1024 * 1024;
const MAX_SCAN_FILES = 5000;

// Dev/test hooks: isolated profile, and a screenshot of the window after load.
if (process.env.SPI_USER_DATA) app.setPath('userData', process.env.SPI_USER_DATA);

const libraryFile = () => path.join(app.getPath('userData'), 'library.json');
const soundpadFile = () => path.join(app.getPath('appData'), 'Leppsoft', 'soundlist.spl');
const isAudioPath = (p) =>
  typeof p === 'string' && path.isAbsolute(p) && AUDIO_EXT.includes(path.extname(p).slice(1).toLowerCase());

let win = null;
let quitting = false;

// ---------- Library file ----------

async function loadLibrary() {
  let text;
  try {
    text = await fs.promises.readFile(libraryFile(), 'utf8');
  } catch (err) {
    if (err.code === 'ENOENT') return { data: null };
    throw err;
  }
  try {
    return { data: JSON.parse(text) };
  } catch {
    // Keep the unreadable file aside rather than overwrite it with an empty library.
    const aside = libraryFile().replace(/\.json$/, `.illisible-${Date.now()}.json`);
    await fs.promises.rename(libraryFile(), aside);
    return { data: null, warning: `Bibliothèque illisible, mise de côté dans ${aside}` };
  }
}

let saveChain = Promise.resolve();
function saveLibrary(data) {
  if (!data || !Array.isArray(data.sounds)) throw new Error('Bibliothèque invalide');
  const text = JSON.stringify(data, null, 2);
  // Serialized, and written to a temp file first so a crash never leaves half a file.
  const job = saveChain.catch(() => {}).then(async () => {
    const tmp = `${libraryFile()}.tmp`;
    await fs.promises.writeFile(tmp, text, 'utf8');
    await fs.promises.rename(tmp, libraryFile());
  });
  saveChain = job;
  return job;
}

async function expandPaths(paths) {
  const out = [];
  async function walk(p, depth) {
    if (out.length >= MAX_SCAN_FILES) return;
    let stat;
    try {
      stat = await fs.promises.stat(p);
    } catch {
      return;
    }
    if (stat.isFile() && isAudioPath(p)) out.push(p);
    else if (stat.isDirectory() && depth < 6) {
      const entries = await fs.promises.readdir(p).catch(() => []);
      for (const e of entries) await walk(path.join(p, e), depth + 1);
    }
  }
  for (const p of Array.isArray(paths) ? paths : []) {
    if (typeof p === 'string' && path.isAbsolute(p)) await walk(p, 0);
  }
  return out;
}

// ---------- Global hotkeys (hotkeys.ps1, Win32 RegisterHotKey) ----------

const hotkeys = {
  proc: null,
  ready: null,
  seq: 0,
  nextId: 1,
  actions: new Map(), // registration id -> action
  pending: new Map(), // SET seq -> resolve(failedIds | null)
  last: [],
  restarts: 0,

  start() {
    const proc = spawn(
      'powershell.exe',
      ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(__dirname, 'hotkeys.ps1')],
      { windowsHide: true },
    );
    this.proc = proc;
    let buf = '';
    proc.stdout.setEncoding('utf8');
    proc.stderr.on('data', (d) => console.error('[hotkeys]', String(d).trim()));
    this.ready = new Promise((resolve, reject) => {
      proc.once('error', reject);
      proc.once('exit', (code) => reject(new Error(`assistant de raccourcis arrêté (code ${code})`)));
      proc.stdout.on('data', (chunk) => {
        buf += chunk;
        let i;
        while ((i = buf.indexOf('\n')) >= 0) {
          const line = buf.slice(0, i).trim();
          buf = buf.slice(i + 1);
          if (line === 'READY') resolve();
          else this.onLine(line);
        }
      });
    });
    this.ready.catch(() => {});
    proc.once('exit', () => {
      this.proc = null;
      for (const resolve of this.pending.values()) resolve(null);
      this.pending.clear();
      if (!quitting && this.restarts < 3) {
        this.restarts++;
        setTimeout(() => this.set(this.last).catch(() => {}), 1000);
      }
    });
  },

  onLine(line) {
    const [kind, a, b] = line.split(' ');
    if (kind === 'HIT') {
      const action = this.actions.get(Number(a));
      if (action && win) win.webContents.send('hotkey', action);
    } else if (kind === 'DONE') {
      const resolve = this.pending.get(Number(a));
      this.pending.delete(Number(a));
      resolve?.(b ? b.split(',').map(Number) : []);
    }
  },

  // list: [{ accel, vk, mods, action }] -> accelerators that could not be registered.
  async set(list) {
    const valid = (Array.isArray(list) ? list : []).filter(
      (h) => h && Number.isInteger(h.vk) && h.vk > 0 && h.vk < 256 && Number.isInteger(h.mods) && h.mods >= 0 && h.mods < 16 && typeof h.action === 'string',
    );
    this.last = valid;
    if (!this.proc) this.start();
    await this.ready;
    const proc = this.proc;
    if (!proc) throw new Error('assistant de raccourcis arrêté');
    const seq = ++this.seq;
    const ids = valid.map(() => {
      this.nextId = (this.nextId % 0xbffe) + 1; // RegisterHotKey app ids: 1..0xBFFF
      return this.nextId;
    });
    this.actions = new Map(ids.map((id, i) => [id, valid[i].action]));
    const failed = await new Promise((resolve) => {
      this.pending.set(seq, resolve);
      proc.stdin.write(`SET ${seq} ${valid.map((h, i) => `${ids[i]}:${h.mods}:${h.vk}`).join(' ')}\n`);
    });
    if (failed === null) throw new Error('assistant de raccourcis arrêté');
    return valid.filter((_, i) => failed.includes(ids[i])).map((h) => h.accel);
  },

  stop() {
    this.proc?.stdin.end();
  },
};

// ---------- Microphone filter (native/: an APO in Windows' audio engine, fed by spi-feed.exe) ----------

const NATIVE = path.join(__dirname, 'native');
const MAX_PCM_SAMPLES = 48000 * 2; // one second of stereo: far above the ~10 ms blocks sent

const micFeed = {
  proc: null,
  startedAt: 0,
  state: 'NOMAP',

  write(pcm) {
    if (!this.proc) {
      if (Date.now() - this.startedAt < 2000) return; // crashed or missing: do not respawn 100 times a second
      this.start();
    }
    this.proc.stdin.write(Buffer.from(pcm.buffer, pcm.byteOffset, pcm.byteLength));
  },

  start() {
    this.startedAt = Date.now();
    const proc = spawn(path.join(NATIVE, 'bin', 'spi-feed.exe'), [], { windowsHide: true, stdio: ['pipe', 'pipe', 'inherit'] });
    this.proc = proc;
    proc.stdin.on('error', () => {}); // EPIPE once it is gone; 'exit' cleans up
    proc.on('error', (err) => {
      console.error('[spi-feed]', err.message); // not built: native\build.cmd
      if (this.proc === proc) this.proc = null;
    });
    let buf = '';
    proc.stdout.setEncoding('utf8');
    proc.stdout.on('data', (chunk) => {
      buf += chunk;
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).trim();
        buf = buf.slice(i + 1);
        if (line.startsWith('STATE ')) this.setState(line.slice(6));
      }
    });
    proc.once('close', () => {
      if (this.proc === proc) this.proc = null;
      this.setState('NOMAP');
    });
  },

  setState(state) {
    this.state = state;
    win?.webContents.send('mic:state', state);
  },

  stop() {
    this.proc?.stdin.end();
  },
};

function powershell(args) {
  return new Promise((resolve) =>
    execFile('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', ...args], { windowsHide: true }, (err, stdout) =>
      resolve({ err, stdout }),
    ),
  );
}

// status runs as the user; install and uninstall need an administrator, so they go through
// Windows' UAC prompt and leave their JSON answer in a temp file.
async function runMicFilter(action) {
  const script = path.join(NATIVE, 'mic-filter.ps1');
  if (action === 'status') {
    const { stdout } = await powershell(['-File', script, '-Action', 'status']);
    try {
      return JSON.parse(stdout.trim().replace(/^﻿/, ''));
    } catch {
      return { ok: false, error: stdout.trim() || 'no-result' };
    }
  }
  const out = path.join(app.getPath('temp'), `spi-mic-filter-${process.pid}-${Date.now()}.json`);
  const inner = `-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "${script}" -Action ${action} -Out "${out}"`;
  const quoted = `'${inner.replace(/'/g, "''")}'`;
  await powershell(['-Command', `$p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList ${quoted}; exit $p.ExitCode`]);
  try {
    return JSON.parse((await fs.promises.readFile(out, 'utf8')).replace(/^﻿/, ''));
  } catch {
    return { ok: false, error: 'cancelled' }; // no answer: the UAC prompt was declined
  } finally {
    fs.promises.rm(out, { force: true }).catch(() => {});
  }
}

ipcMain.on('mic:pcm', (_e, pcm) => {
  if (pcm instanceof Float32Array && pcm.length % 2 === 0 && pcm.length <= MAX_PCM_SAMPLES) micFeed.write(pcm);
});
ipcMain.handle('mic:filter', (_e, action) => {
  if (!['status', 'install', 'uninstall'].includes(action)) throw new Error(`Action inconnue : ${action}`);
  return runMicFilter(action);
});

// ---------- IPC ----------

ipcMain.handle('library:load', () => loadLibrary());
ipcMain.handle('library:save', (_e, data) => saveLibrary(data));

ipcMain.handle('file:read', async (_e, p) => {
  if (!isAudioPath(p)) throw new Error(`Fichier audio refusé : ${p}`);
  const { size } = await fs.promises.stat(p);
  if (size > MAX_AUDIO_BYTES) throw new Error(`Fichier trop gros (${Math.round(size / 1e6)} Mo) : ${p}`);
  return fs.promises.readFile(p);
});

ipcMain.handle('files:exist', (_e, paths) =>
  Promise.all((Array.isArray(paths) ? paths : []).map((p) => fs.promises.access(String(p)).then(() => true, () => false))),
);

ipcMain.handle('paths:expand', (_e, paths) => expandPaths(paths));

ipcMain.handle('dialog:pick-files', async () => {
  const r = await dialog.showOpenDialog(win, {
    title: 'Ajouter des sons',
    properties: ['openFile', 'multiSelections'],
    filters: [{ name: 'Audio', extensions: AUDIO_EXT }],
  });
  return r.canceled ? [] : r.filePaths.filter(isAudioPath);
});

// pick=false: default Soundpad location only (null if absent). pick=true: let the user choose.
ipcMain.handle('soundpad:read', async (_e, pick) => {
  let file = soundpadFile();
  if (pick) {
    const r = await dialog.showOpenDialog(win, {
      title: 'Liste de sons Soundpad',
      defaultPath: fs.existsSync(file) ? file : app.getPath('home'),
      properties: ['openFile'],
      filters: [{ name: 'Liste Soundpad', extensions: ['spl'] }],
    });
    if (r.canceled) return null;
    file = r.filePaths[0];
  }
  try {
    return { path: file, text: await fs.promises.readFile(file, 'utf8') };
  } catch (err) {
    if (err.code === 'ENOENT' && !pick) return null;
    throw err;
  }
});

ipcMain.handle('hotkeys:set', (_e, list) => hotkeys.set(list));
ipcMain.handle('open:vbcable', () => shell.openExternal('https://vb-audio.com/Cable/'));

// ---------- Window ----------

function createWindow() {
  win = new BrowserWindow({
    width: 1320,
    height: 840,
    minWidth: 960,
    minHeight: 600,
    backgroundColor: '#15171a',
    autoHideMenuBar: true,
    title: 'Soundpad Improved',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      autoplayPolicy: 'no-user-gesture-required', // hotkeys play sounds without a click
      backgroundThrottling: false,
    },
  });
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  win.webContents.on('will-navigate', (e) => e.preventDefault()); // a dropped file must not replace the UI
  win.on('closed', () => {
    win = null;
  });
  win.loadFile(path.join(__dirname, 'src', 'index.html'));

  if (process.env.SPI_SCREENSHOT) {
    win.webContents.on('console-message', (e) => console.log('[renderer]', e.message));
    win.webContents.once('did-finish-load', () =>
      setTimeout(async () => {
        if (process.env.SPI_SCREENSHOT_JS) await win.webContents.executeJavaScript(process.env.SPI_SCREENSHOT_JS);
        await new Promise((r) => setTimeout(r, 600));
        await fs.promises.writeFile(process.env.SPI_SCREENSHOT, (await win.webContents.capturePage()).toPNG());
        app.quit();
      }, Number(process.env.SPI_SCREENSHOT_DELAY) || 2500),
    );
  }
}

if (!app.requestSingleInstanceLock()) {
  app.quit(); // a second instance would fight the first one over the hotkeys
} else {
  app.on('second-instance', () => {
    if (win) {
      if (win.isMinimized()) win.restore();
      win.focus();
    }
  });
  app.whenReady().then(() => {
    const allowed = new Set(['media', 'speaker-selection']);
    session.defaultSession.setPermissionRequestHandler((_wc, permission, cb) => cb(allowed.has(permission)));
    session.defaultSession.setPermissionCheckHandler((_wc, permission) => allowed.has(permission));
    createWindow();
  });
  app.on('before-quit', () => {
    quitting = true;
    hotkeys.stop();
    micFeed.stop();
  });
  app.on('window-all-closed', () => app.quit());
}
