import { Engine, FILTER_SINK } from './engine.mjs';
import * as L from './lib.mjs';

const api = window.api;
const $ = (id) => document.getElementById(id);

const GLOBAL_ACTIONS = {
  stopAll: 'Tout arrêter',
  pauseAll: 'Pause / reprise de tout',
  volUp: 'Monter le volume des sons en cours',
  volDown: 'Baisser le volume des sons en cours',
};
const VOLUME_STEP = 0.1;
const CABLE_OFF = '__off__';

const state = {
  lib: null,
  view: { category: 'all', query: '', sort: 'added' },
  missing: new Set(), // sound ids whose file is gone
  failedHotkeys: new Set(),
  devices: { outputs: [], inputs: [] },
  soundpadCount: 0,
  filter: { status: null, live: 'NOMAP', busy: false }, // microphone filter, see micFilterMessage
};
const engine = new Engine(api.readFile, (pcm) => api.sendMicPcm(pcm));
const pads = new Map(); // sound id -> pad element
const voiceRows = new Map(); // voice id -> row element

// ---------- Helpers ----------

function el(tag, className, text) {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text != null) e.textContent = text;
  return e;
}

function button(text, className) {
  const b = el('button', `btn ${className}`, text);
  b.type = 'button';
  return b;
}

function range(className, min, max, step, value) {
  const r = el('input', className);
  Object.assign(r, { type: 'range', min, max, step, value });
  return r;
}

const soundById = (id) => state.lib.sounds.find((s) => s.id === id);
const pct = (v) => `${Math.round(v * 100)} %`;
const rateLabel = (r) => `${r.toFixed(2).replace('.', ',')}×`;
const plural = (n, one, many) => `${n} ${n > 1 ? many : one}`;
const errText = (err) => String(err?.message ?? err).replace(/^Error invoking remote method '[^']+': (Error: )?/, '');

// Stacked, so a success message never hides an error that came just before it.
function toast(message, isError = false) {
  const box = $('toasts');
  const t = el('div', isError ? 'toast is-error' : 'toast', message);
  box.append(t);
  while (box.children.length > 3) box.firstElementChild.remove();
  setTimeout(() => t.remove(), isError ? 8000 : 3500);
}

let saveTimer = 0;
function save({ now = false } = {}) {
  clearTimeout(saveTimer);
  const run = () => api.saveLibrary(state.lib).catch((err) => toast(`Sauvegarde impossible : ${errText(err)}`, true));
  if (now) return run();
  saveTimer = setTimeout(run, 400);
  return Promise.resolve();
}

// ---------- Playing ----------

async function trigger(sound) {
  if (state.missing.has(sound.id)) {
    toast(`Fichier introuvable : ${sound.path}`, true);
    return;
  }
  const voices = [...engine.voices.values()];
  const plan = L.planPlay(
    L.effectiveMode(sound, state.lib.settings),
    voices.filter((v) => v.soundId === sound.id).map((v) => v.id),
    voices.map((v) => v.id),
    state.lib.settings.exclusive,
  );
  for (const id of plan.stop) engine.stop(id);
  if (!plan.play) return;
  try {
    const voice = await engine.play(sound);
    if (!voice) return;
    sound.plays++;
    if (!sound.duration) {
      sound.duration = voice.duration;
      pads.get(sound.id)?.querySelector('.pad-duration')?.replaceChildren(L.formatTime(voice.duration));
    }
    save();
  } catch (err) {
    toast(`Lecture impossible de « ${sound.title} » : ${errText(err)}`, true);
  }
}

function onHotkey(action) {
  if (action.startsWith('play:')) {
    const sound = soundById(action.slice(5));
    if (sound) trigger(sound);
  } else if (action === 'stopAll') engine.stopAll();
  else if (action === 'pauseAll') engine.togglePauseAll();
  else if (action === 'volUp') engine.nudgeVolumes(VOLUME_STEP);
  else if (action === 'volDown') engine.nudgeVolumes(-VOLUME_STEP);
}

// ---------- Sidebar ----------

function renderCategories() {
  const items = [
    { id: 'all', name: 'Tous les sons', count: state.lib.sounds.length },
    { id: 'fav', name: 'Favoris', count: state.lib.sounds.filter((s) => s.favorite).length },
    ...state.lib.categories.map((c) => ({ id: c.id, name: c.name, count: c.soundIds.length, removable: true })),
  ];
  $('categories').replaceChildren(
    ...items.map((it) => {
      const li = el('li', 'category');
      const b = el('button', 'category-btn');
      b.type = 'button';
      b.dataset.category = it.id;
      if (state.view.category === it.id) b.setAttribute('aria-current', 'true');
      b.append(el('span', 'category-name', it.name), el('span', 'count', String(it.count)));
      li.append(b);
      if (it.removable) {
        const x = el('button', 'category-remove', '×');
        x.type = 'button';
        x.dataset.remove = it.id;
        x.setAttribute('aria-label', `Supprimer la catégorie ${it.name}`);
        li.append(x);
      }
      return li;
    }),
  );
}

// ---------- Grid ----------

