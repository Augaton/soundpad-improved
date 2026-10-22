import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as L from '../src/lib.mjs';

test('cleanTitle strips the Voicemod suffix and slug dashes', () => {
  assert.equal(L.cleanTitle('kirby-falling-meme-scream-made-with-Voicemod'), 'kirby falling meme scream');
  assert.equal(L.cleanTitle('3fs-fart-(slow)-made-with-Voicemod'), '3fs fart (slow)');
  assert.equal(L.cleanTitle('El Azhar - El Djihad'), 'El Azhar - El Djihad');
  assert.equal(L.cleanTitle('made-with-Voicemod'), 'made-with-Voicemod');
  assert.equal(L.titleFromPath('C:\\soundpad\\ba dum tss.mp3'), 'ba dum tss');
  assert.equal(L.titleFromPath('/x/y/file.name.ogg'), 'file.name');
});

test('time helpers', () => {
  assert.equal(L.formatTime(0), '0:00');
  assert.equal(L.formatTime(203.9), '3:23');
  assert.equal(L.formatTime(NaN), '0:00');
  assert.equal(L.parseDuration('3:23'), 203);
  assert.equal(L.parseDuration('1:02:03'), 3723);
  assert.equal(L.parseDuration('abc'), null);
  assert.equal(L.searchKey('Même PAS'), 'meme pas');
});

test('virtual-key codes become hotkey strings', () => {
  assert.equal(L.vkToAccelerator(19, 0), 'Pause');
  assert.equal(L.vkToAccelerator(97, 6), 'Ctrl+Shift+num1');
  assert.equal(L.vkToAccelerator(65, 1), 'Alt+A');
  assert.equal(L.vkToAccelerator(124, 8), 'Super+F13');
  assert.equal(L.vkToAccelerator(255, 0), null);
});

test('parseAccelerator inverts vkToAccelerator for every key and modifier set', () => {
  for (let vk = 1; vk < 256; vk++) {
    if (!L.vkToKey(vk)) continue;
    for (let mods = 0; mods < 16; mods++) {
      assert.deepEqual(L.parseAccelerator(L.vkToAccelerator(vk, mods)), { vk, mods }, `vk ${vk} mods ${mods}`);
    }
  }
  assert.equal(L.parseAccelerator('Ctrl+Nope'), null);
  assert.equal(L.parseAccelerator('Hyper+A'), null);
  assert.equal(L.parseAccelerator(''), null);
  assert.equal(L.parseAccelerator(null), null);
});

test('eventToAccelerator uses the Windows key code and ignores lone modifiers', () => {
  const ev = (key, keyCode, mods = {}) => ({ key, keyCode, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods });
  assert.equal(L.eventToAccelerator(ev('Control', 17, { ctrlKey: true })), null);
  assert.equal(L.eventToAccelerator(ev('a', 65, { ctrlKey: true })), 'Ctrl+A');
  assert.equal(L.eventToAccelerator(ev('&', 49)), '1'); // AZERTY top row
  assert.equal(L.eventToAccelerator(ev('Pause', 19)), 'Pause');
  assert.equal(L.eventToAccelerator(ev('1', 97, { shiftKey: true, altKey: true })), 'Alt+Shift+num1');
  assert.equal(L.eventToAccelerator(ev('Unidentified', 0)), null);
  assert.equal(L.formatAccelerator('Ctrl+Shift+num1'), 'Ctrl + Maj + Num 1');
  assert.equal(L.formatAccelerator('numadd'), 'Num +');
});

test('buildHotkeys lists sounds and global actions, skipping invalid strings', () => {
  const lib = L.normalizeLibrary({
    sounds: [
      { id: 'a', path: 'C:\\a.mp3', hotkey: 'Pause' },
      { id: 'b', path: 'C:\\b.mp3', hotkey: 'Bogus+Key' },
      { id: 'c', path: 'C:\\c.mp3' },
    ],
    settings: { hotkeys: { stopAll: 'Ctrl+num0' } },
  });
  assert.deepEqual(L.buildHotkeys(lib), [
    { accel: 'Pause', vk: 19, mods: 0, action: 'play:a' },
    { accel: 'Ctrl+num0', vk: 96, mods: 2, action: 'stopAll' },
  ]);
});

const sine = (amp, seconds, rate = 48000) => Float32Array.from({ length: seconds * rate }, (_, i) => amp * Math.sin((2 * Math.PI * 440 * i) / rate));

test('analyse measures peak and gated RMS, ignoring silence', () => {
  const tone = sine(0.5, 1);
  const a = L.analyse([tone, tone], 48000);
  assert.ok(Math.abs(a.peak - 0.5) < 1e-3);
  assert.ok(Math.abs(a.rms - 0.5 / Math.SQRT2) < 2e-3);
  const padded = new Float32Array(48000 * 4);
  padded.set(tone, 48000);
  assert.ok(Math.abs(L.analyse([padded], 48000).rms - a.rms) < 2e-3, 'silence must not lower loudness');
  assert.deepEqual(L.analyse([new Float32Array(1000)], 48000), { peak: 0, rms: 0 });
  assert.deepEqual(L.analyse([], 48000), { peak: 0, rms: 0 });
});

