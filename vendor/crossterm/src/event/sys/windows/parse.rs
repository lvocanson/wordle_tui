use super::{KeyEventRecord, MouseEventRecord};
use crate::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};

// LOCAL PATCH — see …LOCAL_PATCH.md: the event set is reduced to what this app consumes — key
// *presses* (Esc, Enter, Backspace, layout-resolved characters with modifiers), *left-button-down*
// mouse events, and resizes. Key releases, alt-codes, surrogate pairs, function/navigation keys,
// and every other mouse kind (up/drag/move/scroll, right/middle) are not parsed into events.

// Virtual-key codes (winuser.h) and control-key state bits (wincon.h), declared here rather than
// imported from `winapi`: they are the seven keys and five bits this parser reads.
const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1B;

const RIGHT_ALT_PRESSED: u32 = 0x0001;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const SHIFT_PRESSED: u32 = 0x0010;

/// `MOUSE_EVENT_RECORD::dwButtonState`, leftmost button.
const FROM_LEFT_1ST_BUTTON_PRESSED: u32 = 0x0001;

/// `MOUSE_EVENT_RECORD::dwEventFlags`. A plain press or release reports none of them; a
/// double-click reports `DOUBLE_CLICK`. Motion and the two wheel axes are the flags this app
/// refuses.
const MOUSE_PRESS_OR_RELEASE: u32 = 0x0000;
const DOUBLE_CLICK: u32 = 0x0002;

#[derive(Default)]
pub struct MouseButtonsPressed {
    pub(crate) left: bool,
}

pub(crate) fn left_button(button_state: u32) -> bool {
    button_state & FROM_LEFT_1ST_BUTTON_PRESSED != 0
}

pub(crate) fn handle_mouse_event(
    mouse_event: &MouseEventRecord,
    buttons_pressed: &MouseButtonsPressed,
) -> Option<Event> {
    parse_mouse_event_record(mouse_event, buttons_pressed).map(Event::Mouse)
}

pub(crate) fn handle_key_event(key_event: &KeyEventRecord) -> Option<Event> {
    parse_key_event_record(key_event).map(Event::Key)
}

fn modifiers(control_key_state: u32) -> KeyModifiers {
    let mut modifiers = KeyModifiers::empty();

    if control_key_state & SHIFT_PRESSED != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if control_key_state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    if control_key_state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
        modifiers |= KeyModifiers::ALT;
    }

    modifiers
}

fn parse_key_event_record(key_event: &KeyEventRecord) -> Option<KeyEvent> {
    // LOCAL PATCH — see …LOCAL_PATCH.md: presses only. Upstream keeps releases for the Alt-code
    // exception (an Alt release carrying a u_char); with Alt-code input dropped, releases carry
    // nothing this app reads.
    if key_event.key_down == 0 {
        return None;
    }
    let modifiers = modifiers(key_event.control_key_state);

    let parse_result = match key_event.virtual_key_code {
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
                0x01..=0x1a if key_event.virtual_key_code != VK_TAB => {
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
    event: &MouseEventRecord,
    buttons_pressed: &MouseButtonsPressed,
) -> Option<MouseEvent> {
    // LOCAL PATCH — see …LOCAL_PATCH.md: only a fresh left-button press becomes an event; the
    // upstream arms for releases, right/middle buttons, motion/drag and both scroll axes are
    // gone, and so are the modifier bits the game never reads.
    if event.event_flags != MOUSE_PRESS_OR_RELEASE && event.event_flags != DOUBLE_CLICK {
        return None;
    }

    if !left_button(event.button_state) || buttons_pressed.left {
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
