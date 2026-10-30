// The audio engine: decoding cache, the two outputs, the microphone, and the filter feed.
// cpal streams cannot leave the thread that made them, so one "audio" thread owns them all;
// the rest of the app talks to it through a channel and to the voices through the mixer.
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, StreamConfig};
use nnnoiseless::DenoiseState;

use super::decode::{self, Pcm, RATE};
use super::mixer::{BUSES, CABLE, MONITOR, Mixer, VoiceInfo};
use super::ring::{self, Ring};
use crate::model::{Cable, Device, FilterLive, is_cable_capture, resolve_device};

const CACHE_LIMIT_BYTES: usize = 768 * 1024 * 1024; // decoded PCM is large: ~11 MB per minute
const LIMIT: f32 = 0.708; // safety limiter threshold, -3 dBFS
const POLL: Duration = Duration::from_secs(2); // device list check
const FILTER_BLOCK: usize = 480; // 10 ms at 48 kHz

pub enum Event {
    Started { sound_id: String, duration: f64 },
    Error(String),
    Changed, // outputs, devices or the filter's state
}

pub type Events = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Clone, PartialEq, Debug)]
pub struct AudioConfig {
    pub monitor: Device,
    pub cable: Cable,
    pub mic: Device,
    pub mic_enabled: bool,
    pub noise_suppression: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Status {
    pub outputs: Vec<String>,
    pub inputs: Vec<String>, // virtual cables' capture side left out: it would loop the sounds back
    pub running: [bool; BUSES],
    pub mic: bool,
}

pub struct PlayRequest {
    pub sound_id: String,
    pub path: String,
    pub title: String,
    pub volume: f32,
    pub rate: f32,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------- Decoding cache ----------

type Decoded = Arc<OnceLock<Result<Arc<Pcm>, String>>>;

#[derive(Default)]
struct Cache {
    entries: HashMap<String, (Decoded, u64)>, // path -> (result, last use)
    tick: u64,
}

impl Cache {
    fn get(&mut self, path: &str) -> Decoded {
        self.tick += 1;
        let e = self.entries.entry(path.to_string()).or_default();
        e.1 = self.tick;
        e.0.clone()
    }

    /// After a decode: failures are dropped (a later try decodes again), and the least recently
    /// used sounds go while the total is too large. Voices playing keep their own reference.
    fn settle(&mut self) {
        self.entries
            .retain(|_, (cell, _)| !matches!(cell.get(), Some(Err(_))));
        let size = |cell: &Decoded| match cell.get() {
            Some(Ok(p)) => p.bytes(),
            _ => 0, // still decoding
        };
        let mut total: usize = self.entries.values().map(|(c, _)| size(c)).sum();
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .filter(|(_, (c, _))| size(c) > 0)
            .map(|(k, (_, t))| (*t, k.clone()))
            .collect();
        by_age.sort();
        for (_, path) in by_age {
            if total <= CACHE_LIMIT_BYTES {
                break;
            }
            total -= size(&self.entries[&path].0);
            self.entries.remove(&path);
        }
    }
}

// ---------- Engine ----------

enum Command {
    Configure(AudioConfig),
    Reset, // reopen everything: the Windows audio service restarted
    Quit,
}

#[derive(Clone)]
struct Shared {
    mixer: Arc<Mutex<Mixer>>,
    levels: Arc<[AtomicU32; BUSES]>, // peak since last read, as f32 bits
    filter_live: Arc<AtomicU32>,     // 0 not loaded, 1 idle, else the microphone's rate
    status: Arc<Mutex<Status>>,
    failed: [Arc<AtomicBool>; 3], // monitor, cable, microphone stream errors
    mic: Arc<MicPipe>,
    events: Events,
}

impl Shared {
    fn set_running(&self, bus: usize, running: bool) {
        lock(&self.mixer).set_running(bus, running);
    }

