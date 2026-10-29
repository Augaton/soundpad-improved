// The voices, shared by the control side (UI, hotkeys) and the output callbacks. Each output
// ("bus") reads every voice with its own cursor: the two devices run on separate clocks.
use std::sync::Arc;

use super::decode::{Pcm, RATE};

pub const MONITOR: usize = 0; // what you hear
pub const CABLE: usize = 1; // what the others hear
pub const BUSES: usize = 2;

const SMOOTH_S: f32 = 0.01; // gain changes, fades in and out
const SILENT: f32 = 1e-4;

#[derive(Clone, Debug)]
pub struct VoiceInfo {
    pub id: u64,
    pub sound_id: String,
    pub title: String,
    pub position: f64, // seconds of source audio
    pub duration: f64,
    pub volume: f32,
    pub rate: f32,
    pub looping: bool,
    pub paused: bool,
}

#[derive(Clone, Copy, Default)]
struct Cursor {
    pos: f64, // frames of source audio
    gain: f32,
    live: bool, // the bus was running when the voice started
    done: bool,
}

struct Voice {
    id: u64,
    sound_id: String,
    title: String,
    pcm: Arc<Pcm>,
    volume: f32,
    rate: f32,
    looping: bool,
    paused: bool,
    stopping: bool, // fading out, then removed
    cursors: [Cursor; BUSES],
}

impl Voice {
    fn finished(&self) -> bool {
        self.cursors.iter().filter(|c| c.live).all(|c| c.done)
    }

    fn primary(&self) -> &Cursor {
        self.cursors
            .iter()
            .find(|c| c.live)
            .unwrap_or(&self.cursors[0])
    }
}

pub struct Mixer {
    voices: Vec<Voice>,  // oldest first
    retired: Vec<Voice>, // removed by a callback: freed on the control side, not in real time
    next_id: u64,
    pub generation: u64, // bumped by stop_all, so a sound still decoding does not start after it
    pub normalize: bool,
    pub max_voices: usize,
    pub running: [bool; BUSES],
    pub master: [f32; BUSES],
    pub mic_volume: f32,
}

impl Default for Mixer {
    fn default() -> Mixer {
        Mixer {
            voices: Vec::new(),
            retired: Vec::new(),
            next_id: 1,
            generation: 0,
            normalize: true,
            max_voices: 16,
            running: [false; BUSES],
            master: [0.8, 1.0],
            mic_volume: 1.0,
        }
    }
}

