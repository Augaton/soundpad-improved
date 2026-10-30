// The icon in the notification area: the window can close while the hotkeys keep working.
// A hidden window of its own, on its own thread, receives the icon's clicks; a second
// instance of the app finds it by its class and asks it to bring the window back.
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, FindWindowW, GetCursorPos,
    GetMessageW, GetSystemMetrics, HICON, LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING, MSG,
    PostMessageW, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

pub enum Command {
    Show,
    StopAll,
    Quit,
}

const CLASS: PCWSTR = w!("SoundpadImprovedTray");
const WM_SHOW: u32 = WM_APP + 2; // from a second instance: bring the window back
const WM_TRAY: u32 = WM_APP + 1; // the icon's mouse events
const ID_OPEN: i32 = 1;
const ID_STOP: i32 = 2;
const ID_QUIT: i32 = 3;

static HANDLER: OnceLock<Box<dyn Fn(Command) + Send + Sync>> = OnceLock::new();
static ICON: OnceLock<isize> = OnceLock::new();
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0); // Explorer restarted: add the icon again

pub struct Tray {
    hwnd: isize, // HWND is not Send
}

/// Puts the icon up. `on_command` runs on the tray's thread.
pub fn start(on_command: impl Fn(Command) + Send + Sync + 'static) -> Option<Tray> {
    let _ = HANDLER.set(Box::new(on_command));
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("zone de notification".into())
        .spawn(move || unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassW(&class);
            let Ok(hwnd) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                w!(""),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            ) else {
                let _ = tx.send(None);
                return;
            };
            TASKBAR_CREATED.store(
                RegisterWindowMessageW(w!("TaskbarCreated")),
                Ordering::Relaxed,
            );
            add_icon(hwnd);
            let _ = tx.send(Some(hwnd.0 as isize));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .ok()?;
    rx.recv().ok().flatten().map(|hwnd| Tray { hwnd })
}

/// From a second instance: asks the one running to show its window. False when it has no
/// tray window to ask.
pub fn wake_running() -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(CLASS, PCWSTR::null()) else {
            return false;
        };
        let _ = AllowSetForegroundWindow(ASFW_ANY); // the window may come to the front
        PostMessageW(Some(hwnd), WM_SHOW, WPARAM(0), LPARAM(0)).is_ok()
    }
}

impl Tray {
    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(self.hwnd as _),
            uID: 1,
            ..Default::default()
        }
    }

    /// A Windows notification coming from the icon.
    pub fn notify(&self, title: &str, text: &str) {
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn remove(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
        }
    }
}

/// Into a fixed, zeroed buffer of the Windows API: cut short, ending with its 0.
fn copy_wide(dst: &mut [u16], text: &str) {
    let room = dst.len() - 1;
    for (d, c) in dst[..room].iter_mut().zip(text.encode_utf16()) {
        *d = c;
    }
}

fn send(command: Command) {
    if let Some(handler) = HANDLER.get() {
        handler(command);
    }
}

unsafe fn add_icon(hwnd: HWND) {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: HICON(*ICON.get_or_init(|| icon().map_or(0, |h| h.0 as isize)) as _),
        ..Default::default()
    };
    copy_wide(&mut data.szTip, "Soundpad Improved");
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
    }
}

/// The app's icon at the size of the notification area, as an icon resource: a 32-bit DIB
/// (bottom-up, height doubled for the mask), then an empty mask, the alpha doing the work.
fn icon() -> Option<HICON> {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 256) as u32;
    let rgba = super::theme::app_icon(size);
    let mut res = Vec::new();
    for v in [40, size, size * 2] {
        res.extend(v.to_le_bytes());
    }
    res.extend(1u16.to_le_bytes()); // planes
    res.extend(32u16.to_le_bytes()); // bits per pixel
    res.extend([0u8; 24]); // no compression, sizes and colors unused
    for row in rgba.chunks_exact(size as usize * 4).rev() {
        for [r, g, b, a] in row.as_chunks::<4>().0 {
            res.extend([*b, *g, *r, *a]);
        }
    }
    res.extend(vec![0u8; size.div_ceil(32) as usize * 4 * size as usize]);
    unsafe {
        CreateIconFromResourceEx(
            &res,
            true,
            0x0003_0000,
            size as i32,
            size as i32,
            LR_DEFAULTCOLOR,
        )
    }
    .ok()
}

unsafe fn menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN as usize, w!("Ouvrir"));
        let _ = AppendMenuW(menu, MF_STRING, ID_STOP as usize, w!("Tout arrêter"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT as usize, w!("Quitter"));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // Without it, the menu stays open when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match chosen.0 {
            ID_OPEN => send(Command::Show),
            ID_STOP => send(Command::StopAll),
            ID_QUIT => send(Command::Quit),
            _ => {}
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY => match lparam.0 as u32 & 0xffff {
            WM_LBUTTONUP => send(Command::Show),
            WM_RBUTTONUP | WM_CONTEXTMENU => unsafe { menu(hwnd) },
            _ => {}
        },
        WM_SHOW => send(Command::Show),
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => unsafe { add_icon(hwnd) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}