    fn error(&self, message: String) {
        (self.events)(Event::Error(message));
    }
}

pub struct Engine {
    shared: Shared,
    cache: Mutex<Cache>,
    commands: Sender<Command>,
    preload: Sender<String>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    pub fn new(events: Events) -> Arc<Engine> {
        let shared = Shared {
            mixer: Arc::default(),
            levels: Arc::new([AtomicU32::new(0), AtomicU32::new(0)]),
            filter_live: Arc::default(),
            status: Arc::default(),
            failed: Default::default(),
            mic: Arc::default(),
            events,
        };
        let (commands, rx) = mpsc::channel();
        let audio = shared.clone();
        let thread = thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx, audio))
            .expect("thread audio");
        let (preload, preload_rx) = mpsc::channel::<String>();
        let engine = Arc::new(Engine {
            shared,
            cache: Mutex::default(),
            commands,
            preload,
            thread: Mutex::new(Some(thread)),
        });
        // Decodes ahead of time, one file after the other.
        let weak = Arc::downgrade(&engine);
        thread::Builder::new()
            .name("préchargement".into())
            .spawn(move || {
                while let Ok(path) = preload_rx.recv() {
                    let Some(engine) = weak.upgrade() else { break };
                    let cell = lock(&engine.cache).get(&path);
                    let _ = engine.decode(&path, &cell);
                }
            })
            .expect("thread préchargement");
        engine
    }

    fn mixer(&self) -> MutexGuard<'_, Mixer> {
        lock(&self.shared.mixer)
    }

    pub fn configure(&self, config: AudioConfig) {
        let _ = self.commands.send(Command::Configure(config));
    }

    pub fn reset(&self) {
        let _ = self.commands.send(Command::Reset);
    }

    pub fn status(&self) -> Status {
        let mut s = lock(&self.shared.status).clone();
        s.running = self.mixer().running;
        s
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Quit);
        if let Some(t) = lock(&self.thread).take() {
            let _ = t.join();
        }
    }

    // ---------- Voices ----------

    /// Starts a voice once the file is decoded. Reports Event::Started or Event::Error.
    pub fn play(self: &Arc<Self>, req: PlayRequest) {
        let generation = self.mixer().generation;
        let cell = lock(&self.cache).get(&req.path);
        if let Some(result) = cell.get() {
            self.start(req, result.clone(), generation);
            return;
        }
        let engine = self.clone();
        thread::spawn(move || {
            let result = engine.decode(&req.path, &cell);
            engine.start(req, result, generation);
        });
    }

    fn decode(&self, path: &str, cell: &Decoded) -> Result<Arc<Pcm>, String> {
        // A decoder bug on a damaged file must not take the thread down (nor the preloader).
        let result = cell
            .get_or_init(|| {
                catch_unwind(|| decode::decode(path))
                    .unwrap_or_else(|_| Err("fichier illisible".into()))
                    .map(Arc::new)
            })
            .clone();
        lock(&self.cache).settle();
        result
    }

    fn start(&self, req: PlayRequest, decoded: Result<Arc<Pcm>, String>, generation: u64) {
        let started = decoded.and_then(|pcm| {
            let duration = pcm.duration();
            let (id, retired) = {
                let mut m = self.mixer();
                let id = m.start(
                    &req.sound_id,
                    &req.title,
                    pcm,
                    req.volume,
                    req.rate,
                    generation,
                );
                (id, m.take_retired())
            };
            drop(retired); // unlocked: see Mixer::take_retired
            Ok(id?.map(|_| duration))
        });
        match started {
            Ok(Some(duration)) => (self.shared.events)(Event::Started {
                sound_id: req.sound_id,
                duration,
            }),
            Ok(None) => {} // "stop all" while the file was decoding
            Err(e) => self
                .shared
                .error(format!("Lecture impossible de « {} » : {e}", req.title)),
        }
    }

    /// Decodes in the background so the first play starts at once.
    pub fn preload(&self, path: &str) {
        let _ = self.preload.send(path.to_string());
    }

    pub fn forget(&self, path: &str) {
        lock(&self.cache).entries.remove(path);
    }

    pub fn voices(&self) -> Vec<VoiceInfo> {
        let (voices, retired) = {
            let mut m = self.mixer();
            (m.voices(), m.take_retired())
        };
        drop(retired); // unlocked: see Mixer::take_retired
        voices
    }

    pub fn stop(&self, id: u64) {
        self.mixer().stop(id);
    }

    pub fn stop_all(&self) {
        self.mixer().stop_all();
    }

    pub fn set_paused(&self, id: u64, paused: bool) {
        self.mixer().set_paused(id, paused);
    }

    pub fn toggle_pause_all(&self) {
        self.mixer().toggle_pause_all();
    }

    pub fn seek(&self, id: u64, seconds: f64) {
        self.mixer().seek(id, seconds);
    }

    pub fn set_volume(&self, id: u64, volume: f32) {
        self.mixer().set_volume(id, volume);
    }

    pub fn nudge_volumes(&self, delta: f32) {
        self.mixer().nudge_volumes(delta);
    }

    pub fn set_rate(&self, id: u64, rate: f32) {
        self.mixer().set_rate(id, rate);
    }

    pub fn set_loop(&self, id: u64, looping: bool) {
        self.mixer().set_loop(id, looping);
    }

    pub fn set_normalize(&self, on: bool) {
        self.mixer().normalize = on;
    }

    pub fn set_max_voices(&self, n: usize) {
        self.mixer().max_voices = n;
    }

    pub fn set_bus_volume(&self, bus: usize, volume: f32) {
        self.mixer().master[bus] = volume;
    }

    pub fn set_mic_volume(&self, volume: f32) {
        self.mixer().mic_volume = volume;
    }

    /// Peak level (0..1) that left the output since the last call.
    pub fn take_level(&self, bus: usize) -> f32 {
        f32::from_bits(self.shared.levels[bus].swap(0, Ordering::Relaxed))
    }

    pub fn filter_live(&self) -> FilterLive {
        match self.shared.filter_live.load(Ordering::Relaxed) {
            0 => FilterLive::NoMap,
            1 => FilterLive::Idle,
            rate => FilterLive::Live(rate),
        }
    }
}