impl Mixer {
    fn find(&mut self, id: u64) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id && !v.stopping)
    }

    pub fn voices(&mut self) -> Vec<VoiceInfo> {
        self.retired.clear();
        self.voices
            .iter()
            .filter(|v| !v.stopping)
            .map(|v| {
                let frames = v.pcm.frames().max(1) as f64;
                let pos = v.primary().pos.min(frames);
                VoiceInfo {
                    id: v.id,
                    sound_id: v.sound_id.clone(),
                    title: v.title.clone(),
                    position: pos / f64::from(RATE),
                    duration: v.pcm.duration(),
                    volume: v.volume,
                    rate: v.rate,
                    looping: v.looping,
                    paused: v.paused,
                }
            })
            .collect()
    }

    /// Starts a voice. Ok(None) when a stop_all happened since `generation` was read.
    pub fn start(
        &mut self,
        sound_id: &str,
        title: &str,
        pcm: Arc<Pcm>,
        volume: f32,
        rate: f32,
        generation: u64,
    ) -> Result<Option<u64>, String> {
        if generation != self.generation {
            return Ok(None);
        }
        if !self.running.iter().any(|&r| r) {
            return Err("aucune sortie audio active".into());
        }
        self.retired.clear();
        loop {
            let playing: Vec<u64> = self
                .voices
                .iter()
                .filter(|v| !v.stopping)
                .map(|v| v.id)
                .collect();
            if playing.len() < self.max_voices.max(1) {
                break;
            }
            self.stop(playing[0]);
        }
        let id = self.next_id;
        self.next_id += 1;
        let gain = volume * if self.normalize { pcm.gain } else { 1.0 };
        let cursors = self.running.map(|live| Cursor {
            pos: 0.0,
            gain,
            live,
            done: false,
        });
        self.voices.push(Voice {
            id,
            sound_id: sound_id.into(),
            title: title.into(),
            pcm,
            volume,
            rate,
            looping: false,
            paused: false,
            stopping: false,
            cursors,
        });
        Ok(Some(id))
    }

    /// Fades the voice out; it disappears from voices() at once.
    pub fn stop(&mut self, id: u64) {
        if let Some(v) = self.find(id) {
            v.stopping = true;
        }
        self.sweep();
    }

    pub fn stop_all(&mut self) {
        self.generation += 1;
        for v in &mut self.voices {
            v.stopping = true;
        }
        self.sweep();
    }

    pub fn set_paused(&mut self, id: u64, paused: bool) {
        if let Some(v) = self.find(id) {
            v.paused = paused;
        }
    }

    /// Pauses everything, or resumes everything when all voices are already paused.
    pub fn toggle_pause_all(&mut self) {
        let pause = self.voices.iter().any(|v| !v.stopping && !v.paused);
        for v in self.voices.iter_mut().filter(|v| !v.stopping) {
            v.paused = pause;
        }
    }

    pub fn seek(&mut self, id: u64, seconds: f64) {
        if let Some(v) = self.find(id) {
            let frames = v.pcm.frames() as f64;
            let pos = (seconds * f64::from(RATE)).clamp(0.0, (frames - 480.0).max(0.0));
            for c in &mut v.cursors {
                c.pos = pos;
                c.done = false;
            }
        }
    }

    pub fn set_volume(&mut self, id: u64, volume: f32) {
        if let Some(v) = self.find(id) {
            v.volume = volume.clamp(0.0, 2.0);
        }
    }

    /// For the in-game volume hotkeys: every playing voice, by `delta`.
    pub fn nudge_volumes(&mut self, delta: f32) {
        for v in self.voices.iter_mut().filter(|v| !v.stopping) {
            v.volume = (((v.volume + delta) * 100.0).round() / 100.0).clamp(0.0, 2.0);
        }
    }

    pub fn set_rate(&mut self, id: u64, rate: f32) {
        if let Some(v) = self.find(id) {
            v.rate = rate.clamp(0.5, 2.0);
        }
    }

    pub fn set_loop(&mut self, id: u64, looping: bool) {
        if let Some(v) = self.find(id) {
            v.looping = looping;
        }
    }

    /// A bus starts or stops: voices only play on the buses that were running when they started.
    pub fn set_running(&mut self, bus: usize, running: bool) {
        self.running[bus] = running;
        if !running {
            for v in &mut self.voices {
                v.cursors[bus].live = false;
            }
            self.sweep();
        }
    }

    /// Removes the voices that are over on every bus they play on.
    fn sweep(&mut self) {
        let mut i = 0;
        while i < self.voices.len() {
            let v = &self.voices[i];
            let silent = v.stopping
                && v.cursors
                    .iter()
                    .filter(|c| c.live && !c.done)
                    .all(|c| c.gain <= SILENT);
            if v.finished() || silent {
                self.retired.push(self.voices.remove(i));
            } else {
                i += 1;
            }
        }
    }

    /// Adds every voice to `out` (interleaved stereo at `out_rate`), for one bus.
    pub fn render(&mut self, bus: usize, out: &mut [f32], out_rate: u32) {
        let smooth = 1.0 - (-1.0 / (SMOOTH_S * out_rate as f32)).exp();
        let step_base = f64::from(RATE) / f64::from(out_rate);
        let normalize = self.normalize;
        for v in &mut self.voices {
            let target = if v.stopping || v.paused {
                0.0
            } else {
                v.volume * if normalize { v.pcm.gain } else { 1.0 }
            };
            let step = step_base * f64::from(v.rate);
            let pcm = &v.pcm;
            let frames = v.pcm.frames();
            let c = &mut v.cursors[bus];
            if !c.live || c.done || frames == 0 {
                continue;
            }
            for frame in out.as_chunks_mut::<2>().0 {
                c.gain += (target - c.gain) * smooth;
                if target == 0.0 && c.gain <= SILENT {
                    c.gain = 0.0;
                    c.done |= v.stopping;
                    break; // paused: the cursor waits
                }
                let i0 = c.pos as usize;
                let frac = (c.pos - i0 as f64) as f32;
                let i1 = if i0 + 1 < frames {
                    i0 + 1
                } else if v.looping {
                    0
                } else {
                    i0
                };
                let (a, b) = (pcm.frame(i0), pcm.frame(i1));
                for ch in 0..2 {
                    frame[ch] += (a[ch] + (b[ch] - a[ch]) * frac) * c.gain;
                }
                c.pos += step;
                if c.pos >= frames as f64 {
                    if v.looping {
                        c.pos %= frames as f64;
                    } else {
                        c.done = true;
                        break;
                    }
                }
            }
        }
        self.sweep();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(frames: usize, value: f32) -> Arc<Pcm> {
        Arc::new(Pcm::new(&vec![value; frames * 2], 0.5))
    }

    fn mixer() -> Mixer {
        Mixer {
            running: [true, false],
            ..Mixer::default()
        }
    }

    #[test]
    fn voices_play_to_the_end_then_disappear() {
        let mut m = mixer();
        let id = m
            .start("s", "t", pcm(1000, 0.4), 1.0, 1.0, 0)
            .unwrap()
            .unwrap();
        let mut out = vec![0f32; 600 * 2];
        m.render(MONITOR, &mut out, RATE);
        assert!((out[0] - 0.2).abs() < 1e-4, "normalized: 0.4 * gain 0.5");
        assert_eq!(m.voices()[0].id, id);
        assert!((m.voices()[0].position - 600.0 / 48000.0).abs() < 1e-9);
        out.fill(0.0);
        m.render(MONITOR, &mut out, RATE);
        assert!(
            out[2 * 399] != 0.0 && out[2 * 400] == 0.0,
            "stops after the last frame"
        );
        assert!(m.voices().is_empty());
    }

    #[test]
    fn speed_and_output_rate_set_the_step() {
        let mut m = mixer();
        let id = m
            .start("s", "t", pcm(48000, 0.1), 1.0, 2.0, 0)
            .unwrap()
            .unwrap();
        m.render(MONITOR, &mut vec![0f32; 4800 * 2], 24000);
        // 4800 output frames at 24 kHz, double speed: 4 x 4800 source frames.
        assert!((m.voices()[0].position - 19200.0 / 48000.0).abs() < 1e-9);
        m.seek(id, 0.5);
        assert!((m.voices()[0].position - 0.5).abs() < 1e-9);
    }

    #[test]
    fn stop_fades_out_and_the_voice_count_is_capped() {
        let mut m = mixer();
        m.max_voices = 2;
        let a = m
            .start("a", "a", pcm(48000, 0.5), 1.0, 1.0, 0)
            .unwrap()
            .unwrap();
        m.start("b", "b", pcm(48000, 0.5), 1.0, 1.0, 0).unwrap();
        m.start("c", "c", pcm(48000, 0.5), 1.0, 1.0, 0).unwrap();
        let ids: Vec<String> = m.voices().into_iter().map(|v| v.sound_id).collect();
        assert_eq!(ids, ["b", "c"], "the oldest made room");
        assert!(m.voices().iter().all(|v| v.id != a));
        m.stop_all();
        assert!(m.voices().is_empty());
        let mut out = vec![0f32; 4800 * 2];
        m.render(MONITOR, &mut out, RATE);
        assert!(out[0] > 0.0 && out[2 * 4799].abs() < 1e-3, "fade, no click");
        assert!(m.voices.is_empty(), "removed once silent");
        assert_eq!(
            m.start("d", "d", pcm(10, 0.5), 1.0, 1.0, 0),
            Ok(None),
            "stop_all cancels pending starts"
        );
    }

    #[test]
    fn pause_holds_the_position_and_loop_wraps() {
        let mut m = mixer();
        let id = m
            .start("s", "t", pcm(6000, 0.5), 1.0, 1.0, 0)
            .unwrap()
            .unwrap();
        m.set_paused(id, true);
        let mut out = vec![0f32; 4800 * 2];
        m.render(MONITOR, &mut out, RATE);
        let held = m.voices()[0].position;
        m.render(MONITOR, &mut out, RATE);
        assert_eq!(m.voices()[0].position, held);
        m.set_paused(id, false);
        m.set_loop(id, true);
        m.render(MONITOR, &mut out, RATE);
        assert_eq!(m.voices().len(), 1, "a looping voice never ends");
        assert!(m.voices()[0].position < held, "wrapped to the start");
        m.toggle_pause_all();
        assert!(m.voices()[0].paused);
        m.toggle_pause_all();
        assert!(!m.voices()[0].paused);
    }

    #[test]
    fn nothing_starts_without_an_output() {
        let mut m = Mixer::default();
        assert!(m.start("s", "t", pcm(10, 0.5), 1.0, 1.0, 0).is_err());
        let mut m = mixer();
        m.start("s", "t", pcm(48000, 0.5), 1.0, 1.0, 0).unwrap();
        m.set_running(MONITOR, false);
        assert!(
            m.voices().is_empty(),
            "a voice whose only bus stopped is gone"
        );
    }
}
