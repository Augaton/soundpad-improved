// The app's state and actions, shared by the window and the hotkey thread: a hotkey must play
// even while the window is minimized and draws nothing.
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, RandomState};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SendError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::audio::engine::{AudioConfig, Engine, Event, PlayRequest};
use crate::audio::mixer::{CABLE, MONITOR};
use crate::filter::{self, FilterAction};
use crate::hotkeys::Hotkeys;
use crate::keys;
use crate::link;
use crate::model::{
    Action, Cable, Device, FilterStatus, GlobalAction, Library, Mode, Sound, effective_mode,
    is_newer, is_virtual_cable, pick_random, plan_play, relink,
};
use crate::soundpad;
use crate::store;

pub const VOLUME_STEP: f32 = 0.1;
const SAVE_DELAY: Duration = Duration::from_millis(400);
pub const RELEASES: &str = "https://github.com/Augaton/soundpad-improved/releases/latest";
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/Augaton/soundpad-improved/releases/latest";
/// The version CI gives the exe ("0.2.13"); None for a build from the sources.
pub const VERSION: Option<&str> = option_env!("SPI_VERSION");

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

#[derive(Default)]
pub struct FilterState {
    pub status: Option<FilterStatus>,
    pub busy: bool, // waiting for the UAC prompt or the script
}

/// What the window draws. Rebuilt only when the revision moved, not at every frame.
pub struct Snapshot {
    pub lib: Library,
    pub missing: HashSet<String>,        // sound ids whose file is gone
    pub failed_hotkeys: HashSet<String>, // accelerators another app holds
}

pub struct Core {
    lib: Mutex<Library>,
    missing: Mutex<HashSet<String>>,
    failed_hotkeys: Mutex<HashSet<String>>,
    revision: AtomicU64, // moves with every change the window shows
    writable: bool,      // false when library.json exists but could not be read: never overwrite it
    pub engine: Arc<Engine>,
    pub filter: Mutex<FilterState>,
    pub soundpad_count: Mutex<usize>, // sounds in Soundpad's list, shown while the library is empty
    pub update: Mutex<Option<String>>, // a newer version published on GitHub
    next_random: Mutex<HashMap<String, String>>, // category id -> its hotkey's next sound, decoded ahead
    toasts: Mutex<Vec<Toast>>,
    hotkeys: OnceLock<Hotkeys<Action>>,
    saves: Mutex<Option<Sender<()>>>,
    write_lock: Mutex<()>,
    ctx: OnceLock<eframe::egui::Context>,
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Core {
    pub fn new() -> Arc<Core> {
        let loaded = store::load_library();
        Arc::new_cyclic(|weak: &Weak<Core>| {
            let events = weak.clone();
            let engine = Engine::new(Arc::new(move |e| {
                if let Some(core) = events.upgrade() {
                    core.on_engine_event(e);
                }
            }));
            let core = Core {
                lib: Mutex::new(loaded.lib),
                missing: Mutex::default(),
                failed_hotkeys: Mutex::default(),
                revision: AtomicU64::new(0),
                writable: loaded.writable,
                engine,
                filter: Mutex::default(),
                soundpad_count: Mutex::new(0),
                update: Mutex::default(),
                next_random: Mutex::default(),
                toasts: Mutex::default(),
                hotkeys: OnceLock::new(),
                saves: Mutex::new(None),
                write_lock: Mutex::new(()),
                ctx: OnceLock::new(),
            };
            if let Some(w) = loaded.warning {
                core.toast(w, true);
            }
            core
        })
    }

    pub fn read<R>(&self, f: impl FnOnce(&Library) -> R) -> R {
        f(&lock(&self.lib))
    }

    /// Changes the library; the window picks the change up at its next frame.
    pub fn edit<R>(&self, f: impl FnOnce(&mut Library) -> R) -> R {
        let r = f(&mut lock(&self.lib));
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.repaint();
        r
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            lib: lock(&self.lib).clone(),
            missing: lock(&self.missing).clone(),
            failed_hotkeys: lock(&self.failed_hotkeys).clone(),
        }
    }