// ---------- Audio thread ----------

#[derive(Clone, Default, PartialEq)]
struct Lists {
    outputs: Vec<String>,
    inputs: Vec<String>,
    default_out: String,
    default_in: String,
}

fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|x| x.name().to_string())
}

fn device_lists(host: &cpal::Host) -> Lists {
    let names = |devices: Option<Vec<cpal::Device>>| {
        devices
            .unwrap_or_default()
            .iter()
            .filter_map(device_name)
            .collect::<Vec<_>>()
    };
    Lists {
        outputs: names(host.output_devices().ok().map(Iterator::collect)),
        inputs: names(host.input_devices().ok().map(Iterator::collect))
            .into_iter()
            .filter(|n| !is_cable_capture(n))
            .collect(),
        default_out: host
            .default_output_device()
            .as_ref()
            .and_then(device_name)
            .unwrap_or_default(),
        default_in: host
            .default_input_device()
            .as_ref()
            .and_then(device_name)
            .unwrap_or_default(),
    }
}

#[derive(Clone, PartialEq, Debug)]
enum CableTarget {
    Off,
    Filter,
    Device(String),
}

// Held for as long as the cable output runs: dropping it stops it.
#[allow(dead_code)]
enum CableOut {
    Stream(cpal::Stream),
    Filter(FilterFeed),
}

#[derive(Default)]
struct Outputs {
    monitor: Option<String>, // device asked for, open or not
    monitor_stream: Option<cpal::Stream>,
    cable: Option<CableTarget>,
    cable_out: Option<CableOut>,
    mic: Option<(String, bool)>, // device, noise suppression
    mic_stream: Option<cpal::Stream>,
}