function visibleSounds() {
  const { category, query, sort } = state.view;
  let list = state.lib.sounds;
  if (category === 'fav') list = list.filter((s) => s.favorite);
  else if (category !== 'all') {
    const byId = new Map(list.map((s) => [s.id, s]));
    list = (state.lib.categories.find((c) => c.id === category)?.soundIds ?? []).map((id) => byId.get(id)).filter(Boolean);
  }
  const q = L.searchKey(query.trim());
  if (q) list = list.filter((s) => L.searchKey(s.title).includes(q));
  if (sort === 'title') list = [...list].sort((a, b) => a.title.localeCompare(b.title, 'fr'));
  else if (sort === 'plays') list = [...list].sort((a, b) => b.plays - a.plays);
  return list;
}

function padFor(sound) {
  const pad = el('div', 'pad');
  pad.dataset.id = sound.id;
  const play = el('button', 'pad-play');
  play.type = 'button';
  const meta = el('span', 'pad-meta');
  if (state.missing.has(sound.id)) {
    pad.classList.add('is-missing');
    meta.append(el('span', 'pad-missing', 'Fichier introuvable'));
  }
  if (sound.favorite) {
    const star = el('span', 'pad-fav', '★');
    star.setAttribute('aria-label', 'favori');
    meta.append(star);
  }
  meta.append(el('span', 'pad-duration', sound.duration ? L.formatTime(sound.duration) : ''));
  if (sound.hotkey) {
    const k = el('kbd', 'pad-key', L.formatAccelerator(sound.hotkey));
    if (state.failedHotkeys.has(sound.hotkey)) {
      k.classList.add('is-failed');
      k.title = 'Raccourci indisponible : une autre application l’utilise déjà';
    }
    meta.append(k);
  }
  play.append(el('span', 'pad-title', sound.title), meta);
  const count = el('span', 'pad-count');
  count.setAttribute('aria-hidden', 'true');
  const edit = el('button', 'pad-edit', '•••');
  edit.type = 'button';
  edit.setAttribute('aria-label', `Modifier ${sound.title}`);
  pad.append(play, count, edit);
  pads.set(sound.id, pad);
  return pad;
}

function renderGrid() {
  const list = visibleSounds();
  pads.clear();
  $('grid').replaceChildren(...list.map(padFor));
  $('empty').hidden = list.length > 0;
  if (!list.length) renderEmpty();
  refreshPads();
}

function renderEmpty() {
  const libEmpty = !state.lib.sounds.length;
  let title;
  let text;
  if (libEmpty) {
    title = 'Ta bibliothèque est vide';
    text = state.soundpadCount
      ? `Soundpad est installé avec ${plural(state.soundpadCount, 'son', 'sons')} : importe-les en un clic (catégories et raccourcis compris), ou glisse ici des fichiers audio ou des dossiers.`
      : 'Ajoute des sons, ou glisse ici des fichiers audio ou des dossiers.';
  } else if (state.view.query.trim()) {
    title = 'Aucun résultat';
    text = `Aucun son ne correspond à « ${state.view.query.trim()} ».`;
  } else if (state.view.category === 'fav') {
    title = 'Aucun favori';
    text = 'Coche « Favori » dans la fenêtre d’un son (bouton ••• ou clic droit sur le son).';
  } else {
    title = 'Catégorie vide';
    text = 'Glisse des fichiers sur cette vue, ou coche cette catégorie dans la fenêtre d’un son.';
  }
  $('empty-title').textContent = title;
  $('empty-text').textContent = text;
  $('empty-actions').hidden = !libEmpty;
  $('empty-import').hidden = !state.soundpadCount;
}

// Playing state on the pads: highlight and instance count.
function refreshPads() {
  const counts = new Map();
  for (const v of engine.voices.values()) counts.set(v.soundId, (counts.get(v.soundId) ?? 0) + 1);
  for (const [id, pad] of pads) {
    const n = counts.get(id) ?? 0;
    pad.classList.toggle('is-playing', n > 0);
    pad.querySelector('.pad-count').textContent = n > 1 ? `×${n}` : '';
  }
}

// ---------- Now playing ----------

function sliderLabel(text, input, output) {
  const label = el('label', 'mini');
  label.append(el('span', null, text), input, output);
  return label;
}

function voiceRow(v) {
  const li = el('li', 'voice');
  li.dataset.id = v.id;
  const head = el('div', 'voice-head');
  const time = el('span', 'voice-time');
  head.append(el('span', 'voice-title', v.title), time);
  const seek = range('voice-seek', 0, 1000, 1, 0);
  seek.setAttribute('aria-label', `Position dans ${v.title}`);
  const vol = range('voice-vol', 0, 200, 1, Math.round(v.volume * 100));
  const volOut = el('output', null, pct(v.volume));
  const rate = range('voice-rate', 50, 200, 5, Math.round(v.rate * 100));
  const rateOut = el('output', null, rateLabel(v.rate));
  const sliders = el('div', 'voice-sliders');
  sliders.append(sliderLabel('Volume', vol, volOut), sliderLabel('Vitesse', rate, rateOut));
  const pause = button('Pause', 'voice-pause');
  const loop = button('Boucle', 'voice-loop');
  const keep = button('Garder', 'voice-keep');
  keep.title = 'Garder ce volume et cette vitesse comme réglages du son';
  const stop = button('Arrêter', 'voice-stop btn-danger-text');
  const actions = el('div', 'voice-actions');
  actions.append(pause, loop, keep, stop);
  li.append(head, seek, sliders, actions);
  li.refs = { time, seek, vol, volOut, rate, rateOut, pause, loop, seeking: false };
  return li;
}