    /// Everything that needs the window: hotkeys, audio, background checks.
    pub fn start(self: &Arc<Core>, ctx: eframe::egui::Context) {
        let _ = self.ctx.set(ctx);
        let weak = Arc::downgrade(self);
        let _ = self.hotkeys.set(Hotkeys::start(move |action| {
            if let Some(core) = weak.upgrade() {
                core.on_action(&action);
            }
        }));
        self.start_saver();
        let (normalize, max_voices) = self.read(|l| (l.settings.normalize, l.settings.max_voices));
        self.engine.set_normalize(normalize);
        self.engine.set_max_voices(max_voices as usize);
        self.configure_audio();
        self.apply_hotkeys();
        let core = self.clone();
        thread::spawn(move || {
            core.check_missing();
            core.preload_hotkey_sounds();
            if core.read(|l| l.sounds.is_empty())
                && let Ok(text) = store::read_text(&store::soundpad_file())
            {
                *lock(&core.soundpad_count) = soundpad::parse_soundpad_list(&text).sounds.len();
            }
            core.detect_cable();
            crate::autostart::follow_exe();
            core.repaint();
            core.check_update();
        });
    }

    pub fn repaint(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }

    pub fn toast(&self, text: impl Into<String>, error: bool) {
        lock(&self.toasts).push(Toast {
            text: text.into(),
            error,
            at: Instant::now(),
        });
        self.repaint();
    }

    pub fn take_toasts(&self) -> Vec<Toast> {
        std::mem::take(&mut lock(&self.toasts))
    }

    fn on_engine_event(&self, e: Event) {
        match e {
            Event::Started { sound_id, duration } => {
                self.edit(|lib| {
                    if let Some(s) = lib.sound_mut(&sound_id) {
                        s.plays += 1;
                        s.duration = Some(duration); // the file's own, its leading silence cut
                    }
                });
                self.save();
                self.repaint();
            }
            Event::Error(text) => self.toast(text, true),
            Event::Changed => self.repaint(),
        }
    }

    // ---------- Playing ----------

    pub fn on_action(&self, action: &Action) {
        match action {
            Action::Play(id) => self.trigger(id),
            Action::Random(category) => self.play_random(category),
            Action::Global(GlobalAction::StopAll) => self.engine.stop_all(),
            Action::Global(GlobalAction::PauseAll) => self.engine.toggle_pause_all(),
            Action::Global(GlobalAction::VolUp) => self.engine.nudge_volumes(VOLUME_STEP),
            Action::Global(GlobalAction::VolDown) => self.engine.nudge_volumes(-VOLUME_STEP),
        }
        self.repaint();
    }

    pub fn trigger(&self, sound_id: &str) {
        let Some((sound, mode, exclusive)) = self.read(|lib| {
            lib.sound(sound_id).map(|s| {
                (
                    s.clone(),
                    effective_mode(s, &lib.settings),
                    lib.settings.exclusive,
                )
            })
        }) else {
            return;
        };
        self.play(&sound, mode, exclusive);
    }

    /// The sounds a category's hotkey may play, as (id, path): those whose file is there.
    fn random_candidates(&self, category: &str) -> Vec<(String, String)> {
        let missing = lock(&self.missing).clone();
        self.read(|l| {
            let Some(c) = l.categories.iter().find(|c| c.id == category) else {
                return Vec::new();
            };
            let members: HashSet<&String> = c.sound_ids.iter().collect();
            l.sounds
                .iter()
                .filter(|s| members.contains(&s.id) && !missing.contains(&s.id))
                .map(|s| (s.id.clone(), s.path.clone()))
                .collect()
        })
    }

    /// Picks the category's next sound now, and decodes it: the hotkey plays at once without
    /// decoding the whole category ahead.
    fn plan_random(&self, category: &str, candidates: &[(String, String)], last: Option<&str>) {
        let ids: Vec<&str> = candidates.iter().map(|(id, _)| id.as_str()).collect();
        let seed = RandomState::new().hash_one(Instant::now());
        let Some(i) =
            pick_random(&ids, last, seed).and_then(|id| ids.iter().position(|x| *x == id))
        else {
            return;
        };
        self.engine.preload(&candidates[i].1);
        lock(&self.next_random).insert(category.to_string(), candidates[i].0.clone());
    }

