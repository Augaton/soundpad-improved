// Pure logic shared by the renderer and the Node tests: no DOM, no Electron.

export const MODES = ['overlap', 'restart', 'toggle'];

export const DEFAULT_SETTINGS = {
  monitorDevice: { id: '', label: '' }, // '' = default Windows output
  cableDevice: undefined,               // undefined = never chosen (auto-detect), null = off
  micDevice: { id: '', label: '' },
  micEnabled: true,
  micNoiseSuppression: false,
  micVolume: 1,
  monitorVolume: 0.8,
  cableVolume: 1,
  normalize: true,
  defaultMode: 'overlap',
  exclusive: false,
  maxVoices: 16,
  hotkeys: { stopAll: null, pauseAll: null, volUp: null, volDown: null },
};

// ---------- Titles ----------

export function titleFromPath(p) {
  const base = String(p).split(/[\\/]/).pop() ?? '';
  return base.replace(/\.[^.]+$/, '');
}

// "kirby-falling-meme-scream-made-with-Voicemod" -> "kirby falling meme scream"
export function cleanTitle(raw) {
  const original = String(raw ?? '').trim();
  let t = original.replace(/[-_ ]*made[-_ ]with[-_ ]voicemod$/i, '').trim();
  if (!/\s/.test(t)) t = t.replace(/[-_]+/g, ' ').trim();
  return t || original;
}