function onVoicesChanged() {
  const list = $('voices');
  for (const [id, row] of voiceRows) {
    if (!engine.voices.has(id)) {
      row.remove();
      voiceRows.delete(id);
    }
  }
  for (const v of engine.voices.values()) {
    let row = voiceRows.get(v.id);
    if (!row) {
      row = voiceRow(v);
      voiceRows.set(v.id, row);
      list.prepend(row);
    }
    row.refs.pause.textContent = v.paused ? 'Reprendre' : 'Pause';
    row.refs.loop.setAttribute('aria-pressed', String(v.loop));
    row.classList.toggle('is-paused', v.paused);
  }
  const n = engine.voices.size;
  $('voice-count').textContent = n ? String(n) : '';
  $('voices-empty').hidden = n > 0;
  refreshPads();
  startTicker();
}

let ticking = false;
function startTicker() {
  if (ticking || !engine.voices.size) return;
  ticking = true;
  requestAnimationFrame(tick);
}

function tick() {
  if (!engine.voices.size) {
    ticking = false;
    return;
  }
  const latest = new Map(); // sound id -> progress of its newest voice
  for (const v of engine.voices.values()) {
    const pos = engine.position(v);
    latest.set(v.soundId, v.duration ? pos / v.duration : 0);
    const r = voiceRows.get(v.id)?.refs;
    if (!r) continue;
    const time = `${L.formatTime(pos)} / ${L.formatTime(v.duration)}`;
    if (r.time.textContent !== time) r.time.textContent = time;
    if (!r.seeking) r.seek.value = String(Math.round((pos / v.duration) * 1000));
    // Hotkeys can change the volume behind the slider's back.
    if (document.activeElement !== r.vol && Number(r.vol.value) !== Math.round(v.volume * 100)) {
      r.vol.value = String(Math.round(v.volume * 100));
      r.volOut.textContent = pct(v.volume);
    }
  }
  for (const [id, p] of latest) pads.get(id)?.style.setProperty('--p', p.toFixed(4));
  requestAnimationFrame(tick);
}

function bindVoices() {
  const list = $('voices');
  const target = (e) => {
    const row = e.target.closest('.voice');
    return row && { row, id: Number(row.dataset.id), voice: engine.voices.get(Number(row.dataset.id)) };
  };
  list.addEventListener('input', (e) => {
    const t = target(e);
    if (!t?.voice) return;
    const { refs } = t.row;
    if (e.target === refs.vol) {
      engine.setVolume(t.id, refs.vol.value / 100);
      refs.volOut.textContent = pct(refs.vol.value / 100);
    } else if (e.target === refs.rate) {
      engine.setRate(t.id, refs.rate.value / 100);
      refs.rateOut.textContent = rateLabel(refs.rate.value / 100);
    } else if (e.target === refs.seek) refs.seeking = true;
  });
  list.addEventListener('change', (e) => {
    const t = target(e);
    if (!t?.voice || e.target !== t.row.refs.seek) return;
    engine.seek(t.id, (t.row.refs.seek.value / 1000) * t.voice.duration);
    t.row.refs.seeking = false;
  });
  list.addEventListener('click', (e) => {
    const t = target(e);
    if (!t?.voice) return;
    const b = e.target.closest('button');
    if (!b) return;
    if (b.classList.contains('voice-pause')) engine.setPaused(t.id, !t.voice.paused);
    else if (b.classList.contains('voice-loop')) engine.setLoop(t.id, !t.voice.loop);
    else if (b.classList.contains('voice-stop')) engine.stop(t.id);
    else if (b.classList.contains('voice-keep')) {
      const sound = soundById(t.voice.soundId);
      if (!sound) return;
      sound.volume = t.voice.volume;
      sound.rate = t.voice.rate;
      save();
      toast(`Réglages gardés pour « ${sound.title} » : ${pct(sound.volume)}, ${rateLabel(sound.rate)}`);
    }
  });
}

// ---------- Mixer ----------

function setSlider(id, value) {
  const input = $(id);
  input.value = String(Math.round(value * 100));
  input.nextElementSibling.textContent = pct(value);
}

function renderMixer() {
  const s = state.lib.settings;
  setSlider('vol-monitor', s.monitorVolume);
  setSlider('vol-cable', s.cableVolume);
  setSlider('vol-mic', s.micVolume);
  renderVoiceControls();
}

// With the filter, the voice already is the real microphone: the app leaves it alone.
function renderVoiceControls() {
  const filter = L.isMicFilter(state.lib.settings.cableDevice);
  for (const id of ['set-mic-enabled', 'set-mic', 'set-mic-ns', 'vol-mic']) $(id).disabled = filter;
  $('voice-hint').hidden = !filter;
}

