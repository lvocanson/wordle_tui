use crossterm_winapi::{ControlKeyState, EventFlags, KeyEventRecord, ScreenBuffer};
use winapi::um::{
    wincon::{
        CAPSLOCK_ON, LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED,
        SHIFT_PRESSED,
    },
    winuser::{
        GetForegroundWindow, GetKeyboardLayout, GetWindowThreadProcessId, ToUnicodeEx, VK_BACK,
        VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN, VK_SHIFT,
    },
};

use crate::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

// LOCAL PATCH — see …LOCAL_PATCH.md: the event set is reduced to what this app consumes — key
// *presses* (Esc, Enter, Backspace, layout-resolved characters with modifiers), *left-button-down*
// mouse events, and resizes. Key releases, alt-codes, surrogate pairs, function/navigation keys,
// and every other mouse kind (up/drag/move/scroll, right/middle) are not parsed into events.

#[derive(Default)]
pub struct MouseButtonsPressed {
    pub(crate) left: bool,
}

pub(crate) fn handle_mouse_event(
    mouse_event: crossterm_winapi::MouseEvent,
    buttons_pressed: &MouseButtonsPressed,
) -> Option<Event> {
    if let Ok(Some(event)) = parse_mouse_event_record(&mouse_event, buttons_pressed) {
        return Some(Event::Mouse(event));
    }

    None
}

pub(crate) fn handle_key_event(key_event: KeyEventRecord) -> Option<Event> {
    parse_key_event_record(&key_event).map(Event::Key)
}

impl From<&ControlKeyState> for KeyModifiers {
    fn from(state: &ControlKeyState) -> Self {
        let shift = state.has_state(SHIFT_PRESSED);
        let alt = state.has_state(LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED);
        let control = state.has_state(LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED);

        let mut modifier = KeyModifiers::empty();

        if shift {
            modifier |= KeyModifiers::SHIFT;
        }
        if control {
            modifier |= KeyModifiers::CONTROL;
        }
        if alt {
            modifier |= KeyModifiers::ALT;
        }

        modifier
    }
}

enum CharCase {
    LowerCase,
    UpperCase,
}

// LOCAL PATCH — see …LOCAL_PATCH.md: ASCII-only case correction. Upstream uses the full unicode
// `to_lowercase`/`to_uppercase` iterators, anchoring the case-conversion tables; non-ASCII chars
// now pass through in the case the keyboard layout produced.
fn try_ensure_char_case(ch: char, desired_case: CharCase) -> char {
    match desired_case {
        CharCase::LowerCase => ch.to_ascii_lowercase(),
        CharCase::UpperCase => ch.to_ascii_uppercase(),
    }
}

// Attempts to return the character for a key event accounting for the user's keyboard layout.
// The returned character (if any) is capitalized (if applicable) based on shift and capslock state.
// Returns None if the key doesn't map to a character or if it is a dead key.
// We use the *currently* active keyboard layout (if it can be determined). This layout may not
// correspond to the keyboard layout that was active when the user typed their input, since console
// applications get their input asynchronously from the terminal. By the time a console application
// can process a key input, the user may have changed the active layout. In this case, the character
// returned might not correspond to what the user expects, but there is no way for a console
// application to know what the keyboard layout actually was for a key event, so this is our best
// effort. If a console application processes input in a timely fashion, then it is unlikely that a
// user has time to change their keyboard layout before a key event is processed.
fn get_char_for_key(key_event: &KeyEventRecord) -> Option<char> {
    let virtual_key_code = key_event.virtual_key_code as u32;
    let virtual_scan_code = key_event.virtual_scan_code as u32;
    let key_state = [0u8; 256];
    let mut utf16_buf = [0u16, 16];
    let dont_change_kernel_keyboard_state = 0x4;

    // Best-effort attempt at determining the currently active keyboard layout.
    // At the time of writing, this works for a console application running in Windows Terminal, but
    // doesn't work under a Conhost terminal. For Conhost, the window handle returned by
    // GetForegroundWindow() does not appear to actually be the foreground window which has the
    // keyboard layout associated with it (or perhaps it is, but also has special protection that
    // doesn't allow us to query it).
    // When this determination fails, the returned keyboard layout handle will be null, which is an
    // acceptable input for ToUnicodeEx, as that argument is optional. In this case ToUnicodeEx
    // appears to use the keyboard layout associated with the current thread, which will be the
    // layout that was inherited when the console application started (or possibly when the current
    // thread was spawned). This is then unfortunately not updated when the user changes their
    // keyboard layout in the terminal, but it's what we get.
    let active_keyboard_layout = unsafe {
        let foreground_window = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground_window, std::ptr::null_mut());
        GetKeyboardLayout(foreground_thread)
    };

    let ret = unsafe {
        ToUnicodeEx(
            virtual_key_code,
            virtual_scan_code,
            key_state.as_ptr(),
            utf16_buf.as_mut_ptr(),
            utf16_buf.len() as i32,
            dont_change_kernel_keyboard_state,
            active_keyboard_layout,
        )
    };

    // -1 indicates a dead key.
    // 0 indicates no character for this key.
    if ret < 1 {
        return None;
    }

    let mut ch_iter = std::char::decode_utf16(utf16_buf.into_iter().take(ret as usize));
    let mut ch = ch_iter.next()?.ok()?;
    if ch_iter.next().is_some() {
        // Key doesn't map to a single char.
        return None;
    }

    let is_shift_pressed = key_event.control_key_state.has_state(SHIFT_PRESSED);
    let is_capslock_on = key_event.control_key_state.has_state(CAPSLOCK_ON);
    let desired_case = if is_shift_pressed ^ is_capslock_on {
        CharCase::UpperCase
    } else {
        CharCase::LowerCase
    };
    ch = try_ensure_char_case(ch, desired_case);
    Some(ch)
}

