use crossterm_winapi::{ControlKeyState, EventFlags, KeyEventRecord};
use winapi::um::{
    wincon::{
        LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED, SHIFT_PRESSED,
    },
    winuser::{VK_BACK, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN, VK_SHIFT, VK_TAB},
};

use crate::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};

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
    parse_mouse_event_record(&mouse_event, buttons_pressed).map(Event::Mouse)
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

fn parse_key_event_record(key_event: &KeyEventRecord) -> Option<KeyEvent> {
    // LOCAL PATCH — see …LOCAL_PATCH.md: presses only. Upstream keeps releases for the Alt-code
    // exception (an Alt release carrying a u_char); with Alt-code input dropped, releases carry
    // nothing this app reads.
    if !key_event.key_down {
        return None;
    }
    let modifiers = KeyModifiers::from(&key_event.control_key_state);

    let parse_result = match key_event.virtual_key_code as i32 {
        VK_SHIFT | VK_CONTROL | VK_MENU => None,
        VK_BACK => Some(KeyCode::Backspace),
        VK_ESCAPE => Some(KeyCode::Esc),
        VK_RETURN => Some(KeyCode::Enter),
        // Function/navigation keys fall through to the u_char match below, which resolves them
        // to nothing, so their dedicated KeyCode arms are gone.
        _ => {
            let utf16 = key_event.u_char;
            match utf16 {
                // Ctrl+<letter> reaches us as the control code 0x01..=0x1a whatever the active
                // layout, so the letter is recoverable by arithmetic. Upstream instead asks the
                // layout through ToUnicodeEx (plus GetForegroundWindow/GetKeyboardLayout, a
                // 256-byte key-state buffer and a UTF-16 decode) to also resolve dead keys and
                // non-Latin control combinations, neither of which this app reads.
                // VK_TAB is the one key in this range that is not a Ctrl combination.
                0x01..=0x1a if key_event.virtual_key_code as i32 != VK_TAB => {
                    Some(KeyCode::Char((b'a' + utf16 as u8 - 1) as char))
                }
                // Function and navigation keys arrive here with u_char == 0, dead keys likewise:
                // nothing this app reads.
                0x00..=0x1f => None,
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

fn parse_mouse_event_record(
    event: &crossterm_winapi::MouseEvent,
    buttons_pressed: &MouseButtonsPressed,
) -> Option<MouseEvent> {
    // LOCAL PATCH — see …LOCAL_PATCH.md: only a fresh left-button press becomes an event; the
    // upstream arms for releases, right/middle buttons, motion/drag and both scroll axes are
    // gone, and so are the modifier bits the game never reads.
    match event.event_flags {
        EventFlags::PressOrRelease | EventFlags::DoubleClick => {}
        _ => return None,
    }

    if !event.button_state.left_button() || buttons_pressed.left {
        return None;
    }

    // LOCAL PATCH — see …LOCAL_PATCH.md: the record's y is absolute in the screen buffer, which
    // upstream corrects by reading the window rect on every mouse event. The alternate screen
    // buffer is exactly window-sized, so its top is always 0 and the correction is the identity.
    Some(MouseEvent {
        column: event.mouse_position.x as u16,
        row: event.mouse_position.y as u16,
    })
}
