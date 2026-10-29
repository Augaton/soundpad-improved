// The microphone filter (native/): status, install and uninstall, through native/mic-filter.ps1.
// Install and uninstall need an administrator: they go through Windows' UAC prompt and leave
// their JSON answer in a file.
//
// The script and the filter DLL are embedded in the exe and written, for each run, to a new
// folder of the user's temp directory, which anything running as the user can change. So
// PowerShell is started from System32 by its full path; it runs the script only if it has the
// SHA-256 of the embedded one, from the very bytes it checked; and the installer checks the DLL
// again once copied where only administrators can write, before Windows' audio engine loads it.
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{PCWSTR, w};

use crate::model::FilterStatus;

const SCRIPT: &str = include_str!("../native/mic-filter.ps1");
#[cfg(apo_dll)]
const DLL: Option<&[u8]> = Some(include_bytes!(env!("SPI_APO_DLL")));
#[cfg(not(apo_dll))]
const DLL: Option<&[u8]> = None; // not built: the script answers "not-built"

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TAMPERED: u32 = 3; // the launcher's exit code when the script on disk is not ours

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilterAction {
    Install,
    Uninstall,
}

/// SHA-256 in upper-case hex, as Get-FileHash writes it.
fn sha256(data: &[u8]) -> String {
    let mut out = [0u8; 32];
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    assert!(status.is_ok(), "SHA-256 indisponible ({status:?})");
    out.iter().map(|b| format!("{b:02X}")).collect()
}

/// System32's PowerShell, never one found next to the exe or on the PATH.
fn powershell() -> PathBuf {
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    let system = if n > 0 && n < buf.len() {
        PathBuf::from(String::from_utf16_lossy(&buf[..n]))
    } else {
        PathBuf::from(r"C:\Windows\System32")
    };
    system.join(r"WindowsPowerShell\v1.0\powershell.exe")
}

/// A folder made for one run: the script, and the DLL when installing.
struct Staged {
    dir: PathBuf,
    script: PathBuf,
    dll: Option<PathBuf>,
}

impl Staged {
    fn new(with_dll: bool) -> Result<Staged, String> {
        let dir =
            std::env::temp_dir().join(format!("soundpad-improved-{}", crate::model::new_id()));
        fs::create_dir(&dir).map_err(|e| e.to_string())?; // fails if it exists: this one is ours
        let mut staged = Staged {
            script: dir.join("mic-filter.ps1"),
            dll: None,
            dir,
        };
        fs::write(&staged.script, SCRIPT).map_err(|e| e.to_string())?;
        if let Some(bytes) = DLL.filter(|_| with_dll) {
            let dll = staged.dir.join("SoundpadImprovedApo.dll");
            fs::write(&dll, bytes).map_err(|e| e.to_string())?;
            staged.dll = Some(dll);
        }
        Ok(staged)
    }

    /// PowerShell's -Command: reads the script once, checks its hash, runs those bytes.
    fn command(&self, args: &str) -> String {
        format!(
            "$b=[IO.File]::ReadAllBytes({});\
             if([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($b)).Replace('-','') -ne '{}'){{exit {TAMPERED}}};\
             & ([ScriptBlock]::Create([Text.Encoding]::UTF8.GetString($b))) {args}",
            quote(&self.script),
            sha256(SCRIPT.as_bytes()),
        )
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A PowerShell single-quoted string.
fn quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "''"))
}

fn failed(error: &str) -> FilterStatus {
    FilterStatus {
        ok: false,
        error: error.into(),
        ..Default::default()
    }
}

fn parse(text: &str) -> FilterStatus {
    let text = text.trim().trim_start_matches('\u{feff}');
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return failed(if text.is_empty() { "no-result" } else { text });
    };
    // ConvertTo-Json writes a one-item list as a plain string on some PowerShell versions.
    let list = |k: &str| match &v[k] {
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    };
    FilterStatus {
        ok: v["ok"].as_bool() == Some(true),
        error: v["error"].as_str().unwrap_or_default().to_string(),
        dll: v["dll"].as_bool() == Some(true),
        installed: list("installed"),
        soundpad: list("soundpad"),
        eligible: list("eligible"),
    }
}

