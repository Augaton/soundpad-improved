// Soundpad Improved: a polyphonic soundboard for Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod filter;
mod hotkeys;
mod keys;
mod model;
mod soundpad;
mod state;
mod store;
mod ui;

use eframe::egui;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
};
use windows::core::{PCWSTR, w};

const TITLE: &str = "Soundpad Improved";

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

/// A second instance would fight the first one over the hotkeys: it brings the first one up.
fn already_running() -> bool {
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\SoundpadImprovedInstance")) };
    if mutex.is_err() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        std::mem::forget(mutex); // held until the process exits
        return false;
    }
    if let Ok(hwnd) = unsafe { FindWindowW(None, w!("Soundpad Improved")) } {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
    true
}

fn main() -> eframe::Result {
    if already_running() {
        return Ok(());
    }
    let core = state::Core::new();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size([1320.0, 840.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, core)))),
    )
}