// Case- and accent-insensitive form used by the search box ("Même" matches "meme").
export function searchKey(text) {
  return String(text).normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

export function formatTime(sec) {
  if (!Number.isFinite(sec) || sec < 0) sec = 0;
  const s = Math.floor(sec);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export function parseDuration(text) {
  const parts = String(text ?? '').split(':').map(Number);
  if (!parts.length || parts.some((n) => !Number.isFinite(n))) return null;
  return parts.reduce((acc, n) => acc * 60 + n, 0);
}

// ---------- Hotkeys ----------
// Stored as readable strings ("Ctrl+Shift+num1"), registered as Win32 {vk, mods} by hotkeys.ps1.

const VK_NAMES = {
  8: 'Backspace', 9: 'Tab', 13: 'Enter', 19: 'Pause', 20: 'Capslock', 27: 'Esc', 32: 'Space',
  33: 'PageUp', 34: 'PageDown', 35: 'End', 36: 'Home', 37: 'Left', 38: 'Up', 39: 'Right', 40: 'Down',
  44: 'PrintScreen', 45: 'Insert', 46: 'Delete',
  106: 'nummult', 107: 'numadd', 109: 'numsub', 110: 'numdec', 111: 'numdiv',
  144: 'Numlock', 145: 'Scrolllock',
  173: 'VolumeMute', 174: 'VolumeDown', 175: 'VolumeUp',
  176: 'MediaNextTrack', 177: 'MediaPreviousTrack', 178: 'MediaStop', 179: 'MediaPlayPause',
  186: ';', 187: '=', 188: ',', 189: '-', 190: '.', 191: '/', 192: '`',
  219: '[', 220: '\\', 221: ']', 222: "'",
};

// Windows virtual-key code -> accelerator key name, or null when unsupported.
export function vkToKey(vk) {
  if ((vk >= 48 && vk <= 57) || (vk >= 65 && vk <= 90)) return String.fromCharCode(vk);
  if (vk >= 96 && vk <= 105) return `num${vk - 96}`;
  if (vk >= 112 && vk <= 135) return `F${vk - 111}`;
  return VK_NAMES[vk] ?? null;
}

// Soundpad stores RegisterHotKey-style modifiers: 1 Alt, 2 Ctrl, 4 Shift, 8 Win.
export function vkToAccelerator(vk, mods = 0) {
  const key = vkToKey(vk);
  if (!key) return null;
  const parts = [];
  if (mods & 2) parts.push('Ctrl');
  if (mods & 1) parts.push('Alt');
  if (mods & 4) parts.push('Shift');
  if (mods & 8) parts.push('Super');
  return [...parts, key].join('+');
}

const KEY_TO_VK = new Map();
for (let vk = 255; vk > 0; vk--) {
  const key = vkToKey(vk);
  if (key) KEY_TO_VK.set(key.toLowerCase(), vk);
}
const MOD_BITS = { alt: 1, ctrl: 2, control: 2, shift: 4, super: 8, win: 8 };

// "Ctrl+Shift+num1" -> { vk: 97, mods: 6 }, or null when the string is not a valid hotkey.
export function parseAccelerator(accel) {
  const parts = String(accel ?? '').split('+');
  const vk = KEY_TO_VK.get(parts.pop().toLowerCase());
  if (!vk) return null;
  let mods = 0;
  for (const p of parts) {
    const bit = MOD_BITS[p.toLowerCase()];
    if (!bit) return null;
    mods |= bit;
  }
  return { vk, mods };
}

const MODIFIER_KEYS = new Set(['Control', 'Shift', 'Alt', 'AltGraph', 'Meta', 'OS']);

// KeyboardEvent -> hotkey string. keyCode is the Windows VK code in Chromium, which keeps the
// mapping identical to what RegisterHotKey sees (AZERTY letters included).
export function eventToAccelerator(e) {
  if (MODIFIER_KEYS.has(e.key)) return null;
  const key = vkToKey(e.keyCode);
  if (!key) return null;
  let mods = 0;
  if (e.altKey) mods |= 1;
  if (e.ctrlKey) mods |= 2;
  if (e.shiftKey) mods |= 4;
  if (e.metaKey) mods |= 8;
  return vkToAccelerator(e.keyCode, mods);
}

const KEY_LABELS = {
  Shift: 'Maj', Super: 'Win', nummult: 'Num *', numadd: 'Num +', numsub: 'Num -', numdec: 'Num .',
  numdiv: 'Num /', Up: 'Haut', Down: 'Bas', Left: 'Gauche', Right: 'Droite', Delete: 'Suppr',
  Insert: 'Inser', PageUp: 'Pg Préc', PageDown: 'Pg Suiv', Home: 'Début', End: 'Fin',
  Space: 'Espace', Enter: 'Entrée', Esc: 'Échap', Backspace: 'Retour', Scrolllock: 'Arrêt défil',
  PrintScreen: 'Impr écran', Capslock: 'Verr maj', Numlock: 'Verr num',
};

export function formatAccelerator(accel) {
  if (!accel) return '';
  return accel.split('+').map((k) => KEY_LABELS[k] ?? k.replace(/^num(\d)$/, 'Num $1')).join(' + ');
}

// Every global hotkey the app needs, as { accel, vk, mods, action }. Unparseable ones are skipped.
export function buildHotkeys(library) {
  const list = [];
  const add = (accel, action) => {
    const parsed = accel && parseAccelerator(accel);
    if (parsed) list.push({ accel, ...parsed, action });
  };
  for (const s of library.sounds) add(s.hotkey, `play:${s.id}`);
  for (const [action, accel] of Object.entries(library.settings.hotkeys)) add(accel, action);
  return list;
}

// ---------- Loudness ----------

// Peak and gated RMS (50 ms blocks, blocks quieter than -50 dBFS ignored, so silence
// around a short sound does not drag its loudness down).
export function analyse(channels, sampleRate) {
  const n = channels[0]?.length ?? 0;
  const block = Math.max(1, Math.round(sampleRate * 0.05));
  const gate = 10 ** (-50 / 10);
  let peak = 0;
  let gatedSum = 0;
  let gatedBlocks = 0;
  for (let start = 0; start < n; start += block) {
    const end = Math.min(n, start + block);
    let sum = 0;
    for (const ch of channels) {
      for (let i = start; i < end; i++) {
        const v = ch[i];
        sum += v * v;
        const a = v < 0 ? -v : v;
        if (a > peak) peak = a;
      }
    }
    const meanSquare = sum / ((end - start) * channels.length);
    if (meanSquare > gate) {
      gatedSum += meanSquare;
      gatedBlocks++;
    }
  }
  return { peak, rms: gatedBlocks ? Math.sqrt(gatedSum / gatedBlocks) : 0 };
}

// Gain that brings a sound to targetDb RMS without pushing its peak past 0 dBFS, capped at +12 dB.
export function normalizeGain({ peak, rms }, targetDb = -18) {
  if (!(rms > 0) || !(peak > 0)) return 1;
  return Math.min(10 ** (targetDb / 20) / rms, 1 / peak, 4);
}

// ---------- Playback decisions ----------

export function effectiveMode(sound, settings) {
  return MODES.includes(sound.mode) ? sound.mode : settings.defaultMode;
}

// Which voices to stop and whether to start a new one when a sound is triggered.
export function planPlay(mode, sameSoundIds, allIds, exclusive) {
  if (mode === 'toggle' && sameSoundIds.length) return { stop: [...sameSoundIds], play: false };
  if (exclusive) return { stop: [...allIds], play: true };
  if (mode === 'restart') return { stop: [...sameSoundIds], play: true };
  return { stop: [], play: true };
}

// Playback position of one voice, in seconds of source audio, across rate changes and pauses.
export class PlayheadClock {
  constructor(now, rate = 1, pos = 0) {
    this.base = pos;
    this.t0 = now;
    this.rate = rate;
    this.paused = false;
  }
  position(now) {
    return this.paused ? this.base : this.base + (now - this.t0) * this.rate;
  }
  setRate(rate, now) {
    this.base = this.position(now);
    this.t0 = now;
    this.rate = rate;
  }
  seek(pos, now) {
    this.base = pos;
    this.t0 = now;
  }
  pause(now) {
    this.base = this.position(now);
    this.paused = true;
  }
  resume(now) {
    this.t0 = now;
    this.paused = false;
  }
}

// ---------- Devices ----------

// Saved device ({id, label}) -> current deviceId. Chromium device ids can change between
// sessions, so the label is the fallback. Returns null when the device is gone.
export function resolveDevice(devices, saved) {
  if (!saved) return null;
  if (saved.id === '') return '';
  return (devices.find((d) => d.deviceId === saved.id) ?? devices.find((d) => d.label === saved.label))?.deviceId ?? null;
}

export function findVirtualCable(outputs) {
  return outputs.find((d) => /CABLE Input|VB-Audio|Voicemeeter Input/i.test(d.label)) ?? null;
}

// ---------- Microphone filter (native/) ----------

// cableDevice value for "my real microphone, through the filter": not an output device.
export const MIC_FILTER = { id: 'mic-filter', label: 'Mon micro, avec le filtre' };
export const isMicFilter = (saved) => saved?.id === MIC_FILTER.id;

const FILTER_ERRORS = {
  'not-admin': 'Windows n’a pas donné les droits administrateur.',
  'not-built': 'Filtre non compilé : lance native\\build.cmd (g++ de MSYS2).',
  'no-mic': 'Aucun micro compatible.',
  cancelled: 'Autorisation refusée.',
};
export const micFilterError = (code) => FILTER_ERRORS[code] ?? String(code);

// Settings text about the filter. status: mic-filter.ps1's answer, null while unknown.
// live: spi-feed's state (NOMAP, IDLE, LIVE <rate>), known only while the app sends sounds.
export function micFilterMessage(status, live) {
  if (!status) return 'Vérification du filtre…';
  if (!status.ok) return `État du filtre inconnu : ${micFilterError(status.error)}`;
  const list = (names) => names.map((n) => `« ${n} »`).join(', ');
  if (!status.installed.length) {
    if (!status.eligible.length) {
      return 'Aucun micro compatible : le filtre se pose sur un micro branché qui n’a pas déjà un effet du fabricant.';
    }
    const soundpad = status.soundpad.length
      ? ' Il remplace celui de Soundpad : Soundpad n’enverra plus de son tant que ce filtre est là, et le retirer remet celui de Soundpad.'
      : '';
    return `Filtre non installé. Il se posera sur ${list(status.eligible)}. Windows demandera l’autorisation, et le son du PC se coupera une seconde.${soundpad}`;
  }
  const where = list(status.installed);
  if (live?.startsWith('LIVE')) return `Actif sur ${where} : toutes les applis qui écoutent ce micro entendent les sons.`;
  if (live === 'IDLE') return `Installé sur ${where}. Aucune appli n’écoute ce micro en ce moment.`;
  return `Installé sur ${where}. Il se met en marche dès qu’une appli (Discord, un jeu…) écoute ce micro.`;
}

// ---------- Library ----------

export function newSound(path, extra = {}) {
  return {
    id: crypto.randomUUID(),
    path,
    title: cleanTitle(titleFromPath(path)),
    volume: 1,
    rate: 1,
    mode: 'default',
    hotkey: null,
    favorite: false,
    plays: 0,
    duration: null,
    ...extra,
  };
}

const num = (v, min, max, fallback) => (Number.isFinite(v) ? Math.min(max, Math.max(min, v)) : fallback);
const device = (d) => (d && typeof d.id === 'string' ? { id: d.id, label: String(d.label ?? '') } : null);

// Data read from disk is untrusted: keep only well-formed fields, fill the rest with defaults.
export function normalizeLibrary(raw) {
  const src = raw && typeof raw === 'object' ? raw : {};
  const sounds = (Array.isArray(src.sounds) ? src.sounds : [])
    .filter((s) => s && typeof s.id === 'string' && typeof s.path === 'string')
    .map((s) => ({
      ...newSound(s.path),
      id: s.id,
      title: typeof s.title === 'string' && s.title ? s.title : cleanTitle(titleFromPath(s.path)),
      volume: num(s.volume, 0, 2, 1),
      rate: num(s.rate, 0.5, 2, 1),
      mode: MODES.includes(s.mode) ? s.mode : 'default',
      hotkey: typeof s.hotkey === 'string' && s.hotkey ? s.hotkey : null,
      favorite: s.favorite === true,
      plays: num(s.plays, 0, Number.MAX_SAFE_INTEGER, 0),
      duration: num(s.duration, 0, 86400, null),
    }));
  const ids = new Set(sounds.map((s) => s.id));
  const categories = (Array.isArray(src.categories) ? src.categories : [])
    .filter((c) => c && typeof c.id === 'string' && typeof c.name === 'string')
    .map((c) => ({ id: c.id, name: c.name, soundIds: (Array.isArray(c.soundIds) ? c.soundIds : []).filter((id) => ids.has(id)) }));
  const s = src.settings && typeof src.settings === 'object' ? src.settings : {};
  const d = DEFAULT_SETTINGS;
  const hk = s.hotkeys && typeof s.hotkeys === 'object' ? s.hotkeys : {};
  const settings = {
    monitorDevice: device(s.monitorDevice) ?? d.monitorDevice,
    cableDevice: s.cableDevice === null ? null : device(s.cableDevice) ?? undefined,
    micDevice: device(s.micDevice) ?? d.micDevice,
    micEnabled: typeof s.micEnabled === 'boolean' ? s.micEnabled : d.micEnabled,
    micNoiseSuppression: s.micNoiseSuppression === true,
    micVolume: num(s.micVolume, 0, 2, d.micVolume),
    monitorVolume: num(s.monitorVolume, 0, 2, d.monitorVolume),
    cableVolume: num(s.cableVolume, 0, 2, d.cableVolume),
    normalize: typeof s.normalize === 'boolean' ? s.normalize : d.normalize,
    defaultMode: MODES.includes(s.defaultMode) ? s.defaultMode : d.defaultMode,
    exclusive: s.exclusive === true,
    maxVoices: Math.round(num(s.maxVoices, 1, 64, d.maxVoices)),
    hotkeys: Object.fromEntries(Object.keys(d.hotkeys).map((k) => [k, typeof hk[k] === 'string' && hk[k] ? hk[k] : null])),
  };
  return { version: 1, sounds, categories, settings };
}

// ---------- Soundpad import ----------

const ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" };

export function decodeEntities(s) {
  return s.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (m, e) => {
    if (e[0] !== '#') return ENTITIES[e] ?? m;
    const code = e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
    return String.fromCodePoint(code);
  });
}

// Start/end tags with their attributes. Enough for Soundpad's flat, attribute-only XML.
function* xmlTags(text) {
  const tagRe = /<(\/?)([A-Za-z_][\w.-]*)((?:\s+[\w:.-]+\s*=\s*(?:"[^"]*"|'[^']*'))*)\s*(\/?)>/g;
  for (const m of text.matchAll(tagRe)) {
    const attrs = {};
    for (const a of m[3].matchAll(/([\w:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')/g)) attrs[a[1]] = decodeEntities(a[2] ?? a[3]);
    yield { close: m[1] === '/', name: m[2], attrs, selfClosing: m[4] === '/' };
  }
}

// Soundpad soundlist.spl -> { sounds: [{path, title, hotkey, duration}], categories: [{name, indexes}] }.
// Category entries reference sounds by 1-based position in the list.
export function parseSoundpadList(text) {
  const sounds = [];
  const categories = [];
  const stack = [];
  const catStack = [];
  for (const t of xmlTags(text)) {
    if (t.close) {
      if (stack.pop() === 'Category') catStack.pop();
      continue;
    }
    const parent = stack.at(-1);
    if (t.name === 'Sound' && parent === 'Soundlist' && t.attrs.url) {
      const key = Number(t.attrs.key);
      sounds.push({
        path: t.attrs.url,
        title: cleanTitle(t.attrs.title || titleFromPath(t.attrs.url)),
        hotkey: Number.isInteger(key) && key > 0 ? vkToAccelerator(key, Number(t.attrs.keyModifiers) || 0) : null,
        duration: parseDuration(t.attrs.duration),
      });
    } else if (t.name === 'Sound' && parent === 'Category') {
      catStack.at(-1)?.ids.push(Number(t.attrs.id));
    } else if (t.name === 'Category') {
      // Built-in categories ("All sounds") carry a type and no name.
      const named = !t.attrs.type && t.attrs.name;
      const cat = named ? { name: [...catStack.filter(Boolean).map((c) => c.name), t.attrs.name].join(' / '), ids: [] } : null;
      if (cat) categories.push(cat);
      if (!t.selfClosing) {
        stack.push('Category');
        catStack.push(cat);
      }
      continue;
    }
    if (!t.selfClosing) stack.push(t.name);
  }
  return {
    sounds,
    categories: categories.map((c) => ({
      name: c.name,
      indexes: c.ids.filter((i) => Number.isInteger(i) && i >= 1 && i <= sounds.length).map((i) => i - 1),
    })),
  };
}

const samePath = (a, b) => a.toLowerCase() === b.toLowerCase();

// Adds an imported Soundpad list to the library in place. Sounds already present (same path)
// are reused, never duplicated; a hotkey is only taken over when no other sound uses it.
export function mergeSoundpadImport(library, parsed) {
  const usedKeys = new Set(library.sounds.map((s) => s.hotkey).filter(Boolean));
  let added = 0;
  const ids = parsed.sounds.map((p) => {
    const existing = library.sounds.find((s) => samePath(s.path, p.path));
    if (existing) return existing.id;
    const hotkey = p.hotkey && !usedKeys.has(p.hotkey) ? p.hotkey : null;
    if (hotkey) usedKeys.add(hotkey);
    const sound = newSound(p.path, { title: p.title, hotkey, duration: p.duration });
    library.sounds.push(sound);
    added++;
    return sound.id;
  });
  for (const pc of parsed.categories) {
    let cat = library.categories.find((c) => c.name === pc.name);
    if (!cat) {
      cat = { id: crypto.randomUUID(), name: pc.name, soundIds: [] };
      library.categories.push(cat);
    }
    for (const i of pc.indexes) if (!cat.soundIds.includes(ids[i])) cat.soundIds.push(ids[i]);
  }
  return { added, skipped: parsed.sounds.length - added };
}