function bindMixer() {
  const s = () => state.lib.settings;
  const bind = (id, apply) =>
    $(id).addEventListener('input', (e) => {
      const v = e.target.value / 100;
      e.target.nextElementSibling.textContent = pct(v);
      apply(v);
      save();
    });
  bind('vol-monitor', (v) => {
    s().monitorVolume = v;
    engine.setBusVolume('monitor', v);
  });
  bind('vol-cable', (v) => {
    s().cableVolume = v;
    engine.setBusVolume('cable', v);
  });
  bind('vol-mic', (v) => {
    s().micVolume = v;
    engine.setMicVolume(v);
  });

  const fill = $('meter-cable');
  const meter = fill.parentElement;
  setInterval(() => {
    if (document.hidden) return;
    const level = engine.buses.cable.enabled ? engine.level('cable') : 0;
    const db = 20 * Math.log10(Math.max(level, 1e-6));
    const p = Math.round(Math.min(1, Math.max(0, (db + 60) / 60)) * 100);
    fill.style.setProperty('--level', `${p}%`);
    fill.classList.toggle('is-hot', db > -1);
    meter.setAttribute('aria-valuenow', String(p));
  }, 66);
}

// ---------- Audio devices ----------

let labelsUnlocked = false;

async function setupAudio() {
  const s = state.lib.settings;
  if (!labelsUnlocked) {
    // Device names are only exposed once microphone access has been granted.
    try {
      for (const t of (await navigator.mediaDevices.getUserMedia({ audio: true })).getTracks()) t.stop();
    } catch {
      /* no microphone: outputs still work */
    }
    labelsUnlocked = true;
  }
  const all = await navigator.mediaDevices.enumerateDevices();
  const real = (d) => d.deviceId !== 'default' && d.deviceId !== 'communications';
  state.devices.outputs = all.filter((d) => d.kind === 'audiooutput' && real(d));
  // The cable's own capture side would loop the sounds back into themselves.
  state.devices.inputs = all.filter((d) => d.kind === 'audioinput' && real(d) && !/CABLE Output|VB-Audio/i.test(d.label));

  if (s.cableDevice === undefined) {
    await refreshFilter();
    const cable = L.findVirtualCable(state.devices.outputs);
    if (state.filter.status?.installed?.length) {
      s.cableDevice = { ...L.MIC_FILTER };
      save();
    } else if (cable) {
      s.cableDevice = { id: cable.deviceId, label: cable.label };
      save();
      toast(`Micro virtuel détecté : ${cable.label}`);
    }
  }
  try {
    await engine.setOutput('monitor', L.resolveDevice(state.devices.outputs, s.monitorDevice) ?? '');
  } catch (err) {
    toast(`Sortie casque inaccessible : ${errText(err)}`, true);
  }
  try {
    await engine.setOutput('cable', cableSink(s.cableDevice));
    if (L.isMicFilter(s.cableDevice)) refreshFilter();
  } catch (err) {
    await engine.setOutput('cable', null).catch(() => {});
    toast(`Envoi aux autres impossible : ${errText(err)}`, true);
  }
  engine.setBusVolume('monitor', s.monitorVolume);
  engine.setBusVolume('cable', s.cableVolume);
  await applyMic();
  renderBanner();
}

function cableSink(saved) {
  if (L.isMicFilter(saved)) return FILTER_SINK;
  return saved ? L.resolveDevice(state.devices.outputs, saved) : null;
}

async function applyMic() {
  const s = state.lib.settings;
  try {
    await engine.setMic({
      // With the filter the real microphone already carries the sounds: capturing it would loop them.
      enabled: s.micEnabled && engine.buses.cable.enabled && !L.isMicFilter(s.cableDevice),
      deviceId: L.resolveDevice(state.devices.inputs, s.micDevice) ?? '',
      noiseSuppression: s.micNoiseSuppression,
      volume: s.micVolume,
    });
  } catch (err) {
    toast(`Micro inaccessible : ${errText(err)}`, true);
  }
}

function renderBanner() {
  const s = state.lib.settings;
  const filter = L.isMicFilter(s.cableDevice);
  const cable = !!L.findVirtualCable(state.devices.outputs);
  let text = '';
  if (filter) {
    const status = state.filter.status;
    if (status?.ok && !status.installed.length) {
      text = 'Le filtre micro n’est pas installé : les sons ne sont entendus que par toi. Installe-le dans les réglages.';
    }
  } else if (!engine.buses.cable.enabled) {
    text = cable
      ? 'L’envoi aux autres est désactivé : les sons ne sont entendus que par toi.'
      : 'Les sons ne sont entendus que par toi. Dans les réglages, choisis « Mon micro, avec le filtre » (comme Soundpad), ou installe le câble virtuel VB-Cable.';
  }
  $('banner').hidden = !text;
  $('banner-text').textContent = text;
  $('banner-download').hidden = cable || filter;
}

// ---------- Microphone filter ----------

async function refreshFilter() {
  state.filter.status = await api.micFilter('status').catch((err) => ({ ok: false, error: errText(err) }));
  renderFilter();
  renderBanner();
}

function renderFilter() {
  const on = L.isMicFilter(state.lib.settings.cableDevice);
  const { status, live, busy } = state.filter;
  $('filter-box').hidden = !on;
  if (!on) return;
  $('filter-status').textContent = busy
    ? 'Windows demande l’autorisation dans sa fenêtre de contrôle de compte…'
    : L.micFilterMessage(status, live);
  const installed = status?.ok ? status.installed : [];
  $('filter-install').hidden = !(status?.ok && status.eligible.some((name) => !installed.includes(name)));
  $('filter-uninstall').hidden = !(status?.ok && (installed.length || status.dll));
  $('filter-install').disabled = busy;
  $('filter-uninstall').disabled = busy;
}

