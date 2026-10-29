// Typing a hotkey. Windows' key state is read directly, so every key RegisterHotKey knows
// (Pause, the numeric keypad, F13 to F24...) comes with the virtual-key code it will see,
// AZERTY letters included.
use eframe::egui::{self, Context};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

use crate::keys::{self, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};
use crate::model::GlobalAction;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Sound, // the sound being edited
    Global(GlobalAction),
}

pub struct Capture {
    pub target: Target,
    held: [bool; 256], // keys down at the previous look: a key counts when it goes down
}

fn down(vk: u32) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16) & 0x8000 != 0
}

/// Mouse buttons, modifiers on their own, IME keys.
fn ignored(vk: u32) -> bool {
    matches!(vk, 1..=6 | 16..=18 | 21 | 25 | 91 | 92 | 160..=165 | 229)
}

impl Capture {
    pub fn start(target: Target) -> Capture {
        let mut held = [false; 256];
        for (vk, h) in held.iter_mut().enumerate().skip(1) {
            *h = down(vk as u32);
        }
        Capture { target, held }
    }

    /// Some(Some(hotkey)) once typed, Some(None) when cancelled (Esc or a click), None meanwhile.
    pub fn poll(&mut self, ctx: &Context) -> Option<Option<String>> {
        // The keys belong to the capture, not to the window's widgets.
        ctx.input_mut(|i| {
            i.events
                .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)))
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(15));
        if ctx.input(|i| i.pointer.any_pressed()) {
            return Some(None);
        }
        for vk in 1..255u32 {
            let is_down = down(vk);
            let was = std::mem::replace(&mut self.held[vk as usize], is_down);
            if !is_down || was || ignored(vk) {
                continue;
            }
            if vk == 27 {
                return Some(None);
            }
            let mut mods = 0;
            for (key, bit) in [
                (17, MOD_CTRL),
                (18, MOD_ALT),
                (16, MOD_SHIFT),
                (91, MOD_WIN),
                (92, MOD_WIN),
            ] {
                if down(key) {
                    mods |= bit;
                }
            }
            if let Some(accel) = keys::vk_to_accelerator(vk, mods) {
                return Some(Some(accel));
            }
        }
        None
    }
}