    /// A category's hotkey: the sound picked in advance, then the next one is picked, never the
    /// same twice in a row.
    fn play_random(&self, category: &str) {
        let candidates = self.random_candidates(category);
        let planned = lock(&self.next_random)
            .remove(category)
            .filter(|id| candidates.iter().any(|(x, _)| x == id));
        let id = match planned {
            Some(id) => id,
            None => {
                let ids: Vec<&str> = candidates.iter().map(|(id, _)| id.as_str()).collect();
                let seed = RandomState::new().hash_one(Instant::now());
                let Some(id) = pick_random(&ids, None, seed) else {
                    return;
                };
                id.to_string()
            }
        };
        self.trigger(&id);
        self.plan_random(category, &candidates, Some(&id));
    }

    /// The editor's "Listen": the settings being edited, from the start.
    pub fn preview(&self, sound: &Sound) {
        self.play(sound, Mode::Restart, false);
    }

    fn play(&self, sound: &Sound, mode: Mode, exclusive: bool) {
        if lock(&self.missing).contains(&sound.id) {
            self.toast(format!("Fichier introuvable : {}", sound.path), true);
            return;
        }
        let voices = self.engine.voices();
        let same: Vec<u64> = voices
            .iter()
            .filter(|v| v.sound_id == sound.id)
            .map(|v| v.id)
            .collect();
        let all: Vec<u64> = voices.iter().map(|v| v.id).collect();
        let (stop, play) = plan_play(mode, &same, &all, exclusive);
        for id in stop {
            self.engine.stop(id);
        }
        if play {
            self.engine.play(PlayRequest {
                sound_id: sound.id.clone(),
                path: sound.path.clone(),
                title: sound.title.clone(),
                volume: sound.volume as f32,
                rate: sound.rate as f32,
            });
        }
    }

    // ---------- Saving ----------

    fn start_saver(self: &Arc<Core>) {
        let (tx, rx) = mpsc::channel::<()>();
        *lock(&self.saves) = Some(tx);
        let weak = Arc::downgrade(self);
        thread::Builder::new()
            .name("sauvegarde".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    // Grouped: one write for a burst of changes (a slider being dragged...).
                    loop {
                        match rx.recv_timeout(SAVE_DELAY) {
                            Ok(()) => continue,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    let Some(core) = weak.upgrade() else { break };
                    core.save_now();
                }
            })
            .expect("thread sauvegarde");
    }

    /// Saves a little later, grouped with the changes that follow.
    pub fn save(&self) {
        match lock(&self.saves).as_ref() {
            Some(tx) => {
                let _ = tx.send(());
            }
            None => self.save_now(),
        }
    }

    pub fn save_now(&self) {
        if !self.writable {
            return; // the file on disk could not be read: writing would erase it
        }
        let _w = lock(&self.write_lock);
        let snapshot = lock(&self.lib).clone();
        if let Err(e) = store::save_library(&snapshot) {
            self.toast(format!("Sauvegarde impossible : {e}"), true);
        }
    }

    pub fn shutdown(&self) {
        *lock(&self.saves) = None;
        self.save_now();
        self.engine.shutdown();
    }

    // ---------- Hotkeys ----------

    /// Registers every hotkey of the library; those another app holds are reported.
    pub fn apply_hotkeys(&self) {
        let Some(hk) = self.hotkeys.get() else { return };
        let list = self.read(Library::hotkeys);
        let ok = hk.set(
            list.iter()
                .map(|h| (h.vk, h.mods, h.action.clone()))
                .collect(),
        );
        let failed: Vec<String> = list
            .iter()
            .zip(&ok)
            .filter(|(_, ok)| !**ok)
            .map(|(h, _)| h.accel.clone())
            .collect();
        if !failed.is_empty() {
            let names: Vec<String> = failed.iter().map(|a| keys::format_accelerator(a)).collect();
            self.toast(
                format!(
                    "Raccourci déjà pris par une autre application (Soundpad ouvert ?) : {}",
                    names.join(", ")
                ),
                true,
            );
        }
        *lock(&self.failed_hotkeys) = failed.into_iter().collect();
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.repaint();
    }