// Install and uninstall restart the Windows audio service, so the outputs are set up again after.
async function runFilterAction(action) {
  state.filter.busy = true;
  renderFilter();
  const result = await api.micFilter(action).catch((err) => ({ ok: false, error: errText(err) }));
  state.filter.busy = false;
  if (result.ok) {
    state.filter.status = result;
    toast(action === 'install' ? 'Filtre installé sur ton micro.' : 'Filtre retiré : le micro est revenu à son état d’avant.');
    await setupAudio().catch((err) => toast(`Audio : ${errText(err)}`, true));
  } else {
    toast(`Filtre micro : ${L.micFilterError(result.error)}`, true);
  }
  renderFilter();
  renderBanner();
}

// ---------- Hotkeys ----------

async function applyHotkeys() {
  try {
    const failed = await api.setHotkeys(L.buildHotkeys(state.lib));
    state.failedHotkeys = new Set(failed);
    if (failed.length) {
      toast(`Raccourci déjà pris par une autre application (Soundpad ouvert ?) : ${failed.map(L.formatAccelerator).join(', ')}`, true);
    }
  } catch (err) {
    toast(`Raccourcis globaux indisponibles : ${errText(err)}`, true);
  }
  renderGrid();
  renderSettingsHotkeys();
}

// Who already uses this hotkey, as a label, or null.
function hotkeyOwner(accel, { soundId = null, action = null } = {}) {
  const a = L.parseAccelerator(accel);
  const same = (b) => {
    const p = b && L.parseAccelerator(b);
    return !!p && p.vk === a.vk && p.mods === a.mods;
  };
  const s = state.lib.sounds.find((x) => x.id !== soundId && same(x.hotkey));
  if (s) return `« ${s.title} »`;
  for (const [k, v] of Object.entries(state.lib.settings.hotkeys)) if (k !== action && same(v)) return `« ${GLOBAL_ACTIONS[k]} »`;
  return null;
}

function renderHotkeyButton(btn, accel) {
  btn.textContent = accel ? L.formatAccelerator(accel) : 'Définir un raccourci';
  btn.classList.toggle('is-empty', !accel);
}

let capture = null;

// Waits for a key combination on `btn`. Global hotkeys are released meanwhile, otherwise
// Windows would swallow keys that are already registered.
function startCapture(btn, onDone) {
  capture?.cancel();
  btn.textContent = 'Appuie sur une touche (Échap : annuler)';
  btn.classList.add('is-capturing');
  api.setHotkeys([]).catch(() => {});
  const finish = (accel) => {
    window.removeEventListener('keydown', onKey, true);
    btn.removeEventListener('blur', cancel);
    btn.classList.remove('is-capturing');
    capture = null;
    onDone(accel); // undefined = cancelled
    applyHotkeys();
  };
  const cancel = () => finish(undefined);
  const onKey = (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') return cancel();
    const accel = L.eventToAccelerator(e);
    if (accel) finish(accel);
  };
  window.addEventListener('keydown', onKey, true);
  btn.addEventListener('blur', cancel);
  capture = { cancel };
}

// ---------- Sound editor ----------

let editing = null; // { sound, hotkey }

function openEditor(sound) {
  const dlg = $('dlg-sound');
  const F = $('form-sound').elements;
  editing = { sound, hotkey: sound.hotkey };
  F.namedItem('title').value = sound.title;
  F.namedItem('volume').value = String(Math.round(sound.volume * 100));
  F.namedItem('volumeOut').value = pct(sound.volume);
  F.namedItem('rate').value = String(Math.round(sound.rate * 100));
  F.namedItem('rateOut').value = rateLabel(sound.rate);
  F.namedItem('mode').value = sound.mode;
  F.namedItem('favorite').checked = sound.favorite;
  renderHotkeyButton($('sound-hotkey'), sound.hotkey);
  $('sound-hotkey-error').hidden = true;
  const box = $('sound-categories');
  box.replaceChildren();
  if (!state.lib.categories.length) box.append(el('p', 'muted', 'Aucune catégorie : crée-en une dans la colonne de gauche.'));
  for (const c of state.lib.categories) {
    const label = el('label', 'check');
    const cb = el('input');
    Object.assign(cb, { type: 'checkbox', value: c.id, checked: c.soundIds.includes(sound.id) });
    label.append(cb, ` ${c.name}`);
    box.append(label);
  }
  $('sound-path').textContent = state.missing.has(sound.id) ? `${sound.path} (introuvable)` : sound.path;
  dlg.returnValue = '';
  dlg.showModal();
}

function applyEditor() {
  const { sound, hotkey } = editing;
  const F = $('form-sound').elements;
  sound.title = F.namedItem('title').value.trim() || sound.title;
  sound.volume = F.namedItem('volume').value / 100;
  sound.rate = F.namedItem('rate').value / 100;
  sound.mode = F.namedItem('mode').value;
  sound.favorite = F.namedItem('favorite').checked;
  const hotkeyChanged = sound.hotkey !== hotkey;
  sound.hotkey = hotkey;
  for (const cb of $('sound-categories').querySelectorAll('input')) {
    const cat = state.lib.categories.find((c) => c.id === cb.value);
    const has = cat.soundIds.includes(sound.id);
    if (cb.checked && !has) cat.soundIds.push(sound.id);
    else if (!cb.checked && has) cat.soundIds = cat.soundIds.filter((id) => id !== sound.id);
  }
  save({ now: true });
  renderCategories();
  renderGrid();
  if (hotkeyChanged) applyHotkeys();
}