fn parse_key_event_record(key_event: &KeyEventRecord) -> Option<KeyEvent> {
    // LOCAL PATCH — see …LOCAL_PATCH.md: presses only. Upstream keeps releases for the Alt-code
    // exception (an Alt release carrying a u_char); with Alt-code input dropped, releases carry
    // nothing this app reads, and `KeyEvent::new` defaults the kind to Press.
    if !key_event.key_down {
        return None;
    }
    let modifiers = KeyModifiers::from(&key_event.control_key_state);

    let parse_result = match key_event.virtual_key_code as i32 {
        VK_SHIFT | VK_CONTROL | VK_MENU => None,
        VK_BACK => Some(KeyCode::Backspace),
        VK_ESCAPE => Some(KeyCode::Esc),
        VK_RETURN => Some(KeyCode::Enter),
        // Function/navigation keys fall through here with u_char == 0 (or a control code for
        // Tab): `get_char_for_key` then resolves to nothing (or a control char the game
        // ignores), so their dedicated KeyCode arms are gone.
        _ => {
            let utf16 = key_event.u_char;
            match utf16 {
                0x00..=0x1f => {
                    // Some key combinations generate either no u_char value or generate control
                    // codes. To deliver back a KeyCode::Char(...) event we want to know which
                    // character the key normally maps to on the user's keyboard layout.
                    // The keys that intentionally generate control codes (ESC, ENTER, etc.)
                    // are handled by their virtual key codes above.
                    get_char_for_key(key_event).map(KeyCode::Char)
                }
                // Surrogate halves land in the `None` of `from_u32` and are dropped (upstream
                // pairs them up across events to deliver astral-plane chars).
                unicode_scalar_value => {
                    std::char::from_u32(unicode_scalar_value as u32).map(KeyCode::Char)
                }
            }
        }
    };

    parse_result.map(|key_code| KeyEvent::new(key_code, modifiers))
}

// The 'y' position of a mouse event or resize event is not relative to the window but absolute to screen buffer.
// This means that when the mouse cursor is at the top left it will be x: 0, y: 2295 (e.g. y = number of cells conting from the absolute buffer height) instead of relative x: 0, y: 0 to the window.
pub fn parse_relative_y(y: i16) -> std::io::Result<i16> {
    let window_size = ScreenBuffer::current()?.info()?.terminal_window();
    Ok(y - window_size.top)
}

fn parse_mouse_event_record(
    event: &crossterm_winapi::MouseEvent,
    buttons_pressed: &MouseButtonsPressed,
) -> std::io::Result<Option<MouseEvent>> {
    let modifiers = KeyModifiers::from(&event.control_key_state);

    let xpos = event.mouse_position.x as u16;
    let ypos = parse_relative_y(event.mouse_position.y)? as u16;

    let button_state = event.button_state;

    // LOCAL PATCH — see …LOCAL_PATCH.md: only a fresh left-button press becomes an event; the
    // upstream arms for releases, right/middle buttons, motion/drag and both scroll axes are gone.
    let kind = match event.event_flags {
        EventFlags::PressOrRelease | EventFlags::DoubleClick => {
            if button_state.left_button() && !buttons_pressed.left {
                Some(MouseEventKind::Down(MouseButton::Left))
            } else {
                None
            }
        }
        _ => None,
    };

    Ok(kind.map(|kind| MouseEvent {
        kind,
        column: xpos,
        row: ypos,
        modifiers,
    }))
}