    /// While a hotkey is being typed: Windows would swallow keys that are registered.
    pub fn suspend_hotkeys(&self) {
        if let Some(hk) = self.hotkeys.get() {
            hk.set(Vec::new());
        }
    }

    // ---------- Audio ----------

    pub fn configure_audio(&self) {
        let s = self.read(|l| l.settings.clone());
        self.engine.set_bus_volume(MONITOR, s.monitor_volume as f32);
        self.engine.set_bus_volume(CABLE, s.cable_volume as f32);
        self.engine.set_mic_volume(s.mic_volume as f32);
        self.engine.configure(AudioConfig {
            monitor: s.monitor_device,
            cable: s.cable_device,
            mic: s.mic_device,
            mic_enabled: s.mic_enabled,
            noise_suppression: s.mic_noise_suppression,
        });
    }

    /// First start: the filter if it is installed, else a virtual cable if there is one.
    fn detect_cable(&self) {
        match self.read(|l| l.settings.cable_device.clone()) {
            Cable::Unset => {}
            Cable::Filter => return self.refresh_filter(),
            _ => return,
        }
        self.refresh_filter();
        let installed = lock(&self.filter)
            .status
            .as_ref()
            .is_some_and(|s| s.ok && !s.installed.is_empty());
        let cable = if installed {
            None
        } else {
            // The device list comes from the audio thread, a moment after start-up.
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                let outputs = self.engine.status().outputs;
                if !outputs.is_empty() || Instant::now() > deadline {
                    break outputs.into_iter().find(|n| is_virtual_cable(n));
                }
                thread::sleep(Duration::from_millis(100));
            }
        };
        let chosen = self.edit(|lib| {
            if lib.settings.cable_device != Cable::Unset {
                return false; // chosen meanwhile
            }
            lib.settings.cable_device = match (&cable, installed) {
                (_, true) => Cable::Filter,
                (Some(name), false) => Cable::Device(Device::named(name)),
                (None, false) => return false,
            };
            true
        });
        if !chosen {
            return;
        }
        if let Some(name) = cable {
            self.toast(format!("Micro virtuel détecté : {name}"), false);
        }
        self.save();
        self.configure_audio();
    }

    /// Asks mic-filter.ps1 which microphones have the filter (about a second).
    pub fn refresh_filter(&self) {
        let status = filter::status();
        lock(&self.filter).status = Some(status);
        self.repaint();
    }

    pub fn run_filter(self: &Arc<Core>, action: FilterAction) {
        lock(&self.filter).busy = true;
        let core = self.clone();
        thread::spawn(move || {
            let result = filter::run(action);
            lock(&core.filter).busy = false;
            if result.ok {
                lock(&core.filter).status = Some(result);
                core.toast(
                    if action == FilterAction::Install {
                        "Filtre installé sur ton micro."
                    } else {
                        "Filtre retiré : le micro est revenu à son état d’avant."
                    },
                    false,
                );
                // The script restarted the Windows audio service: every stream is reopened.
                core.engine.reset();
                core.configure_audio();
            } else {
                core.toast(
                    format!(
                        "Filtre micro : {}",
                        crate::model::filter_error(&result.error)
                    ),
                    true,
                );
            }
            core.repaint();
        });
    }

    // ---------- Library ----------

    pub fn check_missing(&self) {
        let sounds: Vec<(String, String)> = self.read(|l| {
            l.sounds
                .iter()
                .map(|s| (s.id.clone(), s.path.clone()))
                .collect()
        });
        // A path the app would refuse to read counts as missing, without being touched.
        let missing = sounds
            .into_iter()
            .filter(|(_, p)| !store::is_audio_path(Path::new(p)) || !Path::new(p).is_file())
            .map(|(id, _)| id)
            .collect();
        *lock(&self.missing) = missing;
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.repaint();
    }

    /// Sounds bound to a hotkey are played blind, in game: decoded ahead of time. For a
    /// category's hotkey, only the sound it plays next.
    pub fn preload_hotkey_sounds(&self) {
        let missing = lock(&self.missing).clone();
        let (paths, categories): (Vec<String>, Vec<String>) = self.read(|l| {
            (
                l.sounds
                    .iter()
                    .filter(|s| s.hotkey.is_some() && !missing.contains(&s.id))
                    .map(|s| s.path.clone())
                    .collect(),
                l.categories
                    .iter()
                    .filter(|c| c.hotkey.is_some())
                    .map(|c| c.id.clone())
                    .collect(),
            )
        });
        for p in paths {
            self.engine.preload(&p);
        }
        for c in categories {
            self.plan_random(&c, &self.random_candidates(&c), None);
        }
    }

    /// Adds files to the library (and to `category` when given). Known paths are skipped.
    pub fn add_paths(&self, paths: &[String], category: Option<&str>) {
        if paths.is_empty() {
            return;
        }
        let added = self.edit(|lib| {
            let mut known: HashSet<String> =
                lib.sounds.iter().map(|s| s.path.to_lowercase()).collect();
            let fresh: Vec<Sound> = paths
                .iter()
                .filter(|p| known.insert(p.to_lowercase()))
                .map(|p| Sound::new(p))
                .collect();
            let ids: Vec<String> = fresh.iter().map(|s| s.id.clone()).collect();
            lib.sounds.extend(fresh);
            if let Some(cat) = category.and_then(|c| lib.categories.iter_mut().find(|x| x.id == c))
            {
                cat.sound_ids.extend(ids.iter().cloned());
            }
            ids.len()
        });
        self.save_now();
        self.toast(
            match added {
                0 => "Ces sons sont déjà dans la bibliothèque".to_string(),
                1 => "1 son ajouté".to_string(),
                n => format!("{n} sons ajoutés"),
            },
            false,
        );
    }

    /// Downloads the sound behind a link (Voicemod Tuna, MyInstants, an audio file) and adds
    /// it. The answer comes on the receiver: its title, or why not; with nobody left to read it
    /// (the dialog was closed), it becomes a toast.
    pub fn add_link(
        self: &Arc<Core>,
        url: String,
        category: Option<String>,
    ) -> Receiver<Result<String, String>> {
        let (tx, rx) = mpsc::channel();
        let core = self.clone();
        thread::spawn(move || {
            let dir = store::sounds_dir();
            let known = |title: &str| {
                core.read(|l| {
                    l.sounds.iter().any(|s| {
                        s.title.eq_ignore_ascii_case(title) && Path::new(&s.path).starts_with(&dir)
                    })
                })
            };
            let result = link::download(&url, &dir, known).map(|d| {
                core.add_sound(&d.path, &d.title, d.duration, category.as_deref());
                d.title
            });
            match tx.send(result) {
                Ok(()) => {}
                Err(SendError(Ok(title))) => core.toast(format!("« {title} » ajouté"), false),
                Err(SendError(Err(e))) => core.toast(format!("Lien : {e}"), true),
            }
            core.repaint();
        });
        rx
    }

    fn add_sound(&self, path: &Path, title: &str, duration: f64, category: Option<&str>) {
        let path = path.to_string_lossy().into_owned();
        self.edit(|lib| {
            let mut s = Sound::new(&path);
            s.title = title.to_string();
            s.duration = Some(duration);
            if let Some(c) = category.and_then(|c| lib.categories.iter_mut().find(|x| x.id == c)) {
                c.sound_ids.push(s.id.clone());
            }
            lib.sounds.push(s);
        });
        self.save_now();
        self.engine.preload(&path);
    }

    pub fn import_soundpad(self: &Arc<Core>, file: &Path) {
        let text = match store::read_text(file) {
            Ok(t) => t,
            Err(e) => return self.toast(format!("Import impossible : {e}"), true),
        };
        let parsed = soundpad::parse_soundpad_list(&text);
        if parsed.sounds.is_empty() {
            return self.toast(format!("Aucun son trouvé dans {}", file.display()), true);
        }
        let (added, skipped) = self.edit(|lib| soundpad::merge_import(lib, &parsed));
        self.save_now();
        let mut text = if added == 1 {
            "1 son importé".to_string()
        } else {
            format!("{added} sons importés")
        };
        if skipped > 0 {
            text += &format!(", {skipped} déjà présents");
        }
        self.toast(text, false);
        let core = self.clone();
        thread::spawn(move || {
            core.apply_hotkeys();
            core.check_missing();
            core.preload_hotkey_sounds();
        });
    }

    pub fn remove_sounds(&self, ids: &[String]) {
        let ids: HashSet<&String> = ids.iter().collect();
        let removed: Vec<Sound> = self.edit(|lib| {
            let (removed, kept) = std::mem::take(&mut lib.sounds)
                .into_iter()
                .partition(|s| ids.contains(&s.id));
            lib.sounds = kept;
            for c in &mut lib.categories {
                c.sound_ids.retain(|x| !ids.contains(x));
            }
            removed
        });
        if removed.is_empty() {
            return;
        }
        for v in self
            .engine
            .voices()
            .into_iter()
            .filter(|v| ids.contains(&v.sound_id))
        {
            self.engine.stop(v.id);
        }
        for s in &removed {
            self.engine.forget(&s.path);
        }
        self.save_now();
        if removed.iter().any(|s| s.hotkey.is_some()) {
            self.apply_hotkeys();
        }
    }

    /// Looks in `folder` for the files of the missing sounds (moved, or a renamed drive), by
    /// their name, and points the sounds there. Files already in the library are left alone.
    pub fn relink(&self, folder: &Path) {
        let ids = lock(&self.missing).clone();
        if ids.is_empty() {
            return;
        }
        let (missing, known): (Vec<(String, String)>, HashSet<String>) = self.read(|l| {
            (
                l.sounds
                    .iter()
                    .filter(|s| ids.contains(&s.id))
                    .map(|s| (s.id.clone(), s.path.clone()))
                    .collect(),
                l.sounds.iter().map(|s| s.path.to_lowercase()).collect(),
            )
        });
        let files: Vec<String> = store::expand_paths(&[folder.to_path_buf()])
            .into_iter()
            .filter(|f| !known.contains(&f.to_lowercase()))
            .collect();
        let found = relink(&missing, &files);
        if !found.is_empty() {
            self.edit(|lib| {
                for (id, path) in &found {
                    if let Some(s) = lib.sound_mut(id) {
                        s.path = path.clone();
                    }
                }
            });
            self.save_now();
            self.check_missing();
            self.preload_hotkey_sounds();
        }
        let left = missing.len() - found.len();
        self.toast(
            match (found.len(), left) {
                (0, _) => format!(
                    "Aucun des sons introuvables n’est dans {}",
                    folder.display()
                ),
                (n, 0) => format!("{n} son{} retrouvé{0}", if n > 1 { "s" } else { "" }),
                (n, left) => format!(
                    "{n} retrouvé{}, {left} toujours introuvable{}",
                    if n > 1 { "s" } else { "" },
                    if left > 1 { "s" } else { "" }
                ),
            },
            found.is_empty(),
        );
    }

    /// Asks GitHub for the latest release; a newer one is offered in the window. A build from
    /// the sources has no version and does not ask.
    fn check_update(&self) {
        let Some(current) = VERSION else { return };
        let Ok(response) = link::get(LATEST_RELEASE_API, 1024 * 1024) else {
            return;
        };
        let latest = serde_json::from_slice::<serde_json::Value>(&response.body)
            .ok()
            .and_then(|v| {
                v["tag_name"]
                    .as_str()
                    .map(|t| t.trim_start_matches('v').to_string())
            });
        if let Some(v) = latest.filter(|v| is_newer(v, current)) {
            *lock(&self.update) = Some(v);
            self.repaint();
        }
    }
}
