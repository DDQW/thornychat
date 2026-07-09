//! Driving the composer input's native Cut/Copy from the right-click menu.
//!
//! iced's `text_input` owns its cursor and text selection internally and
//! exposes no way for app code to read them, nor any operation to copy/cut the
//! selection. The only code paths that respect the *live* selection are the
//! widget's own Ctrl+C / Ctrl+X keyboard handlers. So the menu "presses the
//! keys" for the user: it injects the real Ctrl chord at the OS level with
//! `SendInput`, exactly as a keypress would arrive, and the focused input then
//! copies/cuts its actual selection through its normal handler (which, for
//! Cut, publishes the edit back to us as an ordinary `BodyChanged`).
//!
//! Only Cut and Copy come through here. Paste is handled app-side (append the
//! clipboard text / stage its files) so it still works when the input isn't
//! focused, and Select All uses iced's native `text_input::select_all`
//! operation. Windows-only, like the rest of the app's platform glue.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_C, VK_CONTROL, VK_X,
};

/// Which native clipboard edit to drive on the focused composer input.
#[derive(Debug, Clone, Copy)]
pub enum Edit {
    Copy,
    Cut,
}

/// Synthesizes Ctrl+C / Ctrl+X so the focused `text_input` runs its own
/// copy/cut over its current selection. A no-op in practice when nothing is
/// focused or selected: an unfocused input ignores keyboard events outright,
/// and copy/cut with an empty selection writes nothing.
pub fn edit(edit: Edit) {
    let letter = match edit {
        Edit::Copy => VK_C,
        Edit::Cut => VK_X,
    };
    // Ctrl down, letter down, letter up, Ctrl up — one SendInput batch so the
    // modifier is reliably held across the letter (and released after).
    let inputs = [
        key_event(VK_CONTROL, false),
        key_event(letter, false),
        key_event(letter, true),
        key_event(VK_CONTROL, true),
    ];
    // SAFETY: `inputs` is a stack-owned slice of fully-initialized INPUT
    // records; SendInput only reads them, and the reported size is this
    // build's `INPUT` layout.
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn key_event(vk: VIRTUAL_KEY, release: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if release { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