test('normalizeGain targets -18 dBFS without clipping, within +12 dB', () => {
  const target = 10 ** (-18 / 20);
  assert.ok(Math.abs(L.normalizeGain({ peak: 0.5, rms: target }) - 1) < 1e-9);
  assert.ok(L.normalizeGain({ peak: 1, rms: 0.5 }) < 0.3); // loud meme sound is turned down
  assert.equal(L.normalizeGain({ peak: 0.8, rms: 0.01 }), 1 / 0.8); // quiet but peaky: peak-limited
  assert.equal(L.normalizeGain({ peak: 0.01, rms: 0.001 }), 4); // capped at +12 dB
  assert.equal(L.normalizeGain({ peak: 0, rms: 0 }), 1);
});

test('planPlay covers every mode', () => {
  assert.deepEqual(L.planPlay('overlap', [1], [1, 2], false), { stop: [], play: true });
  assert.deepEqual(L.planPlay('restart', [1], [1, 2], false), { stop: [1], play: true });
  assert.deepEqual(L.planPlay('toggle', [1], [1, 2], false), { stop: [1], play: false });
  assert.deepEqual(L.planPlay('toggle', [], [2], false), { stop: [], play: true });
  assert.deepEqual(L.planPlay('overlap', [], [2, 3], true), { stop: [2, 3], play: true });
  assert.deepEqual(L.planPlay('toggle', [3], [2, 3], true), { stop: [3], play: false });
  assert.equal(L.effectiveMode({ mode: 'default' }, { defaultMode: 'restart' }), 'restart');
  assert.equal(L.effectiveMode({ mode: 'toggle' }, { defaultMode: 'restart' }), 'toggle');
});

test('PlayheadClock follows rate changes, pauses and seeks', () => {
  const c = new L.PlayheadClock(10, 1);
  assert.equal(c.position(12), 2);
  c.setRate(2, 12);
  assert.equal(c.position(13), 4);
  c.pause(13);
  assert.equal(c.position(100), 4);
  c.resume(100);
  assert.equal(c.position(101), 6);
  c.seek(1, 101);
  assert.equal(c.position(101.5), 2);
});

test('resolveDevice falls back to the label when the id changed', () => {
  const devices = [
    { deviceId: 'x1', label: 'Casque (TONOR)' },
    { deviceId: 'x2', label: 'CABLE Input (VB-Audio Virtual Cable)' },
  ];
  assert.equal(L.resolveDevice(devices, { id: 'x1', label: 'whatever' }), 'x1');
  assert.equal(L.resolveDevice(devices, { id: 'old', label: 'Casque (TONOR)' }), 'x1');
  assert.equal(L.resolveDevice(devices, { id: '', label: '' }), '');
  assert.equal(L.resolveDevice(devices, { id: 'gone', label: 'gone' }), null);
  assert.equal(L.resolveDevice(devices, null), null);
  assert.equal(L.findVirtualCable(devices).deviceId, 'x2');
  assert.equal(L.findVirtualCable(devices.slice(0, 1)), null);
});

test('the microphone filter is a saved choice, not a device', () => {
  const lib = L.normalizeLibrary({ settings: { cableDevice: L.MIC_FILTER } });
  assert.ok(L.isMicFilter(lib.settings.cableDevice));
  assert.ok(!L.isMicFilter({ id: 'x1', label: 'CABLE Input' }));
  assert.ok(!L.isMicFilter(null));
});

