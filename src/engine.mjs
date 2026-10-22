// Audio engine: any number of overlapping voices, each with its own live volume, speed,
// loop, pause and position, rendered to two outputs at once:
//   monitor: what you hear (headset)
//   cable:   what others hear: either a virtual cable used as microphone, plus your real mic,
//            or FILTER_SINK, where the PCM goes to onPcm for the microphone filter (native/).
import { PlayheadClock, analyse, normalizeGain } from './lib.mjs';

export const FILTER_SINK = 'filter';
const CACHE_LIMIT_BYTES = 768 * 1024 * 1024; // decoded PCM is large: ~23 MB per stereo minute
const FADE_S = 0.03;
const now = () => performance.now() / 1000;

function makeBus(sampleRate) {
  const ctx = new AudioContext({ latencyHint: 'interactive', sampleRate });
  const master = ctx.createGain(); // volume of the sounds on this output
  // Safety limiter, so stacked sounds squash instead of clipping. The mic joins after the
  // master gain so the sound volume never changes your voice.
  const limiter = ctx.createDynamicsCompressor();
  limiter.threshold.value = -3;
  limiter.knee.value = 0;
  limiter.ratio.value = 20;
  limiter.attack.value = 0.002;
  limiter.release.value = 0.15;
  const analyser = ctx.createAnalyser();
  analyser.fftSize = 1024;
  master.connect(limiter).connect(ctx.destination);
  limiter.connect(analyser);
  return { ctx, master, limiter, analyser, samples: new Float32Array(analyser.fftSize), enabled: true };
}

export class Engine extends EventTarget {
  constructor(readFile, onPcm) {
    super();
    this.readFile = readFile;
    this.onPcm = onPcm;
    // The microphone filter's ring runs at 48 kHz (native/shared.h).
    this.buses = { monitor: makeBus(), cable: makeBus(48000) };
    this.decoder = new OfflineAudioContext(2, 1, 48000);
    this.cache = new Map(); // path -> Promise<{ buffer, gain, bytes }>, oldest first
    this.sizes = new Map(); // path -> bytes, decoded entries only
    this.voices = new Map(); // id -> voice, oldest first
    this.nextId = 1;
    this.stopGeneration = 0;
    this.normalize = true;
    this.maxVoices = 16;
    this.mic = null;
  }

  // ---------- Outputs and microphone ----------

  // deviceId: '' = Windows default, null = output off, FILTER_SINK = no device, PCM to onPcm.
  async setOutput(name, deviceId) {
    const bus = this.buses[name];
    const filter = deviceId === FILTER_SINK;
    await bus.ctx.setSinkId(deviceId === null || filter ? { type: 'none' } : deviceId);
    bus.enabled = deviceId !== null;
    await this.setTap(bus, filter);
    if (bus.ctx.state === 'suspended') await bus.ctx.resume();
  }

  // bus.tap holds a promise, so overlapping calls never build two taps (twice the PCM).
  async setTap(bus, on) {
    if (on) {
      bus.tap ??= this.makeTap(bus);
      try {
        await bus.tap;
      } catch (err) {
        bus.tap = null;
        throw err;
      }
      return;
    }
    const pending = bus.tap;
    bus.tap = null;
    const tap = await pending?.catch(() => null);
    if (!tap) return;
    tap.port.onmessage = null;
    bus.limiter.disconnect(tap);
    tap.disconnect();
  }

  async makeTap(bus) {
    await bus.ctx.audioWorklet.addModule(new URL('./pcm-tap.js', import.meta.url));
    const tap = new AudioWorkletNode(bus.ctx, 'pcm-tap', {
      channelCount: 2,
      channelCountMode: 'explicit',
      outputChannelCount: [1],
    });
    tap.port.onmessage = (e) => this.onPcm(e.data);
    bus.limiter.connect(tap).connect(bus.ctx.destination); // connected so the graph pulls it
    return tap;
  }

  setBusVolume(name, volume) {
    const { ctx, master } = this.buses[name];
    master.gain.setTargetAtTime(volume, ctx.currentTime, 0.02);
  }