function removeSound(sound) {
  for (const v of [...engine.voices.values()]) if (v.soundId === sound.id) engine.stop(v.id);
  state.lib.sounds = state.lib.sounds.filter((s) => s !== sound);
  for (const c of state.lib.categories) c.soundIds = c.soundIds.filter((id) => id !== sound.id);
  engine.forget(sound.path);
  save({ now: true });
  renderCategories();
  renderGrid();
  if (sound.hotkey) applyHotkeys();
}

function bindEditor() {
  const dlg = $('dlg-sound');
  const F = $('form-sound').elements;
  F.namedItem('volume').addEventListener('input', (e) => (F.namedItem('volumeOut').value = pct(e.target.value / 100)));
  F.namedItem('rate').addEventListener('input', (e) => (F.namedItem('rateOut').value = rateLabel(e.target.value / 100)));
  $('sound-cancel').addEventListener('click', () => dlg.close('cancel'));
  dlg.addEventListener('close', () => {
    capture?.cancel();
    if (dlg.returnValue === 'save' && editing) applyEditor();
    editing = null;
  });
  $('sound-hotkey').addEventListener('click', (e) => {
    const btn = e.currentTarget;
    $('sound-hotkey-error').hidden = true;
    startCapture(btn, (accel) => {
      if (!editing) return;
      const owner = accel && hotkeyOwner(accel, { soundId: editing.sound.id });
      if (owner) {
        $('sound-hotkey-error').textContent = `${L.formatAccelerator(accel)} est déjà utilisé par ${owner}.`;
        $('sound-hotkey-error').hidden = false;
      } else if (accel) editing.hotkey = accel;
      renderHotkeyButton(btn, editing.hotkey);
    });
  });
  $('sound-hotkey-clear').addEventListener('click', () => {
    if (!editing) return;
    editing.hotkey = null;
    renderHotkeyButton($('sound-hotkey'), null);
  });
  $('sound-preview').addEventListener('click', () => {
    if (!editing) return;
    const { sound } = editing;
    trigger({ ...sound, volume: F.namedItem('volume').value / 100, rate: F.namedItem('rate').value / 100, mode: 'restart' });
  });
  $('sound-delete').addEventListener('click', () => {
    if (!editing) return;
    const { sound } = editing;
    if (!confirm(`Retirer « ${sound.title} » de la bibliothèque ?\nLe fichier audio reste sur le disque.`)) return;
    editing = null;
    dlg.close('deleted');
    removeSound(sound);
  });
}

// ---------- Settings ----------

function fillSelect(select, devices, current, first) {
  select.replaceChildren(...first.map(([label, value]) => new Option(label, value)));
  for (const d of devices) select.append(new Option(d.label || 'Périphérique sans nom', d.deviceId));
  select.value = current;
  if (select.value !== current) select.selectedIndex = 0;
}

function renderSettingsHotkeys() {
  for (const btn of document.querySelectorAll('#dlg-settings [data-action]')) {
    if (!btn.classList.contains('is-capturing')) renderHotkeyButton(btn, state.lib.settings.hotkeys[btn.dataset.action]);
  }
}

function renderSettings() {
  const s = state.lib.settings;
  const { outputs, inputs } = state.devices;
  fillSelect($('set-monitor'), outputs, L.resolveDevice(outputs, s.monitorDevice) ?? '', [['Sortie par défaut de Windows', '']]);
  const filter = L.isMicFilter(s.cableDevice);
  fillSelect($('set-cable'), outputs, filter ? L.MIC_FILTER.id : (s.cableDevice && L.resolveDevice(outputs, s.cableDevice)) ?? CABLE_OFF, [
    ['Désactivé', CABLE_OFF],
    ['Mon micro, avec le filtre (comme Soundpad)', L.MIC_FILTER.id],
  ]);
  fillSelect($('set-mic'), inputs, L.resolveDevice(inputs, s.micDevice) ?? '', [['Micro par défaut de Windows', '']]);
  $('set-mic-enabled').checked = s.micEnabled;
  $('set-mic-ns').checked = s.micNoiseSuppression;
  $('set-mode').value = s.defaultMode;
  $('set-exclusive').checked = s.exclusive;
  $('set-normalize').checked = s.normalize;
  $('set-max').value = String(s.maxVoices);
  $('settings-hotkey-error').hidden = true;
  renderSettingsHotkeys();
  const cable = L.findVirtualCable(outputs);
  $('cable-hint').textContent = filter
    ? 'Windows ajoute les sons à ton vrai micro : dans Discord, garde ton micro habituel. Coupe la suppression de bruit de Discord (Krisp), sinon elle efface la musique.'
    : cable
      ? 'Dans Discord : Paramètres, Voix et vidéo, Périphérique d’entrée : « CABLE Output (VB-Audio Virtual Cable) ». Coupe aussi la suppression de bruit de Discord, sinon elle efface la musique.'
      : 'Aucun câble virtuel trouvé. Choisis plutôt « Mon micro, avec le filtre », ou installe VB-Cable (gratuit, vb-audio.com), redémarre le PC, puis choisis ici « CABLE Input ».';
  renderVoiceControls();
  renderFilter();
  if (filter) refreshFilter();
}

