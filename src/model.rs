// The library (sounds, categories, settings) and the pure logic around it. The JSON layout is
// the one of the Electron version, so an existing library.json keeps working.
use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::keys;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Default, // the general setting
    Overlap,
    Restart,
    Toggle,
}

impl Mode {
    fn parse(v: &Value) -> Option<Mode> {
        Some(match v.as_str()? {
            "default" => Mode::Default,
            "overlap" => Mode::Overlap,
            "restart" => Mode::Restart,
            "toggle" => Mode::Toggle,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Default => "Réglage général",
            Mode::Overlap => "Le superposer (plusieurs fois en même temps)",
            Mode::Restart => "Le redémarrer depuis le début",
            Mode::Toggle => "L’arrêter (bascule lecture / arrêt)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sound {
    pub id: String,
    pub path: String,
    pub title: String,
    pub volume: f64,
    pub rate: f64,
    pub mode: Mode,
    pub hotkey: Option<String>,
    pub favorite: bool,
    pub plays: u64,
    pub duration: Option<f64>,
}

impl Sound {
    pub fn new(path: &str) -> Sound {
        Sound {
            id: new_id(),
            path: path.to_string(),
            title: clean_title(&title_from_path(path)),
            volume: 1.0,
            rate: 1.0,
            mode: Mode::Default,
            hotkey: None,
            favorite: false,
            plays: 0,
            duration: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    pub sound_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
pub struct Device {
    pub id: String, // "" = the Windows default
    pub label: String,
}

impl Device {
    pub fn named(name: &str) -> Device {
        Device {
            id: name.to_string(),
            label: name.to_string(),
        }
    }
}

pub const MIC_FILTER_ID: &str = "mic-filter";

/// Where the sounds go for the others to hear.
#[derive(Clone, Debug, PartialEq)]
pub enum Cable {
    Unset, // never chosen: detected at start-up
    Off,
    Filter, // the real microphone, through the filter (native/)
    Device(Device),
}

impl Serialize for Cable {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Cable::Unset | Cable::Off => s.serialize_none(),
            Cable::Filter => Device {
                id: MIC_FILTER_ID.into(),
                label: "Mon micro, avec le filtre".into(),
            }
            .serialize(s),
            Cable::Device(d) => d.serialize(s),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalHotkeys {
    pub stop_all: Option<String>,
    pub pause_all: Option<String>,
    pub vol_up: Option<String>,
    pub vol_down: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GlobalAction {
    StopAll,
    PauseAll,
    VolUp,
    VolDown,
}

impl GlobalAction {
    pub const ALL: [GlobalAction; 4] = [
        GlobalAction::StopAll,
        GlobalAction::PauseAll,
        GlobalAction::VolUp,
        GlobalAction::VolDown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GlobalAction::StopAll => "Tout arrêter",
            GlobalAction::PauseAll => "Pause / reprise de tout",
            GlobalAction::VolUp => "Monter le volume des sons en cours",
            GlobalAction::VolDown => "Baisser le volume des sons en cours",
        }
    }
}

impl GlobalHotkeys {
    pub fn get(&self, a: GlobalAction) -> &Option<String> {
        match a {
            GlobalAction::StopAll => &self.stop_all,
            GlobalAction::PauseAll => &self.pause_all,
            GlobalAction::VolUp => &self.vol_up,
            GlobalAction::VolDown => &self.vol_down,
        }
    }

    pub fn get_mut(&mut self, a: GlobalAction) -> &mut Option<String> {
        match a {
            GlobalAction::StopAll => &mut self.stop_all,
            GlobalAction::PauseAll => &mut self.pause_all,
            GlobalAction::VolUp => &mut self.vol_up,
            GlobalAction::VolDown => &mut self.vol_down,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub monitor_device: Device,
    #[serde(skip_serializing_if = "is_unset")]
    pub cable_device: Cable,
    pub mic_device: Device,
    pub mic_enabled: bool,
    pub mic_noise_suppression: bool,
    pub mic_volume: f64,
    pub monitor_volume: f64,
    pub cable_volume: f64,
    pub normalize: bool,
    pub default_mode: Mode,
    pub exclusive: bool,
    pub max_voices: u32,
    pub hotkeys: GlobalHotkeys,
}

fn is_unset(c: &Cable) -> bool {
    *c == Cable::Unset
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            monitor_device: Device::default(),
            cable_device: Cable::Unset,
            mic_device: Device::default(),
            mic_enabled: true,
            mic_noise_suppression: false,
            mic_volume: 1.0,
            monitor_volume: 0.8,
            cable_volume: 1.0,
            normalize: true,
            default_mode: Mode::Overlap,
            exclusive: false,
            max_voices: 16,
            hotkeys: GlobalHotkeys::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Library {
    pub version: u32,
    pub sounds: Vec<Sound>,
    pub categories: Vec<Category>,
    pub settings: Settings,
}

impl Default for Library {
    fn default() -> Library {
        Library {
            version: 1,
            sounds: Vec::new(),
            categories: Vec::new(),
            settings: Settings::default(),
        }
    }
}

impl Library {
    pub fn sound(&self, id: &str) -> Option<&Sound> {
        self.sounds.iter().find(|s| s.id == id)
    }

    pub fn sound_mut(&mut self, id: &str) -> Option<&mut Sound> {
        self.sounds.iter_mut().find(|s| s.id == id)
    }

    /// Every global hotkey the app needs, as (accelerator, vk, mods, action).
    /// Unparseable strings are skipped.
    pub fn hotkeys(&self) -> Vec<Hotkey> {
        let mut list = Vec::new();
        let mut add = |accel: &Option<String>, action: Action| {
            if let Some(accel) = accel
                && let Some((vk, mods)) = keys::parse_accelerator(accel)
            {
                list.push(Hotkey {
                    accel: accel.clone(),
                    vk,
                    mods,
                    action,
                });
            }
        };
        for s in &self.sounds {
            add(&s.hotkey, Action::Play(s.id.clone()));
        }
        for a in GlobalAction::ALL {
            add(self.settings.hotkeys.get(a), Action::Global(a));
        }
        list
    }

    /// Who already uses this hotkey, as a label, or None. `except` is the owner being edited.
    pub fn hotkey_owner(&self, accel: &str, except: &Action) -> Option<String> {
        let taken = |h: &Option<String>| h.as_deref().is_some_and(|h| keys::same_hotkey(h, accel));
        if let Some(s) = self
            .sounds
            .iter()
            .find(|s| !matches!(except, Action::Play(id) if *id == s.id) && taken(&s.hotkey))
        {
            return Some(format!("« {} »", s.title));
        }
        GlobalAction::ALL
            .into_iter()
            .find(|&a| *except != Action::Global(a) && taken(self.settings.hotkeys.get(a)))
            .map(|a| format!("« {} »", a.label()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Play(String), // sound id
    Global(GlobalAction),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hotkey {
    pub accel: String,
    pub vk: u32,
    pub mods: u32,
    pub action: Action,
}

// ---------- Untrusted JSON -> Library ----------

fn num(v: Option<&Value>, min: f64, max: f64) -> Option<f64> {
    v.and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .map(|x| x.clamp(min, max))
}

fn string(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn device(v: Option<&Value>) -> Option<Device> {
    let o = v?.as_object()?;
    Some(Device {
        id: o.get("id")?.as_str()?.to_string(),
        label: o
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn boolean(v: Option<&Value>) -> Option<bool> {
    v.and_then(Value::as_bool)
}

/// Data read from disk is untrusted: keep only well-formed fields, fill the rest with defaults.
pub fn normalize_library(raw: &Value) -> Library {
    let empty = serde_json::Map::new();
    let src = raw.as_object().unwrap_or(&empty);
    let sounds: Vec<Sound> = src
        .get("sounds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let s = s.as_object()?;
            let id = s.get("id")?.as_str()?;
            let path = s.get("path")?.as_str()?;
            Some(Sound {
                id: id.to_string(),
                path: path.to_string(),
                title: string(s.get("title"))
                    .unwrap_or_else(|| clean_title(&title_from_path(path))),
                volume: num(s.get("volume"), 0.0, 2.0).unwrap_or(1.0),
                rate: num(s.get("rate"), 0.5, 2.0).unwrap_or(1.0),
                mode: s.get("mode").and_then(Mode::parse).unwrap_or(Mode::Default),
                hotkey: string(s.get("hotkey")),
                favorite: boolean(s.get("favorite")) == Some(true),
                plays: num(s.get("plays"), 0.0, 9.007_199_254_740_991e15).unwrap_or(0.0) as u64,
                duration: num(s.get("duration"), 0.0, 86400.0),
            })
        })
        .collect();
    let categories = src
        .get("categories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let c = c.as_object()?;
            let sound_ids = c
                .get("soundIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|id| sounds.iter().any(|s| s.id == *id))
                .map(str::to_string)
                .collect();
            Some(Category {
                id: c.get("id")?.as_str()?.to_string(),
                name: c.get("name")?.as_str()?.to_string(),
                sound_ids,
            })
        })
        .collect();

    let s = src
        .get("settings")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let d = Settings::default();
    let hk = s
        .get("hotkeys")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let cable_device = match s.get("cableDevice") {
        Some(Value::Null) => Cable::Off,
        v => match device(v) {
            Some(dev) if dev.id == MIC_FILTER_ID => Cable::Filter,
            Some(dev) => Cable::Device(dev),
            None => Cable::Unset,
        },
    };
    let mode = s
        .get("defaultMode")
        .and_then(Mode::parse)
        .filter(|m| *m != Mode::Default);
    let settings = Settings {
        monitor_device: device(s.get("monitorDevice")).unwrap_or(d.monitor_device),
        cable_device,
        mic_device: device(s.get("micDevice")).unwrap_or(d.mic_device),
        mic_enabled: boolean(s.get("micEnabled")).unwrap_or(d.mic_enabled),
        mic_noise_suppression: boolean(s.get("micNoiseSuppression")) == Some(true),
        mic_volume: num(s.get("micVolume"), 0.0, 2.0).unwrap_or(d.mic_volume),
        monitor_volume: num(s.get("monitorVolume"), 0.0, 2.0).unwrap_or(d.monitor_volume),
        cable_volume: num(s.get("cableVolume"), 0.0, 2.0).unwrap_or(d.cable_volume),
        normalize: boolean(s.get("normalize")).unwrap_or(d.normalize),
        default_mode: mode.unwrap_or(d.default_mode),
        exclusive: boolean(s.get("exclusive")) == Some(true),
        max_voices: num(s.get("maxVoices"), 1.0, 64.0).map_or(d.max_voices, |n| n.round() as u32),
        hotkeys: GlobalHotkeys {
            stop_all: string(hk.get("stopAll")),
            pause_all: string(hk.get("pauseAll")),
            vol_up: string(hk.get("volUp")),
            vol_down: string(hk.get("volDown")),
        },
    };
    Library {
        version: 1,
        sounds,
        categories,
        settings,
    }
}

// ---------- Titles, search, time ----------

pub fn title_from_path(p: &str) -> String {
    let base = p.rsplit(['\\', '/']).next().unwrap_or("");
    match base.rfind('.') {
        Some(i) if i + 1 < base.len() => base[..i].to_string(),
        _ => base.to_string(),
    }
}

/// "kirby-falling-meme-scream-made-with-Voicemod" -> "kirby falling meme scream"
pub fn clean_title(raw: &str) -> String {
    let original = raw.trim();
    let mut t = original;
    // [-_ ]*made[-_ ]with[-_ ]voicemod at the end, any case.
    let lower = t.to_lowercase();
    if lower.len() == t.len() {
        for sep1 in ['-', '_', ' '] {
            for sep2 in ['-', '_', ' '] {
                let suffix = format!("made{sep1}with{sep2}voicemod");
                if lower.ends_with(&suffix) {
                    t = t[..t.len() - suffix.len()].trim_end_matches(['-', '_', ' ']);
                }
            }
        }
    }
    let mut t = t.trim().to_string();
    if !t.chars().any(char::is_whitespace) {
        t = t
            .split(['-', '_'])
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    if t.is_empty() {
        original.to_string()
    } else {
        t
    }
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        _ => c,
    }
}

/// Case- and accent-insensitive form used by the search box ("Même" matches "meme").
pub fn search_key(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .map(fold_accent)
        .collect()
}

pub fn format_time(sec: f64) -> String {
    let s = if sec.is_finite() && sec > 0.0 {
        sec.floor() as u64
    } else {
        0
    };
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn parse_duration(text: &str) -> Option<f64> {
    text.split(':').try_fold(0.0, |acc, p| {
        p.trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| acc * 60.0 + n)
    })
}

// ---------- Loudness ----------

/// Peak and gated RMS of interleaved samples (50 ms blocks, blocks quieter than -50 dBFS
/// ignored, so silence around a short sound does not drag its loudness down).
pub fn analyse(samples: &[f32], channels: usize, rate: u32) -> (f32, f32) {
    let frames = samples.len() / channels.max(1);
    let block = ((rate as f64 * 0.05).round() as usize).max(1);
    let gate = 10f64.powf(-50.0 / 10.0);
    let mut peak = 0f32;
    let (mut gated_sum, mut gated_blocks) = (0f64, 0u32);
    let mut start = 0;
    while start < frames {
        let end = frames.min(start + block);
        let chunk = &samples[start * channels..end * channels];
        let mut sum = 0f64;
        for &v in chunk {
            sum += f64::from(v) * f64::from(v);
            peak = peak.max(v.abs());
        }
        let mean_square = sum / chunk.len() as f64;
        if mean_square > gate {
            gated_sum += mean_square;
            gated_blocks += 1;
        }
        start = end;
    }
    let rms = if gated_blocks > 0 {
        (gated_sum / f64::from(gated_blocks)).sqrt() as f32
    } else {
        0.0
    };
    (peak, rms)
}

/// Gain that brings a sound to -18 dBFS RMS without pushing its peak past 0 dBFS, capped at +12 dB.
pub fn normalize_gain(peak: f32, rms: f32) -> f32 {
    if rms.is_nan() || peak.is_nan() || rms <= 0.0 || peak <= 0.0 {
        return 1.0;
    }
    (10f32.powf(-18.0 / 20.0) / rms).min(1.0 / peak).min(4.0)
}

// ---------- Playback decisions ----------

pub fn effective_mode(sound: &Sound, settings: &Settings) -> Mode {
    if sound.mode == Mode::Default {
        settings.default_mode
    } else {
        sound.mode
    }
}

/// Which voices to stop, and whether to start a new one, when a sound is triggered.
pub fn plan_play(mode: Mode, same_sound: &[u64], all: &[u64], exclusive: bool) -> (Vec<u64>, bool) {
    if mode == Mode::Toggle && !same_sound.is_empty() {
        return (same_sound.to_vec(), false);
    }
    if exclusive {
        return (all.to_vec(), true);
    }
    if mode == Mode::Restart {
        return (same_sound.to_vec(), true);
    }
    (Vec::new(), true)
}

// ---------- Devices ----------

/// Saved device -> current device name. "" is the Windows default. Names are compared to
/// the id first, then the label (ids saved by the Electron version were Chromium hashes).
pub fn resolve_device(names: &[String], saved: &Device) -> Option<String> {
    if saved.id.is_empty() {
        return Some(String::new());
    }
    names
        .iter()
        .find(|n| **n == saved.id)
        .or_else(|| names.iter().find(|n| **n == saved.label))
        .cloned()
}

pub fn is_virtual_cable(name: &str) -> bool {
    let n = name.to_lowercase();
    ["cable input", "vb-audio", "voicemeeter input"]
        .iter()
        .any(|k| n.contains(k))
}

/// The capture side of a virtual cable: mixing it back in would loop the sounds.
pub fn is_cable_capture(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("cable output") || n.contains("vb-audio")
}

// ---------- Microphone filter (native/) ----------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterStatus {
    pub ok: bool,
    pub error: String,
    pub dll: bool,
    pub installed: Vec<String>,
    pub soundpad: Vec<String>,
    pub eligible: Vec<String>,
}

/// What the app sees of the filter's ring (native/shared.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterLive {
    NoMap,     // not loaded: not installed, or no app records the microphone
    Idle,      // loaded, but no microphone processed for a second
    Live(u32), // a microphone at this rate receives the sounds
}

pub fn filter_error(code: &str) -> String {
    match code {
        "not-admin" => "Windows n’a pas donné les droits administrateur.".into(),
        "not-built" => "Filtre absent de cette version : compile native\\build.cmd (g++ de MSYS2), puis l’appli.".into(),
        "no-mic" => "Aucun micro compatible.".into(),
        "cancelled" => "Autorisation refusée.".into(),
        "tampered" => "Le script du filtre a été modifié sur le disque avant de s’exécuter : rien n’a été fait.".into(),
        "bad-dll" => "Le filtre a été modifié avant d’être installé : installation annulée.".into(),
        other => other.into(),
    }
}

fn quoted_list(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("« {n} »"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Settings text about the filter. status: None while unknown.
pub fn filter_message(status: Option<&FilterStatus>, live: FilterLive) -> String {
    let Some(status) = status else {
        return "Vérification du filtre…".into();
    };
    if !status.ok {
        return format!("État du filtre inconnu : {}", filter_error(&status.error));
    }
    if status.installed.is_empty() {
        if status.eligible.is_empty() {
            return "Aucun micro compatible : le filtre se pose sur un micro branché qui n’a pas déjà un effet du fabricant.".into();
        }
        let soundpad = if status.soundpad.is_empty() {
            ""
        } else {
            " Il remplace celui de Soundpad : Soundpad n’enverra plus de son tant que ce filtre est là, et le retirer remet celui de Soundpad."
        };
        return format!(
            "Filtre non installé. Il se posera sur {}. Windows demandera l’autorisation, et le son du PC se coupera une seconde.{soundpad}",
            quoted_list(&status.eligible)
        );
    }
    let at = quoted_list(&status.installed);
    match live {
        FilterLive::Live(_) => {
            format!("Actif sur {at} : toutes les applis qui écoutent ce micro entendent les sons.")
        }
        FilterLive::Idle => {
            format!("Installé sur {at}. Aucune appli n’écoute ce micro en ce moment.")
        }
        FilterLive::NoMap => format!(
            "Installé sur {at}. Il se met en marche dès qu’une appli (Discord, un jeu…) écoute ce micro."
        ),
    }
}

// ---------- Ids ----------

pub fn new_id() -> String {
    // A random GUID, written like crypto.randomUUID().
    let g = unsafe { windows::Win32::System::Com::CoCreateGuid() }.unwrap_or_default();
    let d4 = g.data4;
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1, g.data2, g.data3, d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clean_title_strips_the_voicemod_suffix_and_slug_dashes() {
        assert_eq!(
            clean_title("kirby-falling-meme-scream-made-with-Voicemod"),
            "kirby falling meme scream"
        );
        assert_eq!(
            clean_title("3fs-fart-(slow)-made-with-Voicemod"),
            "3fs fart (slow)"
        );
        assert_eq!(clean_title("El Azhar - El Djihad"), "El Azhar - El Djihad");
        assert_eq!(clean_title("made-with-Voicemod"), "made-with-Voicemod");
        assert_eq!(
            title_from_path("C:\\soundpad\\ba dum tss.mp3"),
            "ba dum tss"
        );
        assert_eq!(title_from_path("/x/y/file.name.ogg"), "file.name");
    }

    #[test]
    fn time_and_search_helpers() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(203.9), "3:23");
        assert_eq!(format_time(f64::NAN), "0:00");
        assert_eq!(parse_duration("3:23"), Some(203.0));
        assert_eq!(parse_duration("1:02:03"), Some(3723.0));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(search_key("Même PAS"), "meme pas");
        assert_eq!(search_key("E\u{301}te\u{301}"), "ete");
    }

    #[test]
    fn hotkeys_list_sounds_and_global_actions_skipping_invalid_strings() {
        let lib = normalize_library(&json!({
            "sounds": [
                { "id": "a", "path": "C:\\a.mp3", "hotkey": "Pause" },
                { "id": "b", "path": "C:\\b.mp3", "hotkey": "Bogus+Key" },
                { "id": "c", "path": "C:\\c.mp3" },
            ],
            "settings": { "hotkeys": { "stopAll": "Ctrl+num0" } },
        }));
        assert_eq!(
            lib.hotkeys(),
            vec![
                Hotkey {
                    accel: "Pause".into(),
                    vk: 19,
                    mods: 0,
                    action: Action::Play("a".into())
                },
                Hotkey {
                    accel: "Ctrl+num0".into(),
                    vk: 96,
                    mods: 2,
                    action: Action::Global(GlobalAction::StopAll)
                },
            ]
        );
        assert_eq!(
            lib.hotkey_owner("pause", &Action::Global(GlobalAction::VolUp))
                .as_deref(),
            Some("« a »")
        );
        assert_eq!(lib.hotkey_owner("Pause", &Action::Play("a".into())), None);
        assert_eq!(
            lib.hotkey_owner("Control+num0", &Action::Play("a".into()))
                .as_deref(),
            Some("« Tout arrêter »")
        );
    }

    fn sine(amp: f32, seconds: usize) -> Vec<f32> {
        (0..seconds * 48000)
            .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin())
            .collect()
    }

    #[test]
    fn analyse_measures_peak_and_gated_rms_ignoring_silence() {
        let tone = sine(0.5, 1);
        let stereo: Vec<f32> = tone.iter().flat_map(|&v| [v, v]).collect();
        let (peak, rms) = analyse(&stereo, 2, 48000);
        assert!((peak - 0.5).abs() < 1e-3);
        assert!((rms - 0.5 / 2f32.sqrt()).abs() < 2e-3);
        let mut padded = vec![0f32; 48000 * 4];
        padded[48000..96000].copy_from_slice(&tone);
        assert!(
            (analyse(&padded, 1, 48000).1 - rms).abs() < 2e-3,
            "silence must not lower loudness"
        );
        assert_eq!(analyse(&[0.0; 1000], 1, 48000), (0.0, 0.0));
        assert_eq!(analyse(&[], 2, 48000), (0.0, 0.0));
    }

    #[test]
    fn normalize_gain_targets_minus_18_dbfs_without_clipping_within_12_db() {
        let target = 10f32.powf(-18.0 / 20.0);
        assert!((normalize_gain(0.5, target) - 1.0).abs() < 1e-6);
        assert!(normalize_gain(1.0, 0.5) < 0.3);
        assert_eq!(normalize_gain(0.8, 0.01), 1.0 / 0.8);
        assert_eq!(normalize_gain(0.01, 0.001), 4.0);
        assert_eq!(normalize_gain(0.0, 0.0), 1.0);
    }

    #[test]
    fn plan_play_covers_every_mode() {
        assert_eq!(
            plan_play(Mode::Overlap, &[1], &[1, 2], false),
            (vec![], true)
        );
        assert_eq!(
            plan_play(Mode::Restart, &[1], &[1, 2], false),
            (vec![1], true)
        );
        assert_eq!(
            plan_play(Mode::Toggle, &[1], &[1, 2], false),
            (vec![1], false)
        );
        assert_eq!(plan_play(Mode::Toggle, &[], &[2], false), (vec![], true));
        assert_eq!(
            plan_play(Mode::Overlap, &[], &[2, 3], true),
            (vec![2, 3], true)
        );
        assert_eq!(
            plan_play(Mode::Toggle, &[3], &[2, 3], true),
            (vec![3], false)
        );
        let s = Settings {
            default_mode: Mode::Restart,
            ..Settings::default()
        };
        let mut sound = Sound::new("C:\\a.mp3");
        assert_eq!(effective_mode(&sound, &s), Mode::Restart);
        sound.mode = Mode::Toggle;
        assert_eq!(effective_mode(&sound, &s), Mode::Toggle);
    }

    #[test]
    fn resolve_device_falls_back_to_the_label() {
        let names = vec![
            "Casque (TONOR)".to_string(),
            "CABLE Input (VB-Audio Virtual Cable)".to_string(),
        ];
        let dev = |id: &str, label: &str| Device {
            id: id.into(),
            label: label.into(),
        };
        assert_eq!(
            resolve_device(&names, &dev("Casque (TONOR)", "x")).as_deref(),
            Some("Casque (TONOR)")
        );
        assert_eq!(
            resolve_device(&names, &dev("chromium-hash", "Casque (TONOR)")).as_deref(),
            Some("Casque (TONOR)")
        );
        assert_eq!(resolve_device(&names, &dev("", "")).as_deref(), Some(""));
        assert_eq!(resolve_device(&names, &dev("gone", "gone")), None);
        assert!(is_virtual_cable(&names[1]) && !is_virtual_cable(&names[0]));
    }

    #[test]
    fn filter_message_follows_install_state_then_the_ring() {
        let mic = "Microphone (TONOR TD510 Dynamic Mic)".to_string();
        let base = FilterStatus {
            ok: true,
            eligible: vec![mic.clone()],
            ..Default::default()
        };
        assert!(filter_message(None, FilterLive::NoMap).contains("Vérification"));
        let refused = FilterStatus {
            error: "cancelled".into(),
            ..Default::default()
        };
        assert!(filter_message(Some(&refused), FilterLive::NoMap).contains("Autorisation refusée"));
        let none = FilterStatus {
            eligible: vec![],
            ..base.clone()
        };
        assert!(filter_message(Some(&none), FilterLive::NoMap).contains("Aucun micro compatible"));
        let fresh = filter_message(Some(&base), FilterLive::NoMap);
        assert!(
            fresh.contains("non installé")
                && fresh.contains("TONOR")
                && !fresh.contains("Soundpad")
        );
        let sp = FilterStatus {
            soundpad: vec![mic.clone()],
            ..base.clone()
        };
        assert!(
            filter_message(Some(&sp), FilterLive::NoMap).contains("remplace celui de Soundpad")
        );
        let on = FilterStatus {
            dll: true,
            installed: vec![mic],
            ..base
        };
        assert!(
            filter_message(Some(&on), FilterLive::Live(48000))
                .starts_with("Actif sur « Microphone (TONOR")
        );
        assert!(filter_message(Some(&on), FilterLive::Idle).contains("Aucune appli"));
        assert!(filter_message(Some(&on), FilterLive::NoMap).contains("dès qu’une appli"));
        assert!(filter_error("not-built").contains("build.cmd"));
        assert_eq!(filter_error("weird"), "weird");
    }

    #[test]
    fn normalize_library_keeps_valid_data_and_repairs_the_rest() {
        let empty = normalize_library(&Value::Null);
        assert!(empty.sounds.is_empty());
        assert_eq!(empty.settings.cable_device, Cable::Unset);
        assert_eq!(empty.settings.default_mode, Mode::Overlap);

        let lib = normalize_library(&json!({
            "sounds": [
                { "id": "a", "path": "C:\\a.mp3", "volume": 9, "rate": 0.1, "mode": "weird", "plays": -3, "title": "" },
                { "id": "b" },
                "junk",
            ],
            "categories": [{ "id": "c", "name": "Mes sons", "soundIds": ["a", "ghost"] }, { "name": "no id" }],
            "settings": { "cableDevice": null, "maxVoices": 500, "hotkeys": { "stopAll": "Pause", "bogus": "F1" } },
        }));
        assert_eq!(lib.sounds.len(), 1);
        let a = &lib.sounds[0];
        assert_eq!(
            (a.volume, a.rate, a.mode, a.plays, a.title.as_str()),
            (2.0, 0.5, Mode::Default, 0, "a")
        );
        assert_eq!(
            lib.categories,
            vec![Category {
                id: "c".into(),
                name: "Mes sons".into(),
                sound_ids: vec!["a".into()]
            }]
        );
        assert_eq!(lib.settings.cable_device, Cable::Off);
        assert_eq!(lib.settings.max_voices, 64);
        assert_eq!(
            lib.settings.hotkeys,
            GlobalHotkeys {
                stop_all: Some("Pause".into()),
                ..Default::default()
            }
        );
    }

    #[test]
    fn the_library_round_trips_through_json() {
        let mut lib = Library::default();
        lib.sounds.push(Sound::new("C:\\x\\ba dum tss.mp3"));
        lib.settings.cable_device = Cable::Filter;
        lib.settings.hotkeys.vol_up = Some("Ctrl+Up".into());
        let text = serde_json::to_string(&lib).unwrap();
        assert_eq!(
            normalize_library(&serde_json::from_str(&text).unwrap()),
            lib
        );
        // Unset is written as an absent key, Off as null: the two must not merge.
        let unset = serde_json::to_value(Library::default()).unwrap();
        assert!(unset["settings"].get("cableDevice").is_none());
        lib.settings.cable_device = Cable::Off;
        assert!(serde_json::to_value(&lib).unwrap()["settings"]["cableDevice"].is_null());
    }
}
