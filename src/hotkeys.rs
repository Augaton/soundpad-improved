// Global hotkeys through Win32 RegisterHotKey, the way Soundpad does it: they work in games,
// and so do keys other APIs refuse (Pause among them). A thread with its own message queue
// owns the registrations; each key press calls `on_hit` with that hotkey's value.
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY,
};

const WM_SET: u32 = WM_APP + 1;

type Request<T> = (Vec<(u32, u32, T)>, Sender<Vec<bool>>);

pub struct Hotkeys<T> {
    thread_id: u32,
    requests: Sender<Request<T>>,
}

impl<T: Clone + Send + 'static> Hotkeys<T> {
    pub fn start(on_hit: impl Fn(T) + Send + 'static) -> Hotkeys<T> {
        let (requests, rx) = mpsc::channel::<Request<T>>();
        let (id_tx, id_rx) = mpsc::channel();
        thread::Builder::new()
            .name("raccourcis".into())
            .spawn(move || {
                let mut msg = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE); // creates the queue
                    let _ = id_tx.send(GetCurrentThreadId());
                }
                let mut actions: Vec<T> = Vec::new(); // registration id - 1 -> value
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                    if msg.message == WM_HOTKEY {
                        if let Some(a) = actions.get(msg.wParam.0.wrapping_sub(1)) {
                            on_hit(a.clone());
                        }
                    } else if msg.message == WM_SET {
                        while let Ok((list, reply)) = rx.try_recv() {
                            for id in 1..=actions.len() {
                                let _ = unsafe { UnregisterHotKey(None, id as i32) };
                            }
                            let ok = list
                                .iter()
                                .enumerate()
                                .map(|(i, (vk, mods, _))| {
                                    unsafe {
                                        RegisterHotKey(
                                            None,
                                            i as i32 + 1,
                                            HOT_KEY_MODIFIERS(*mods) | MOD_NOREPEAT,
                                            *vk,
                                        )
                                    }
                                    .is_ok()
                                })
                                .collect();
                            actions = list.into_iter().map(|(_, _, a)| a).collect();
                            let _ = reply.send(ok);
                        }
                    }
                }
            })
            .expect("thread raccourcis");
        let thread_id = id_rx.recv().unwrap_or(0);
        Hotkeys {
            thread_id,
            requests,
        }
    }

    /// Replaces every registration with `list` of (virtual key, modifiers, value).
    /// Returns, for each one, whether Windows accepted it (false: another app holds the key).
    pub fn set(&self, list: Vec<(u32, u32, T)>) -> Vec<bool> {
        let n = list.len();
        let (reply, answer) = mpsc::channel();
        if self.requests.send((list, reply)).is_err()
            || unsafe { PostThreadMessageW(self.thread_id, WM_SET, WPARAM(0), LPARAM(0)) }.is_err()
        {
            return vec![false; n];
        }
        answer
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| vec![false; n])
    }
}