  // Peak level (0..1) of what currently leaves an output.
  level(name) {
    const { analyser, samples } = this.buses[name];
    analyser.getFloatTimeDomainData(samples);
    let peak = 0;
    for (const v of samples) peak = Math.max(peak, Math.abs(v));
    return peak;
  }

  async setMic({ enabled, deviceId, noiseSuppression, volume }) {
    this.stopMic();
    if (!enabled) return;
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        deviceId: deviceId ? { exact: deviceId } : undefined,
        echoCancellation: false,
        autoGainControl: false,
        noiseSuppression: !!noiseSuppression,
      },
    });
    const { ctx, limiter } = this.buses.cable;
    const source = ctx.createMediaStreamSource(stream);
    const gain = ctx.createGain();
    gain.gain.value = volume;
    source.connect(gain).connect(limiter);
    this.mic = { stream, source, gain };
  }

  stopMic() {
    if (!this.mic) return;
    this.mic.source.disconnect();
    this.mic.gain.disconnect();
    for (const t of this.mic.stream.getTracks()) t.stop();
    this.mic = null;
  }

  setMicVolume(volume) {
    if (this.mic) this.mic.gain.gain.setTargetAtTime(volume, this.buses.cable.ctx.currentTime, 0.02);
  }

  // ---------- Decoding cache ----------

  load(path) {
    const hit = this.cache.get(path);
    if (hit) {
      this.cache.delete(path); // move to the most recent end
      this.cache.set(path, hit);
      return hit;
    }
    const entry = this.decode(path);
    this.cache.set(path, entry);
    entry.then(
      (e) => {
        this.sizes.set(path, e.bytes);
        this.evict();
      },
      () => this.forget(path),
    );
    return entry;
  }

  async decode(path) {
    const bytes = await this.readFile(path);
    const data = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    const buffer = await this.decoder.decodeAudioData(data);
    const channels = Array.from({ length: buffer.numberOfChannels }, (_, c) => buffer.getChannelData(c));
    return {
      buffer,
      gain: normalizeGain(analyse(channels, buffer.sampleRate)),
      bytes: buffer.length * buffer.numberOfChannels * 4,
    };
  }

  evict() {
    let total = 0;
    for (const b of this.sizes.values()) total += b;
    for (const path of this.cache.keys()) {
      if (total <= CACHE_LIMIT_BYTES) break;
      if (!this.sizes.has(path)) continue; // still decoding
      total -= this.sizes.get(path);
      this.forget(path); // voices already playing keep their own reference
    }
  }

  forget(path) {
    this.cache.delete(path);
    this.sizes.delete(path);
  }

  // ---------- Voices ----------

  // Starts a new voice for `sound` ({ id, path, title, volume, rate }). Resolves to the voice,
  // or null when a "stop all" happened while the file was decoding.
  async play(sound) {
    const generation = this.stopGeneration;
    const entry = await this.load(sound.path);
    if (generation !== this.stopGeneration) return null;
    while (this.voices.size >= this.maxVoices) this.stop(this.voices.keys().next().value);
    const voice = {
      id: this.nextId++,
      soundId: sound.id,
      title: sound.title,
      buffer: entry.buffer,
      normGain: entry.gain,
      duration: entry.buffer.duration,
      volume: sound.volume,
      rate: sound.rate,
      loop: false,
      paused: false,
      clock: new PlayheadClock(now(), sound.rate),
      nodes: [],
    };
    this.voices.set(voice.id, voice);
    this.startNodes(voice, 0);
    this.changed();
    return voice;
  }

  gainOf(voice) {
    return voice.volume * (this.normalize ? voice.normGain : 1);
  }

  startNodes(voice, offset) {
    voice.nodes = [];
    for (const bus of Object.values(this.buses)) {
      if (!bus.enabled) continue;
      const { ctx } = bus;
      const src = ctx.createBufferSource();
      src.buffer = voice.buffer;
      src.loop = voice.loop;
      src.playbackRate.value = voice.rate;
      const gain = ctx.createGain();
      gain.gain.value = this.gainOf(voice);
      src.connect(gain).connect(bus.master);
      src.onended = () => this.finish(voice);
      src.start(0, offset);
      voice.nodes.push({ ctx, src, gain });
    }
    voice.clock.seek(offset, now());
    voice.clock.resume(now());
  }

  // Stops the voice's nodes (with a short fade, no click) without forgetting the voice.
  haltNodes(voice) {
    for (const { ctx, src, gain } of voice.nodes) {
      const t = ctx.currentTime;
      src.onended = () => {
        src.disconnect();
        gain.disconnect();
      };
      gain.gain.cancelScheduledValues(t);
      gain.gain.setValueAtTime(gain.gain.value, t);
      gain.gain.linearRampToValueAtTime(0, t + FADE_S);
      src.stop(t + FADE_S);
    }
    voice.nodes = [];
  }

  // Natural end of a non-looping voice.
  finish(voice) {
    if (!this.voices.has(voice.id)) return;
    for (const { src, gain } of voice.nodes) {
      src.disconnect();
      gain.disconnect();
    }
    voice.nodes = [];
    this.voices.delete(voice.id);
    this.changed();
  }

  position(voice) {
    const p = voice.clock.position(now());
    return voice.loop ? p % voice.duration : Math.min(p, voice.duration);
  }

  stop(id) {
    const voice = this.voices.get(id);
    if (!voice) return;
    this.haltNodes(voice);
    this.voices.delete(id);
    this.changed();
  }

  stopAll() {
    this.stopGeneration++;
    for (const id of [...this.voices.keys()]) this.stop(id);
  }

  setPaused(id, paused) {
    const voice = this.voices.get(id);
    if (!voice || voice.paused === paused) return;
    if (paused) {
      voice.clock.pause(now());
      this.haltNodes(voice);
    } else {
      this.startNodes(voice, this.position(voice));
    }
    voice.paused = paused;
    this.changed();
  }

  // Pauses everything, or resumes everything when all voices are already paused.
  togglePauseAll() {
    const voices = [...this.voices.values()];
    const pause = voices.some((v) => !v.paused);
    for (const v of voices) this.setPaused(v.id, pause);
  }

  seek(id, seconds) {
    const voice = this.voices.get(id);
    if (!voice) return;
    const pos = Math.min(Math.max(0, seconds), Math.max(0, voice.duration - 0.01));
    if (voice.paused) {
      voice.clock.seek(pos, now());
      return;
    }
    this.haltNodes(voice);
    this.startNodes(voice, pos);
  }

  setVolume(id, volume) {
    const voice = this.voices.get(id);
    if (!voice) return;
    voice.volume = volume;
    for (const { ctx, gain } of voice.nodes) gain.gain.setTargetAtTime(this.gainOf(voice), ctx.currentTime, 0.015);
  }

  // Nudges every playing voice, for the in-game volume hotkeys.
  nudgeVolumes(delta) {
    for (const v of this.voices.values()) this.setVolume(v.id, Math.min(2, Math.max(0, Math.round((v.volume + delta) * 100) / 100)));
  }

  setRate(id, rate) {
    const voice = this.voices.get(id);
    if (!voice) return;
    voice.clock.setRate(rate, now());
    voice.rate = rate;
    for (const { ctx, src } of voice.nodes) src.playbackRate.setValueAtTime(rate, ctx.currentTime);
  }

  setLoop(id, loop) {
    const voice = this.voices.get(id);
    if (!voice) return;
    voice.clock.seek(this.position(voice), now()); // fold past loops back into [0, duration)
    voice.loop = loop;
    for (const { src } of voice.nodes) src.loop = loop;
    this.changed();
  }

  setNormalize(on) {
    this.normalize = on;
    for (const v of this.voices.values()) this.setVolume(v.id, v.volume);
  }

  changed() {
    this.dispatchEvent(new Event('change'));
  }
}
