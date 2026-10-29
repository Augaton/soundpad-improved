// Files: the library, Soundpad's list, and the audio files dropped or picked.
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

use serde_json::Value;

use crate::model::{Library, normalize_library};

pub const AUDIO_EXT: [&str; 9] = [
    "mp3", "wav", "ogg", "oga", "m4a", "aac", "flac", "webm", "aiff",
];
const MAX_SCAN_FILES: usize = 5000;

fn app_data() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// %APPDATA%\soundpad-improved, the folder the Electron version used. SPI_USER_DATA isolates
/// a test profile.
pub fn data_dir() -> PathBuf {
    std::env::var_os("SPI_USER_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| app_data().join("soundpad-improved"))
}

fn library_file() -> PathBuf {
    data_dir().join("library.json")
}

pub fn soundpad_file() -> PathBuf {
    app_data().join("Leppsoft").join("soundlist.spl")
}

pub struct Loaded {
    pub lib: Library,
    pub warning: Option<String>,
    pub writable: bool, // false: library.json is still there, unread; saving would erase it
}

/// The library. A file that is not JSON is kept aside; one that cannot be read at all (locked,
/// no access) stays where it is and nothing is saved over it.
pub fn load_library() -> Loaded {
    let file = library_file();
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Loaded {
                lib: Library::default(),
                warning: None,
                writable: true,
            };
        }
        Err(e) => {
            let warning = format!(
                "Bibliothèque illisible ({e}) : rien ne sera enregistré, pour ne pas l’écraser. Ferme et relance l’appli."
            );
            return Loaded {
                lib: Library::default(),
                warning: Some(warning),
                writable: false,
            };
        }
    };
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(v) => Loaded {
            lib: normalize_library(&v),
            warning: None,
            writable: true,
        },
        Err(_) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let aside = file.with_file_name(format!("library.illisible-{stamp}.json"));
            match fs::rename(&file, &aside) {
                Ok(()) => Loaded {
                    lib: Library::default(),
                    warning: Some(format!(
                        "Bibliothèque illisible, mise de côté dans {}",
                        aside.display()
                    )),
                    writable: true,
                },
                Err(e) => Loaded {
                    lib: Library::default(),
                    warning: Some(format!(
                        "Bibliothèque illisible ({e}) : rien ne sera enregistré."
                    )),
                    writable: false,
                },
            }
        }
    }
}

/// Written to a temp file first, so a crash never leaves half a library.
pub fn save_library(lib: &Library) -> io::Result<()> {
    let file = library_file();
    fs::create_dir_all(data_dir())?;
    let text = serde_json::to_string_pretty(lib).map_err(io::Error::other)?;
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, &file)
}

pub fn read_text(path: &Path) -> io::Result<String> {
    let text = fs::read_to_string(path)?;
    Ok(text
        .strip_prefix('\u{feff}')
        .map(str::to_string)
        .unwrap_or(text))
}

/// An audio file the app may open: on a drive (C:\...) or a network share (\\server\share\...),
/// with an audio extension. Device paths (\\.\pipe\x.mp3, \\?\...) are refused: opening one
/// from a crafted library could hang a thread or reach a device.
pub fn is_audio_path(p: &Path) -> bool {
    let local_or_share = matches!(p.components().next(), Some(Component::Prefix(px)) if matches!(px.kind(), Prefix::Disk(_) | Prefix::UNC(..)));
    local_or_share
        && p.is_absolute()
        && p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| AUDIO_EXT.contains(&e.to_lowercase().as_str()))
}

/// Dropped or picked paths -> audio files, folders walked six levels deep.
pub fn expand_paths(paths: &[PathBuf]) -> Vec<String> {
    fn walk(p: &Path, depth: u32, out: &mut Vec<String>) {
        if out.len() >= MAX_SCAN_FILES {
            return;
        }
        let Ok(meta) = fs::metadata(p) else { return };
        if meta.is_file() && is_audio_path(p) {
            out.push(p.to_string_lossy().into_owned());
        } else if meta.is_dir() && depth < 6 {
            let mut entries: Vec<PathBuf> = fs::read_dir(p)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            for e in entries {
                walk(&e, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths.iter().filter(|p| p.is_absolute()) {
        walk(p, 0, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_walks_folders_and_keeps_audio_only() {
        let dir = std::env::temp_dir().join(format!("spi-expand-{}", std::process::id()));
        fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["a.MP3", "notes.txt", "sub/b.ogg"] {
            fs::write(dir.join(f), b"x").unwrap();
        }
        let found = expand_paths(std::slice::from_ref(&dir));
        assert_eq!(found.len(), 2);
        assert!(found[0].ends_with("a.MP3") && found[1].ends_with("b.ogg"));
        assert!(expand_paths(&[PathBuf::from("relative.mp3")]).is_empty());
        assert!(
            is_audio_path(Path::new(r"C:\a\b.Mp3"))
                && is_audio_path(Path::new(r"\\nas\sons\b.wav"))
        );
        for refused in [
            r"\\.\pipe\x.mp3",
            r"\\?\C:\a.mp3",
            r"C:\a\b.exe",
            r"\a.mp3",
            r"C:a.mp3",
        ] {
            assert!(!is_audio_path(Path::new(refused)), "{refused}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