function bindSettings() {
  const s = () => state.lib.settings;
  const dlg = $('dlg-settings');
  const deviceOf = (select) => ({ id: select.value, label: select.selectedOptions[0]?.textContent ?? '' });
  dlg.addEventListener('close', () => capture?.cancel());

  $('set-monitor').addEventListener('change', async (e) => {
    s().monitorDevice = deviceOf(e.target);
    save();
    await engine.setOutput('monitor', e.target.value).catch((err) => toast(`Sortie casque inaccessible : ${errText(err)}`, true));
  });
  $('set-cable').addEventListener('change', async (e) => {
    const value = e.target.value;
    s().cableDevice = value === CABLE_OFF ? null : value === L.MIC_FILTER.id ? { ...L.MIC_FILTER } : deviceOf(e.target);
    save();
    await engine.setOutput('cable', cableSink(s().cableDevice)).catch((err) => toast(`Envoi aux autres impossible : ${errText(err)}`, true));
    await applyMic();
    renderSettings();
    renderBanner();
  });
  $('filter-install').addEventListener('click', () => runFilterAction('install'));
  $('filter-uninstall').addEventListener('click', () => runFilterAction('uninstall'));
  $('set-mic-enabled').addEventListener('change', (e) => {
    s().micEnabled = e.target.checked;
    save();
    applyMic();
  });
  $('set-mic').addEventListener('change', (e) => {
    s().micDevice = deviceOf(e.target);
    save();
    applyMic();
  });
  $('set-mic-ns').addEventListener('change', (e) => {
    s().micNoiseSuppression = e.target.checked;
    save();
    applyMic();
  });
  $('set-mode').addEventListener('change', (e) => {
    s().defaultMode = e.target.value;
    save();
  });
  $('set-exclusive').addEventListener('change', (e) => {
    s().exclusive = e.target.checked;
    save();
  });
  $('set-normalize').addEventListener('change', (e) => {
    s().normalize = e.target.checked;
    engine.setNormalize(e.target.checked);
    save();
  });
  $('set-max').addEventListener('change', (e) => {
    const n = Math.round(Math.min(64, Math.max(1, Number(e.target.value) || 16)));
    e.target.value = String(n);
    s().maxVoices = n;
    engine.maxVoices = n;
    save();
  });

  dlg.addEventListener('click', (e) => {
    const err = $('settings-hotkey-error');
    const set = e.target.closest('[data-action]');
    const clear = e.target.closest('[data-clear]');
    if (set) {
      const action = set.dataset.action;
      err.hidden = true;
      startCapture(set, (accel) => {
        const owner = accel && hotkeyOwner(accel, { action });
        if (owner) {
          err.textContent = `${L.formatAccelerator(accel)} est déjà utilisé par ${owner}.`;
          err.hidden = false;
        } else if (accel) {
          s().hotkeys[action] = accel;
          save();
        }
        renderHotkeyButton(set, s().hotkeys[action]);
      });
    } else if (clear) {
      s().hotkeys[clear.dataset.clear] = null;
      save();
      applyHotkeys();
    }
  });
}

// ---------- Adding and importing ----------

async function addPaths(paths) {
  if (!paths.length) return;
  const known = new Set(state.lib.sounds.map((s) => s.path.toLowerCase()));
  const fresh = [];
  for (const p of paths) {
    if (known.has(p.toLowerCase())) continue;
    known.add(p.toLowerCase());
    fresh.push(L.newSound(p));
  }
  state.lib.sounds.push(...fresh);
  const cat = state.lib.categories.find((c) => c.id === state.view.category);
  if (cat) cat.soundIds.push(...fresh.map((s) => s.id));
  await save({ now: true });
  renderCategories();
  renderGrid();
  toast(fresh.length ? plural(fresh.length, 'son ajouté', 'sons ajoutés') : 'Ces sons sont déjà dans la bibliothèque');
}

async function importSoundpad() {
  let file;
  try {
    file = (await api.readSoundpadList(false)) ?? (await api.readSoundpadList(true));
  } catch (err) {
    toast(`Import impossible : ${errText(err)}`, true);
    return;
  }
  if (!file) return;
  const parsed = L.parseSoundpadList(file.text);
  if (!parsed.sounds.length) {
    toast(`Aucun son trouvé dans ${file.path}`, true);
    return;
  }
  const { added, skipped } = L.mergeSoundpadImport(state.lib, parsed);
  await save({ now: true });
  await checkMissing();
  renderCategories();
  await applyHotkeys();
  toast(`${plural(added, 'son importé', 'sons importés')}${skipped ? `, ${skipped} déjà présents` : ''}`);
  preloadHotkeySounds();
}

async function checkMissing() {
  const exists = await api.filesExist(state.lib.sounds.map((s) => s.path));
  state.missing = new Set(state.lib.sounds.filter((_, i) => !exists[i]).map((s) => s.id));
  renderGrid();
}

