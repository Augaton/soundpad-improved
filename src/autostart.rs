// Starting with Windows: a value in the current user's Run key, which launches the exe hidden
// in the notification area.
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("Bruitage");
const FORMER_NAME: PCWSTR = w!("Soundpad Improved"); // the app's name before 0.4
/// On the command line: start hidden in the notification area.
pub const HIDDEN: &str = "--tray";

fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {HIDDEN}", exe.display()))
}

fn registered(name: PCWSTR) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = size_of_val(&buf) as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    let text = String::from_utf16_lossy(&buf[..size as usize / 2]);
    Some(text.trim_end_matches('\0').to_string())
}

pub fn enabled() -> bool {
    registered(NAME).is_some()
}

pub fn set(on: bool) -> Result<(), String> {
    let result: WIN32_ERROR = if on {
        let command: Vec<u16> = command()
            .ok_or("chemin de l’appli introuvable")?
            .encode_utf16()
            .chain([0])
            .collect();
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                NAME,
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                (command.len() * 2) as u32,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) }
    };
    result.ok().map_err(|e| e.message())
}

/// The exe is portable, and each version is a new file: the entry follows the one that runs.
/// An entry under the app's former name becomes one under its name.
pub fn follow_exe() {
    let former = registered(FORMER_NAME).is_some();
    if former {
        let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, FORMER_NAME) };
    }
    if let Some(current) = command()
        && (former || registered(NAME).is_some_and(|r| r != current))
    {
        let _ = set(true);
    }
}
