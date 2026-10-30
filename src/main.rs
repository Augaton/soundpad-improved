// Bruitage: a polyphonic soundboard for Windows, formerly Soundpad Improved.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod autostart;
mod filter;
mod hotkeys;
mod keys;
mod link;
mod model;
mod soundpad;
mod state;
mod store;
mod ui;

use eframe::egui;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, MB_ICONINFORMATION, MB_OK, MessageBoxW, SW_RESTORE, SW_SHOWNORMAL,
    SetForegroundWindow, ShowWindow,
};
use windows::core::{PCWSTR, w};

const TITLE: &str = "Bruitage";

pub fn open_url(url: &str) {
    let url: Vec<u16> = url.encode_utf16().chain([0]).collect();
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
}

/// A second instance would fight the first one over the hotkeys: it brings the first one up,
/// through its notification-area icon (the window may be hidden there).
/// A test profile (SPI_USER_DATA) runs beside the everyday one.
fn already_running() -> bool {
    let mut name = "Local\\BruitageInstance".to_string();
    if let Some(dir) = std::env::var_os("SPI_USER_DATA") {
        use std::hash::{BuildHasher, BuildHasherDefault};
        let hash = BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default()
            .hash_one(dir);
        name += &format!("-{hash:x}");
    }
    let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mutex = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) };
    if mutex.is_err() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        std::mem::forget(mutex); // held until the process exits
        return false;
    }
    if ui::wake_running() {
        return true;
    }
    if let Ok(hwnd) = unsafe { FindWindowW(None, w!("Bruitage")) } {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
    true
}

/// The app under its former name, Soundpad Improved, still running (in the notification area
/// since 0.3): it would hold the hotkeys and keep writing its library, which moves now.
fn former_running() -> bool {
    if std::env::var_os("SPI_USER_DATA").is_some() {
        return false;
    }
    let Ok(mutex) = (unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE,
            false,
            w!("Local\\SoundpadImprovedInstance"),
        )
    }) else {
        return false;
    };
    let _ = unsafe { CloseHandle(mutex) };
    unsafe {
        MessageBoxW(
            None,
            w!(
                "Soundpad Improved, l’ancien nom de Bruitage, est encore ouvert, peut-être dans la zone de notification. Quitte-le (clic droit sur son icône, « Quitter »), puis relance Bruitage : il reprendra ta bibliothèque."
            ),
            w!("Bruitage"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
    true
}

fn main() -> eframe::Result {
    if former_running() || already_running() {
        return Ok(());
    }
    let hidden = std::env::args().any(|a| a == autostart::HIDDEN);
    let core = state::Core::new();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_icon(ui::window_icon())
            .with_inner_size([1320.0, 840.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, core, hidden)))),
    )
}