fn run(rx: Receiver<Command>, sh: Shared) {
    let host = cpal::default_host();
    let mut out = Outputs::default();
    let mut config: Option<AudioConfig> = None;
    let mut known = Lists::default();
    loop {
        let command = rx.recv_timeout(POLL);
        let lists = device_lists(&host);
        match command {
            Ok(Command::Configure(c)) => config = Some(c),
            Ok(Command::Reset) => {
                out = Outputs::default();
                sh.set_running(MONITOR, false);
                sh.set_running(CABLE, false);
            }
            Ok(Command::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                // A stream that failed (device unplugged...) is rebuilt; so is anything whose
                // device came, went, or became the Windows default.
                let failed: Vec<bool> = sh
                    .failed
                    .iter()
                    .map(|f| f.swap(false, Ordering::Relaxed))
                    .collect();
                if failed[0] {
                    out.monitor = None;
                }
                if failed[1] {
                    out.cable = None;
                }
                if failed[2] {
                    out.mic = None;
                }
                if lists == known && !failed.contains(&true) {
                    continue;
                }
            }
        }
        known = lists;
        if let Some(c) = &config {
            apply(&host, c, &known, &mut out, &sh);
        }
        let status = Status {
            outputs: known.outputs.clone(),
            inputs: known.inputs.clone(),
            running: [false; BUSES],
            mic: out.mic_stream.is_some(),
        };
        *lock(&sh.status) = status;
        (sh.events)(Event::Changed);
    }
    drop(out);
    sh.set_running(MONITOR, false);
    sh.set_running(CABLE, false);
}

fn apply(host: &cpal::Host, c: &AudioConfig, l: &Lists, out: &mut Outputs, sh: &Shared) {
    // Monitor: the saved device, else the Windows default.
    let monitor = resolve_device(&l.outputs, &c.monitor)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| l.default_out.clone());
    if out.monitor.as_ref() != Some(&monitor) {
        out.monitor_stream = None;
        sh.set_running(MONITOR, false);
        match open_output(host, &monitor, MONITOR, sh) {
            Ok(s) => {
                out.monitor_stream = Some(s);
                sh.set_running(MONITOR, true);
            }
            Err(e) => sh.error(format!("Sortie casque inaccessible : {e}")),
        }
        out.monitor = Some(monitor);
    }

    let cable = match &c.cable {
        Cable::Filter => CableTarget::Filter,
        Cable::Device(d) => resolve_device(&l.outputs, d)
            .filter(|n| !n.is_empty())
            .map_or(CableTarget::Off, CableTarget::Device),
        Cable::Off | Cable::Unset => CableTarget::Off,
    };
    if out.cable.as_ref() != Some(&cable) {
        out.mic_stream = None;
        out.mic = None;
        out.cable_out = None;
        sh.set_running(CABLE, false);
        match &cable {
            CableTarget::Off => {}
            CableTarget::Filter => {
                out.cable_out = Some(CableOut::Filter(FilterFeed::start(sh)));
                sh.set_running(CABLE, true);
            }
            CableTarget::Device(name) => match open_output(host, name, CABLE, sh) {
                Ok(s) => {
                    out.cable_out = Some(CableOut::Stream(s));
                    sh.set_running(CABLE, true);
                }
                Err(e) => sh.error(format!("Envoi aux autres impossible : {e}")),
            },
        }
        out.cable = Some(cable.clone());
    }

    // Your voice joins the sounds on a virtual cable. With the filter it already is the real
    // microphone: capturing it would loop the sounds.
    let cable_stream = matches!(out.cable_out, Some(CableOut::Stream(_)));
    let mic = (c.mic_enabled && cable_stream).then(|| {
        let name = resolve_device(&l.inputs, &c.mic)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| l.default_in.clone());
        (name, c.noise_suppression)
    });
    if out.mic != mic {
        out.mic_stream = None;
        sh.mic.reset(false);
        if let Some((name, ns)) = &mic {
            match open_mic(host, name, *ns, sh) {
                Ok(s) => {
                    out.mic_stream = Some(s);
                    sh.mic.reset(true);
                }
                Err(e) => sh.error(format!("Micro inaccessible : {e}")),
            }
        }
        out.mic = mic;
    }
}