// Sounds bound to a hotkey are played blind, in game: decode them ahead of time.
async function preloadHotkeySounds() {
  for (const s of state.lib.sounds) {
    if (s.hotkey && !state.missing.has(s.id)) await engine.load(s.path).catch(() => {});
  }
}

// ---------- Wiring ----------

function bindUi() {
  $('search').addEventListener('input', (e) => {
    state.view.query = e.target.value;
    renderGrid();
  });
  $('sort').addEventListener('change', (e) => {
    state.view.sort = e.target.value;
    renderGrid();
  });
  document.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      $('search').focus();
      $('search').select();
    }
  });

  const pickAndAdd = async () => addPaths(await api.pickFiles());
  $('btn-add').addEventListener('click', pickAndAdd);
  $('empty-add').addEventListener('click', pickAndAdd);
  $('btn-import').addEventListener('click', importSoundpad);
  $('empty-import').addEventListener('click', importSoundpad);
  $('btn-stop-all').addEventListener('click', () => engine.stopAll());
  const openSettings = () => {
    renderSettings();
    $('dlg-settings').showModal();
  };
  $('btn-settings').addEventListener('click', openSettings);
  $('banner-settings').addEventListener('click', openSettings);
  $('banner-download').addEventListener('click', () => api.openVbCable());

  $('categories').addEventListener('click', (e) => {
    const remove = e.target.closest('[data-remove]');
    const pick = e.target.closest('[data-category]');
    if (remove) {
      const cat = state.lib.categories.find((c) => c.id === remove.dataset.remove);
      if (!cat || !confirm(`Supprimer la catégorie « ${cat.name} » ?\nSes sons restent dans la bibliothèque.`)) return;
      state.lib.categories = state.lib.categories.filter((c) => c !== cat);
      if (state.view.category === cat.id) state.view.category = 'all';
      save({ now: true });
    } else if (pick) {
      state.view.category = pick.dataset.category;
    } else return;
    renderCategories();
    renderGrid();
  });
  $('new-category').addEventListener('submit', (e) => {
    e.preventDefault();
    const input = $('new-category-name');
    const name = input.value.trim();
    if (!name) return;
    const existing = state.lib.categories.find((c) => c.name.toLowerCase() === name.toLowerCase());
    if (existing) state.view.category = existing.id;
    else {
      const cat = { id: crypto.randomUUID(), name, soundIds: [] };
      state.lib.categories.push(cat);
      state.view.category = cat.id;
      save({ now: true });
    }
    input.value = '';
    renderCategories();
    renderGrid();
  });

  const grid = $('grid');
  const soundOf = (e) => {
    const pad = e.target.closest('.pad');
    return pad && soundById(pad.dataset.id);
  };
  grid.addEventListener('click', (e) => {
    const sound = soundOf(e);
    if (!sound) return;
    if (e.target.closest('.pad-edit')) openEditor(sound);
    else if (e.target.closest('.pad-play')) trigger(sound);
  });
  grid.addEventListener('contextmenu', (e) => {
    const sound = soundOf(e);
    if (!sound) return;
    e.preventDefault();
    openEditor(sound);
  });
  // Decode on hover so the click plays at once.
  grid.addEventListener('pointerover', (e) => {
    const pad = e.target.closest('.pad');
    if (!pad || pad.dataset.warm) return;
    pad.dataset.warm = '1';
    const sound = soundById(pad.dataset.id);
    if (sound && !state.missing.has(sound.id)) engine.load(sound.path).catch(() => {});
  });

  let dragDepth = 0;
  const hint = $('drop-hint');
  window.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    dragDepth++;
    hint.hidden = false;
  });
  window.addEventListener('dragleave', () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (!dragDepth) hint.hidden = true;
  });
  window.addEventListener('dragover', (e) => e.preventDefault());
  window.addEventListener('drop', async (e) => {
    e.preventDefault();
    dragDepth = 0;
    hint.hidden = true;
    const paths = [...e.dataTransfer.files].map((f) => api.pathForFile(f)).filter(Boolean);
    addPaths(await api.expandPaths(paths));
  });

  bindVoices();
  bindMixer();
  bindEditor();
  bindSettings();
  engine.addEventListener('change', onVoicesChanged);
  api.onHotkey(onHotkey);
  api.onMicState((live) => {
    state.filter.live = live;
    renderFilter();
  });
  let deviceTimer = 0;
  navigator.mediaDevices.addEventListener('devicechange', () => {
    clearTimeout(deviceTimer);
    deviceTimer = setTimeout(() => setupAudio().catch((err) => toast(errText(err), true)), 500);
  });
}

async function init() {
  const { data, warning } = await api.loadLibrary();
  state.lib = L.normalizeLibrary(data);
  if (warning) toast(warning, true);
  engine.normalize = state.lib.settings.normalize;
  engine.maxVoices = state.lib.settings.maxVoices;
  if (!state.lib.sounds.length) {
    const file = await api.readSoundpadList(false).catch(() => null);
    if (file) state.soundpadCount = L.parseSoundpadList(file.text).sounds.length;
  }
  bindUi();
  renderCategories();
  renderMixer();
  onVoicesChanged();
  await checkMissing();
  await setupAudio().catch((err) => toast(`Audio : ${errText(err)}`, true));
  await applyHotkeys();
  preloadHotkeySounds();
}

init().catch((err) => toast(`Démarrage impossible : ${errText(err)}`, true));