test('micFilterMessage follows install state, then what the feeder sees', () => {
  const mic = 'Microphone (TONOR TD510 Dynamic Mic)';
  const base = { ok: true, dll: false, installed: [], soundpad: [], eligible: [mic] };
  assert.match(L.micFilterMessage(null, null), /Vérification/);
  assert.match(L.micFilterMessage({ ok: false, error: 'cancelled' }, null), /Autorisation refusée/);
  assert.match(L.micFilterMessage({ ...base, eligible: [] }, null), /Aucun micro compatible/);
  const fresh = L.micFilterMessage(base, null);
  assert.match(fresh, /non installé.*TONOR/);
  assert.doesNotMatch(fresh, /Soundpad/);
  assert.match(L.micFilterMessage({ ...base, soundpad: [mic] }, null), /remplace celui de Soundpad/);
  const on = { ...base, dll: true, installed: [mic] };
  assert.match(L.micFilterMessage(on, 'LIVE 48000'), /^Actif sur « Microphone \(TONOR/);
  assert.match(L.micFilterMessage(on, 'IDLE'), /Aucune appli/);
  assert.match(L.micFilterMessage(on, 'NOMAP'), /dès qu’une appli/);
  assert.equal(L.micFilterError('not-built').includes('build.cmd'), true);
  assert.equal(L.micFilterError('weird'), 'weird');
});

test('normalizeLibrary keeps valid data and repairs the rest', () => {
  const empty = L.normalizeLibrary(null);
  assert.deepEqual(empty.sounds, []);
  assert.equal(empty.settings.cableDevice, undefined);
  assert.equal(empty.settings.defaultMode, 'overlap');

  const lib = L.normalizeLibrary({
    sounds: [
      { id: 'a', path: 'C:\\a.mp3', volume: 9, rate: 0.1, mode: 'weird', plays: -3, title: '' },
      { id: 'b' },
      'junk',
    ],
    categories: [{ id: 'c', name: 'Mes sons', soundIds: ['a', 'ghost'] }, { name: 'no id' }],
    settings: { cableDevice: null, maxVoices: 500, hotkeys: { stopAll: 'Pause', bogus: 'F1' } },
  });
  assert.equal(lib.sounds.length, 1);
  const [a] = lib.sounds;
  assert.equal(a.volume, 2);
  assert.equal(a.rate, 0.5);
  assert.equal(a.mode, 'default');
  assert.equal(a.plays, 0);
  assert.equal(a.title, 'a');
  assert.deepEqual(lib.categories, [{ id: 'c', name: 'Mes sons', soundIds: ['a'] }]);
  assert.equal(lib.settings.cableDevice, null);
  assert.equal(lib.settings.maxVoices, 64);
  assert.deepEqual(lib.settings.hotkeys, { stopAll: 'Pause', pauseAll: null, volUp: null, volDown: null });
});

const SPL = `<?xml version="1.0" encoding="UTF-8"?>
<Soundlist>
  <Sound hash="1" url="C:\\soundpad\\ba dum tss.mp3" artist="" title="ba dum tss" duration="0:02" addedOn="2026-03-09"/>
  <Sound hash="2" url="C:\\soundpad\\kirby-made-with-Voicemod.mp3" artist="" title="kirby-made-with-Voicemod" duration="0:06"/>
  <Sound hash="3" url="C:\\soundpad\\Tom &amp; Jerry.mp3" artist="" title="Tom &amp; Jerry &#233;t&#xE9;" duration="3:23" keyModifiers="0" key="19"/>
  <Categories>
    <Category type="1" icon="stock_icon_cat_list" hidden="true"/>
    <Category name="Mes sons" icon="stock_icon_cat_home">
      <Sound id="3"/>
      <Sound id="1"/>
      <Sound id="99"/>
      <Category name="Memes">
        <Sound id="2"/>
      </Category>
    </Category>
  </Categories>
  <Hotbar>
    <HotbarPage/>
  </Hotbar>
</Soundlist>`;

test('parseSoundpadList reads sounds, hotkeys and nested categories', () => {
  const parsed = L.parseSoundpadList(SPL);
  assert.deepEqual(parsed.sounds, [
    { path: 'C:\\soundpad\\ba dum tss.mp3', title: 'ba dum tss', hotkey: null, duration: 2 },
    { path: 'C:\\soundpad\\kirby-made-with-Voicemod.mp3', title: 'kirby', hotkey: null, duration: 6 },
    { path: 'C:\\soundpad\\Tom & Jerry.mp3', title: 'Tom & Jerry été', hotkey: 'Pause', duration: 203 },
  ]);
  assert.deepEqual(parsed.categories, [
    { name: 'Mes sons', indexes: [2, 0] },
    { name: 'Mes sons / Memes', indexes: [1] },
  ]);
});

test('mergeSoundpadImport never duplicates sounds, categories or hotkeys', () => {
  const lib = L.normalizeLibrary({
    sounds: [{ id: 'mine', path: 'c:\\SOUNDPAD\\ba dum tss.mp3', hotkey: null }, { id: 'k', path: 'C:\\k.mp3', hotkey: 'Pause' }],
  });
  const parsed = L.parseSoundpadList(SPL);
  assert.deepEqual(L.mergeSoundpadImport(lib, parsed), { added: 2, skipped: 1 });
  assert.equal(lib.sounds.length, 4);
  const tom = lib.sounds.find((s) => s.title === 'Tom & Jerry été');
  assert.equal(tom.hotkey, null, 'Pause already belongs to another sound');
  const mesSons = lib.categories.find((c) => c.name === 'Mes sons');
  assert.deepEqual(mesSons.soundIds, [tom.id, 'mine']);

  assert.deepEqual(L.mergeSoundpadImport(lib, parsed), { added: 0, skipped: 3 });
  assert.equal(lib.categories.length, 2);
  assert.deepEqual(lib.categories.find((c) => c.name === 'Mes sons').soundIds, [tom.id, 'mine']);
});