fn find_device(
    devices: Result<impl Iterator<Item = cpal::Device>, cpal::Error>,
    name: &str,
) -> Result<cpal::Device, String> {
    if name.is_empty() {
        return Err("aucun périphérique".into());
    }
    devices
        .map_err(|e| e.to_string())?
        .find(|d| device_name(d).as_deref() == Some(name))
        .ok_or_else(|| format!("« {name} » introuvable"))
}

fn open_output(
    host: &cpal::Host,
    name: &str,
    bus: usize,
    sh: &Shared,
) -> Result<cpal::Stream, String> {
    let device = find_device(host.output_devices(), name)?;
    let make = |config: StreamConfig| {
        let mut dsp = BusDsp::new(
            bus,
            config.sample_rate,
            sh,
            (bus == CABLE).then(|| sh.mic.clone()),
        );
        let channels = usize::from(config.channels);
        let failed = sh.failed[bus].clone();
        device.build_output_stream::<f32, _, _>(
            config,
            {
                let failed = failed.clone();
                // A panic would silently kill the output: silence instead, and the stream is rebuilt.
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if catch_unwind(AssertUnwindSafe(|| dsp.render(data, channels))).is_err() {
                        data.fill(0.0);
                        failed.store(true, Ordering::Relaxed);
                    }
                }
            },
            move |err| {
                eprintln!("[audio] sortie {bus} : {err}");
                failed.store(true, Ordering::Relaxed);
            },
            None,
        )
    };
    // Windows converts 48 kHz stereo to whatever the device runs at; the device's own format
    // is the fallback.
    let stream = make(StreamConfig {
        channels: 2,
        sample_rate: RATE,
        buffer_size: BufferSize::Default,
    })
    .or_else(|_| {
        let d = device.default_output_config()?;
        make(StreamConfig {
            channels: d.channels(),
            sample_rate: d.sample_rate(),
            buffer_size: BufferSize::Default,
        })
    })
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn open_mic(
    host: &cpal::Host,
    name: &str,
    noise_suppression: bool,
    sh: &Shared,
) -> Result<cpal::Stream, String> {
    let device = find_device(host.input_devices(), name)?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let channels = usize::from(config.channels());
    let mut mic = MicInput::new(
        config.sample_rate(),
        channels,
        noise_suppression,
        sh.mic.clone(),
    );
    let failed = sh.failed[2].clone();
    let on_error = move |err| {
        eprintln!("[audio] micro : {err}");
        failed.store(true, Ordering::Relaxed);
    };
    let stream = match config.sample_format() {
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let _ = catch_unwind(AssertUnwindSafe(|| mic.push(data)));
            },
            on_error,
            None,
        ),
        SampleFormat::I16 => {
            let mut buf = Vec::new();
            device.build_input_stream::<i16, _, _>(
                config.config(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    buf.clear();
                    buf.extend(data.iter().map(|&s| f32::from(s) / 32768.0));
                    let _ = catch_unwind(AssertUnwindSafe(|| mic.push(&buf)));
                },
                on_error,
                None,
            )
        }
        other => return Err(format!("format {other:?} non pris en charge")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

// ---------- Output processing ----------

/// One output's chain: the voices, the volume of the sounds, your voice (cable only, after
/// that volume so it never changes your voice), then a limiter so stacked sounds squash
/// instead of clipping.
struct BusDsp {
    bus: usize,
    rate: u32,
    mixer: Arc<Mutex<Mixer>>,
    levels: Arc<[AtomicU32; BUSES]>,
    mic: Option<Arc<MicPipe>>,
    master: f32,
    mic_gain: f32,
    env: f32,
    mix: Vec<f32>,
    mic_buf: Vec<f32>,
}

impl BusDsp {
    fn new(bus: usize, rate: u32, sh: &Shared, mic: Option<Arc<MicPipe>>) -> BusDsp {
        let master = lock(&sh.mixer).master[bus];
        BusDsp {
            bus,
            rate,
            mixer: sh.mixer.clone(),
            levels: sh.levels.clone(),
            mic,
            master,
            mic_gain: 0.0,
            env: 0.0,
            mix: Vec::new(),
            mic_buf: Vec::new(),
        }
    }

    fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.mix.clear();
        self.mix.resize(frames * 2, 0.0);
        let (master, mic_volume) = {
            let mut m = lock(&self.mixer);
            m.render(self.bus, &mut self.mix, self.rate);
            (m.master[self.bus], m.mic_volume)
        };
        let mic = self
            .mic
            .as_ref()
            .filter(|p| p.active.load(Ordering::Relaxed));
        if let Some(p) = mic {
            self.mic_buf.resize(frames, 0.0);
            p.pull(&mut self.mic_buf, self.rate);
        }
        let rate = self.rate as f32;
        let smooth = 1.0 - (-1.0 / (0.02 * rate)).exp();
        let release = (-1.0 / (0.15 * rate)).exp();
        let mut peak_out = 0f32;
        for (i, f) in out.chunks_exact_mut(channels).enumerate() {
            self.master += (master - self.master) * smooth;
            let (mut l, mut r) = (
                self.mix[2 * i] * self.master,
                self.mix[2 * i + 1] * self.master,
            );
            if mic.is_some() {
                self.mic_gain += (mic_volume - self.mic_gain) * smooth;
                let v = self.mic_buf[i] * self.mic_gain;
                l += v;
                r += v;
            }
            let peak = l.abs().max(r.abs());
            self.env = if peak > self.env {
                peak
            } else {
                self.env * release + peak * (1.0 - release)
            };
            let g = if self.env > LIMIT {
                LIMIT / self.env
            } else {
                1.0
            };
            l = (l * g).clamp(-1.0, 1.0);
            r = (r * g).clamp(-1.0, 1.0);
            peak_out = peak_out.max(l.abs()).max(r.abs());
            if channels == 1 {
                f[0] = (l + r) * 0.5;
            } else {
                f[0] = l;
                f[1] = r;
                f[2..].fill(0.0);
            }
        }
        // Positive floats order like their bits.
        self.levels[self.bus].fetch_max(peak_out.to_bits(), Ordering::Relaxed);
    }
}