fn status_from(staged: &Staged) -> FilterStatus {
    let out = Command::new(powershell())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &staged.command("-Action status"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) if o.status.code() == Some(TAMPERED as i32) => failed("tampered"),
        Ok(o) => parse(&String::from_utf8_lossy(&o.stdout)),
        Err(e) => failed(&e.to_string()),
    }
}

/// Which microphones have the filter. Runs as the user, takes about a second.
pub fn status() -> FilterStatus {
    match Staged::new(false) {
        Ok(staged) => status_from(&staged),
        Err(e) => failed(&e),
    }
}

/// Installs or removes the filter, as administrator. Blocks until the script is done.
pub fn run(action: FilterAction) -> FilterStatus {
    let staged = match Staged::new(action == FilterAction::Install) {
        Ok(s) => s,
        Err(e) => return failed(&e),
    };
    let out = staged.dir.join("result.json");
    let mut args = format!(
        "-Action {} -Out {}",
        if action == FilterAction::Install {
            "install"
        } else {
            "uninstall"
        },
        quote(&out)
    );
    if let (Some(dll), Some(bytes)) = (&staged.dll, DLL) {
        args += &format!(" -Source {} -SourceHash '{}'", quote(dll), sha256(bytes));
    }
    let params = format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -Command \"{}\"",
        staged.command(&args)
    );
    match run_elevated(&params) {
        None => failed("cancelled"), // the UAC prompt was declined
        Some(TAMPERED) => failed("tampered"),
        Some(_) => match fs::read_to_string(&out) {
            Ok(text) => parse(&text),
            Err(_) => failed("no-result"),
        },
    }
}

/// Runs PowerShell through the UAC prompt; its exit code, or None when refused.
fn run_elevated(params: &str) -> Option<u32> {
    let file: Vec<u16> = powershell()
        .to_string_lossy()
        .encode_utf16()
        .chain([0])
        .collect();
    let params: Vec<u16> = params.encode_utf16().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if ShellExecuteExW(&mut info).is_err() || info.hProcess.is_invalid() {
            return None;
        }
        let mut code = 0u32;
        let done = WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0
            && GetExitCodeProcess(info.hProcess, &mut code).is_ok();
        let _ = CloseHandle(info.hProcess);
        done.then_some(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_script_answer() {
        let s = parse(
            "\u{feff}{\"dll\":true,\"installed\":\"Mic (A)\",\"soundpad\":[],\"eligible\":[\"Mic (A)\",\"Mic (B)\"],\"ok\":true}",
        );
        assert!(s.ok && s.dll);
        assert_eq!(s.installed, ["Mic (A)"]);
        assert_eq!(s.eligible.len(), 2);
        assert_eq!(
            parse("{\"ok\":false,\"error\":\"not-admin\"}").error,
            "not-admin"
        );
        assert_eq!(parse("").error, "no-result");
    }

    #[test]
    fn sha256_matches_the_standard() {
        assert_eq!(
            sha256(b"abc"),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
    }

    #[test]
    fn powershell_comes_from_system32() {
        let p = powershell();
        assert!(p.is_absolute() && p.exists(), "{}", p.display());
    }

    #[test]
    fn the_script_runs_only_unchanged() {
        let staged = Staged::new(false).unwrap();
        let s = status_from(&staged);
        assert!(
            s.error != "tampered" && s.error != "no-result",
            "the embedded script ran: {s:?}"
        );
        fs::write(&staged.script, "'{\"ok\":true}'").unwrap(); // what another program could leave there
        assert_eq!(status_from(&staged).error, "tampered");
        let dir = staged.dir.clone();
        drop(staged);
        assert!(!dir.exists());
    }

    #[test]
    fn quotes_survive_an_apostrophe() {
        assert_eq!(
            quote(Path::new(r"C:\Users\O'Brien\x.ps1")),
            r"'C:\Users\O''Brien\x.ps1'"
        );
    }
}
