// Hotkeys are stored as readable strings ("Ctrl+Shift+num1") and registered as Win32
// {vk, mods} pairs. Modifiers use RegisterHotKey's bits, as Soundpad stores them:
// 1 Alt, 2 Ctrl, 4 Shift, 8 Win.

pub const MOD_ALT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

fn vk_name(vk: u32) -> Option<&'static str> {
    Some(match vk {
        8 => "Backspace",
        9 => "Tab",
        13 => "Enter",
        19 => "Pause",
        20 => "Capslock",
        27 => "Esc",
        32 => "Space",
        33 => "PageUp",
        34 => "PageDown",
        35 => "End",
        36 => "Home",
        37 => "Left",
        38 => "Up",
        39 => "Right",
        40 => "Down",
        44 => "PrintScreen",
        45 => "Insert",
        46 => "Delete",
        106 => "nummult",
        107 => "numadd",
        109 => "numsub",
        110 => "numdec",
        111 => "numdiv",
        144 => "Numlock",
        145 => "Scrolllock",
        173 => "VolumeMute",
        174 => "VolumeDown",
        175 => "VolumeUp",
        176 => "MediaNextTrack",
        177 => "MediaPreviousTrack",
        178 => "MediaStop",
        179 => "MediaPlayPause",
        186 => ";",
        187 => "=",
        188 => ",",
        189 => "-",
        190 => ".",
        191 => "/",
        192 => "`",
        219 => "[",
        220 => "\\",
        221 => "]",
        222 => "'",
        _ => return None,
    })
}

/// Windows virtual-key code -> key name, or None when unsupported.
pub fn vk_to_key(vk: u32) -> Option<String> {
    match vk {
        48..=57 | 65..=90 => Some(char::from(vk as u8).to_string()),
        96..=105 => Some(format!("num{}", vk - 96)),
        112..=135 => Some(format!("F{}", vk - 111)),
        _ => vk_name(vk).map(str::to_string),
    }
}

pub fn vk_to_accelerator(vk: u32, mods: u32) -> Option<String> {
    let key = vk_to_key(vk)?;
    let mut parts = Vec::new();
    if mods & MOD_CTRL != 0 {
        parts.push("Ctrl".to_string());
    }
    if mods & MOD_ALT != 0 {
        parts.push("Alt".to_string());
    }
    if mods & MOD_SHIFT != 0 {
        parts.push("Shift".to_string());
    }
    if mods & MOD_WIN != 0 {
        parts.push("Super".to_string());
    }
    parts.push(key);
    Some(parts.join("+"))
}

/// "Ctrl+Shift+num1" -> (97, 6), or None when the string is not a valid hotkey.
pub fn parse_accelerator(accel: &str) -> Option<(u32, u32)> {
    let mut parts: Vec<&str> = accel.split('+').collect();
    let last = parts.pop()?.to_lowercase();
    // Highest code first, so a name shared by two codes resolves like the JS version did.
    let vk = (1..256)
        .rev()
        .find(|&vk| vk_to_key(vk).is_some_and(|k| k.to_lowercase() == last))?;
    let mut mods = 0;
    for p in parts {
        mods |= match p.to_lowercase().as_str() {
            "alt" => MOD_ALT,
            "ctrl" | "control" => MOD_CTRL,
            "shift" => MOD_SHIFT,
            "super" | "win" => MOD_WIN,
            _ => return None,
        };
    }
    Some((vk, mods))
}

pub fn same_hotkey(a: &str, b: &str) -> bool {
    matches!((parse_accelerator(a), parse_accelerator(b)), (Some(x), Some(y)) if x == y)
}

fn key_label(k: &str) -> String {
    let label = match k {
        "Shift" => "Maj",
        "Super" => "Win",
        "nummult" => "Num *",
        "numadd" => "Num +",
        "numsub" => "Num -",
        "numdec" => "Num .",
        "numdiv" => "Num /",
        "Up" => "Haut",
        "Down" => "Bas",
        "Left" => "Gauche",
        "Right" => "Droite",
        "Delete" => "Suppr",
        "Insert" => "Inser",
        "PageUp" => "Pg Préc",
        "PageDown" => "Pg Suiv",
        "Home" => "Début",
        "End" => "Fin",
        "Space" => "Espace",
        "Enter" => "Entrée",
        "Esc" => "Échap",
        "Backspace" => "Retour",
        "Scrolllock" => "Arrêt défil",
        "PrintScreen" => "Impr écran",
        "Capslock" => "Verr maj",
        "Numlock" => "Verr num",
        _ => {
            return match k.strip_prefix("num") {
                Some(d) if d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()) => {
                    format!("Num {d}")
                }
                _ => k.to_string(),
            };
        }
    };
    label.to_string()
}

/// Display form: "Ctrl+Shift+num1" -> "Ctrl + Maj + Num 1".
pub fn format_accelerator(accel: &str) -> String {
    accel
        .split('+')
        .map(key_label)
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_key_codes_become_hotkey_strings() {
        assert_eq!(vk_to_accelerator(19, 0).as_deref(), Some("Pause"));
        assert_eq!(vk_to_accelerator(97, 6).as_deref(), Some("Ctrl+Shift+num1"));
        assert_eq!(vk_to_accelerator(65, 1).as_deref(), Some("Alt+A"));
        assert_eq!(vk_to_accelerator(124, 8).as_deref(), Some("Super+F13"));
        assert_eq!(vk_to_accelerator(255, 0), None);
    }

    #[test]
    fn parse_inverts_format_for_every_key_and_modifier_set() {
        for vk in 1..256 {
            if vk_to_key(vk).is_none() {
                continue;
            }
            for mods in 0..16 {
                let accel = vk_to_accelerator(vk, mods).unwrap();
                assert_eq!(parse_accelerator(&accel), Some((vk, mods)), "{accel}");
            }
        }
        assert_eq!(parse_accelerator("Ctrl+Nope"), None);
        assert_eq!(parse_accelerator("Hyper+A"), None);
        assert_eq!(parse_accelerator(""), None);
    }

    #[test]
    fn display_labels_are_french() {
        assert_eq!(format_accelerator("Ctrl+Shift+num1"), "Ctrl + Maj + Num 1");
        assert_eq!(format_accelerator("numadd"), "Num +");
        assert!(same_hotkey("ctrl+a", "Control+A"));
        assert!(!same_hotkey("Ctrl+A", "A"));
    }
}