// ---------- Microphone (virtual cable mode) ----------

const MIC_TARGET: f64 = 0.03 * RATE as f64; // buffered frames the reader aims for
const MIC_MAX: usize = (0.2 * RATE as f64) as usize;

/// Your voice, mono at 48 kHz, from the microphone's callback to the cable's.
#[derive(Default)]
struct MicPipe {
    active: AtomicBool,
    buf: Mutex<MicBuf>,
}

#[derive(Default)]
struct MicBuf {
    data: VecDeque<f32>,
    pos: f64,
    primed: bool, // enough buffered to start reading
}

impl MicPipe {
    fn reset(&self, active: bool) {
        *lock(&self.buf) = MicBuf::default();
        self.active.store(active, Ordering::Relaxed);
    }

    fn push(&self, samples: &[f32]) {
        let mut b = lock(&self.buf);
        b.data.extend(samples);
        if b.data.len() > MIC_MAX {
            let excess = b.data.len() - MIC_TARGET as usize;
            b.data.drain(..excess);
            b.pos = 0.0;
        }
    }

    /// Reads at `out_rate`, a little faster or slower to follow the drift between the
    /// microphone's clock and the output's.
    fn pull(&self, out: &mut [f32], out_rate: u32) {
        let mut b = lock(&self.buf);
        let fill = b.data.len() as f64 - b.pos;
        if !b.primed && fill < MIC_TARGET {
            out.fill(0.0);
            return;
        }
        b.primed = true;
        let adjust = (1.0 + (fill - MIC_TARGET) / MIC_TARGET * 0.005).clamp(0.99, 1.01);
        let step = f64::from(RATE) / f64::from(out_rate) * adjust;
        for o in out.iter_mut() {
            let i0 = b.pos as usize;
            if i0 + 1 >= b.data.len() {
                *o = 0.0;
                b.primed = false; // ran dry: wait for a fresh margin
                continue;
            }
            let frac = (b.pos - i0 as f64) as f32;
            *o = b.data[i0] + (b.data[i0 + 1] - b.data[i0]) * frac;
            b.pos += step;
        }
        let used = (b.pos as usize).min(b.data.len());
        b.data.drain(..used);
        b.pos -= used as f64;
    }
}

/// The microphone callback's side: mono, 48 kHz, optional noise reduction (RNNoise).
struct MicInput {
    channels: usize,
    step: f64, // input samples per output sample
    pos: f64,
    prev: f32,
    mono: Vec<f32>,
    out: Vec<f32>,
    denoise: Option<(Box<DenoiseState<'static>>, Vec<f32>, Vec<f32>)>, // state, pending input, frame out
    pipe: Arc<MicPipe>,
}

impl MicInput {
    fn new(rate: u32, channels: usize, noise_suppression: bool, pipe: Arc<MicPipe>) -> MicInput {
        MicInput {
            channels: channels.max(1),
            step: f64::from(rate) / f64::from(RATE),
            pos: 0.0,
            prev: 0.0,
            mono: Vec::new(),
            out: Vec::new(),
            denoise: noise_suppression.then(|| {
                (
                    DenoiseState::new(),
                    Vec::new(),
                    vec![0.0; DenoiseState::FRAME_SIZE],
                )
            }),
            pipe,
        }
    }

    fn push(&mut self, data: &[f32]) {
        self.mono.clear();
        self.mono.extend(
            data.chunks_exact(self.channels)
                .map(|f| f.iter().sum::<f32>() / self.channels as f32),
        );
        self.out.clear();
        if self.step == 1.0 {
            self.out.extend_from_slice(&self.mono);
        } else if let Some(&last) = self.mono.last() {
            // Linear interpolation; index -1 is the last sample of the previous block.
            let n = self.mono.len();
            while self.pos < (n - 1) as f64 {
                let i0 = self.pos.floor() as isize;
                let frac = (self.pos - i0 as f64) as f32;
                let s0 = if i0 < 0 {
                    self.prev
                } else {
                    self.mono[i0 as usize]
                };
                let s1 = self.mono[(i0 + 1) as usize];
                self.out.push(s0 + (s1 - s0) * frac);
                self.pos += self.step;
            }
            self.pos -= n as f64;
            self.prev = last;
        }
        match &mut self.denoise {
            None => self.pipe.push(&self.out),
            Some((state, pending, frame)) => {
                // RNNoise works on 10 ms frames of 16-bit-scaled samples.
                pending.extend(self.out.iter().map(|s| s * 32768.0));
                while pending.len() >= DenoiseState::FRAME_SIZE {
                    state.process_frame(frame, &pending[..DenoiseState::FRAME_SIZE]);
                    pending.drain(..DenoiseState::FRAME_SIZE);
                    for s in frame.iter_mut() {
                        *s /= 32768.0;
                    }
                    self.pipe.push(frame);
                }
            }
        }
    }
}

// ---------- Microphone filter feed ----------

/// Renders the cable bus at 48 kHz on its own clock and writes it into the filter's ring.
struct FilterFeed {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FilterFeed {
    fn start(sh: &Shared) -> FilterFeed {
        let stop = Arc::new(AtomicBool::new(false));
        let mut dsp = BusDsp::new(CABLE, RATE, sh, None);
        let (live, events, quit) = (sh.filter_live.clone(), sh.events.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("filtre micro".into())
            .spawn(move || {
                let name = ring::ring_name();
                let mut ring: Option<Ring> = None;
                let mut last_open: Option<Instant> = None;
                let mut state = FilterLive::NoMap;
                let start = Instant::now();
                let mut done: u64 = 0; // frames rendered
                let mut block = vec![0f32; FILTER_BLOCK * 2];
                while !quit.load(Ordering::Relaxed) {
                    if ring.is_none()
                        && last_open.is_none_or(|t| t.elapsed() >= Duration::from_millis(500))
                    {
                        last_open = Some(Instant::now());
                        ring = Ring::open(&name);
                    }
                    let due = (start.elapsed().as_secs_f64() * f64::from(RATE)) as u64;
                    if due > done + u64::from(RATE) {
                        done = due; // the PC slept: skip rather than catch up
                    }
                    while done + FILTER_BLOCK as u64 <= due {
                        dsp.render(&mut block, 2);
                        if let Some(r) = ring.as_mut().filter(|r| r.ready()) {
                            r.write(&block);
                        }
                        done += FILTER_BLOCK as u64;
                    }
                    let next = ring.as_ref().map_or(FilterLive::NoMap, Ring::state);
                    if next != state {
                        state = next;
                        live.store(encode_live(state), Ordering::Relaxed);
                        events(Event::Changed);
                    }
                    thread::sleep(Duration::from_millis(10)); // the filter buffers far more than that
                }
                live.store(encode_live(FilterLive::NoMap), Ordering::Relaxed);
            })
            .expect("thread filtre micro");
        FilterFeed {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FilterFeed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn encode_live(state: FilterLive) -> u32 {
    match state {
        FilterLive::NoMap => 0,
        FilterLive::Idle => 1,
        FilterLive::Live(rate) => rate.max(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mic_pipe_primes_then_follows_the_writer() {
        let pipe = MicPipe::default();
        let mut out = vec![1f32; 480];
        pipe.push(&[0.5; 480]);
        pipe.pull(&mut out, RATE);
        assert!(out.iter().all(|&s| s == 0.0), "not enough buffered yet");
        pipe.push(&[0.5; 1440]);
        pipe.pull(&mut out, RATE);
        assert!(out.iter().all(|&s| s == 0.5));
        // A writer far ahead is cut back to the target latency.
        pipe.push(&[0.25; 20000]);
        assert!(lock(&pipe.buf).data.len() <= MIC_TARGET as usize + 1);
    }

    #[test]
    fn mic_input_resamples_to_48k() {
        let pipe = Arc::new(MicPipe::default());
        let mut mic = MicInput::new(44100, 2, false, pipe.clone());
        for _ in 0..10 {
            mic.push(&[0.3; 441 * 2]); // 10 ms of stereo at 44.1 kHz
        }
        let n = lock(&pipe.buf).data.len();
        assert!((n as i64 - 4800).abs() <= 2, "{n} samples for 100 ms");
        assert!(
            lock(&pipe.buf)
                .data
                .iter()
                .skip(1)
                .all(|&s| (s - 0.3).abs() < 1e-6)
        );
    }

    #[test]
    fn the_limiter_keeps_stacked_sounds_below_full_scale() {
        let sh = Shared {
            mixer: Arc::default(),
            levels: Arc::new([AtomicU32::new(0), AtomicU32::new(0)]),
            filter_live: Arc::default(),
            status: Arc::default(),
            failed: Default::default(),
            mic: Arc::default(),
            events: Arc::new(|_| {}),
        };
        {
            let mut m = lock(&sh.mixer);
            m.running = [true, false];
            m.normalize = false;
            m.master = [1.0, 1.0];
            for _ in 0..4 {
                m.start("s", "t", Arc::new(Pcm::new(&[0.9; 9600], 1.0)), 1.0, 1.0, 0)
                    .unwrap();
            }
        }
        let mut dsp = BusDsp::new(MONITOR, RATE, &sh, None);
        let mut out = vec![0f32; 4800 * 2];
        dsp.render(&mut out, 2);
        assert!(out.iter().all(|s| s.abs() <= LIMIT + 1e-6));
        assert!((out[100] - LIMIT).abs() < 1e-3);
        let level = f32::from_bits(sh.levels[MONITOR].load(Ordering::Relaxed));
        assert!((level - LIMIT).abs() < 1e-3);
    }
}
