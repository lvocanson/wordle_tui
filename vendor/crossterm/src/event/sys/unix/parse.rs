// LOCAL PATCH — see …LOCAL_PATCH.md: this parser decodes only the events the game consumes —
// key presses (Esc, Enter, Backspace, Ctrl+letter, printable characters) and left-button-down
// mouse events. Everything else is framed and dropped.

use std::io;

use crate::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};

// Event parsing
//
// Every fn returns Result<Option<Event>>
//
// Ok(None) -> wait for more bytes
// Err(_) -> failed to parse event, clear the buffer
// Ok(Some(event)) -> we have event, clear the buffer

fn could_not_parse_event_error() -> io::Error {
    // LOCAL PATCH — see …LOCAL_PATCH.md: no message payload (a &str payload anchors
    // Box<dyn Error> + its Debug/Display vtables + char::escape_debug's unicode tables).
    io::Error::from(io::ErrorKind::Other)
}

pub(crate) fn parse_event(buffer: &[u8], input_available: bool) -> io::Result<Option<Event>> {
    if buffer.is_empty() {
        return Ok(None);
    }

    // LOCAL PATCH — see …LOCAL_PATCH.md: only the events the game consumes are produced —
    // Esc, Enter, Backspace, Ctrl+letter and printable characters. Everything else (SS3 and
    // Alt+key sequences, Tab, the other control codes) is dropped with `Err`, which the caller
    // turns into "clear the buffer and carry on", exactly as it already does for a sequence
    // upstream cannot parse. Whole sequences are always consumed before being dropped, so their
    // tail can never resurface as fake keystrokes.
    match buffer[0] {
        b'\x1B' => {
            if buffer.len() == 1 {
                if input_available {
                    // Possible Esc sequence
                    Ok(None)
                } else {
                    Ok(Some(Event::Key(KeyCode::Esc.into())))
                }
            } else {
                match buffer[1] {
                    b'[' => parse_csi(buffer),
                    b'\x1B' => Ok(Some(Event::Key(KeyCode::Esc.into()))),
                    // SS3 (ESC O x) is always three bytes; wait for the third, then drop it.
                    b'O' if buffer.len() == 2 => Ok(None),
                    _ => Err(could_not_parse_event_error()),
                }
            }
        }
        b'\r' => Ok(Some(Event::Key(KeyCode::Enter.into()))),
        b'\x7F' => Ok(Some(Event::Key(KeyCode::Backspace.into()))),
        // Ctrl+letter, the range that carries Ctrl+C. \n (Ctrl+J) and \t (Ctrl+I) land here too:
        // upstream maps them to Enter/Tab, but only outside raw mode for \n, and this app is
        // always in raw mode.
        c @ b'\x01'..=b'\x1A' => Ok(Some(Event::Key(KeyEvent::new(
            KeyCode::Char((c - 0x1 + b'a') as char),
            KeyModifiers::CONTROL,
        )))),
        _ => parse_utf8_char(buffer).map(|maybe_char| {
            maybe_char
                .map(KeyCode::Char)
                .map(char_code_to_event)
                .map(Event::Key)
        }),
    }
}

// converts KeyCode to KeyEvent (adds shift modifier in case of uppercase characters)
fn char_code_to_event(code: KeyCode) -> KeyEvent {
    // LOCAL PATCH — see …LOCAL_PATCH.md: ASCII-only uppercase test (`char::is_uppercase` anchors
    // the unicode case tables). A non-ASCII capital now arrives without the SHIFT modifier; the
    // character itself is unchanged, and this game reads neither.
    let modifiers = match code {
        KeyCode::Char(c) if c.is_ascii_uppercase() => KeyModifiers::SHIFT,
        _ => KeyModifiers::empty(),
    };
    KeyEvent::new(code, modifiers)
}

// LOCAL PATCH — see …LOCAL_PATCH.md: of the CSI sequences, only the two mouse encodings the
// terminal can send us are decoded. Every other one — arrows, F-keys, Home/End/PageUp/PageDown,
// Insert/Delete, BackTab, focus in/out, the kitty protocol, cursor-position and device-attribute
// replies, bracketed paste — is dropped.
//
// Dropping still has to *frame* the sequence: a CSI sequence runs until its first final byte
// (0x40..=0x7E), so incomplete ones keep returning `Ok(None)`. Erroring out early would clear the
// buffer mid-sequence and the tail would be re-parsed as ordinary keystrokes — an arrow key would
// type letters into the game.
fn parse_csi(buffer: &[u8]) -> io::Result<Option<Event>> {
    assert!(buffer.starts_with(b"\x1B[")); // ESC [

    if buffer.len() == 2 {
        return Ok(None);
    }

    match buffer[2] {
        b'M' => parse_csi_normal_mouse(buffer),
        b'<' => parse_csi_sgr_mouse(buffer),
        // `ESC [ [ x` (Linux console F1-F5): '[' is itself a final byte, so this one needs its
        // own arm or the rule below would drop it a byte early and leak the 'x'.
        b'[' if buffer.len() == 3 => Ok(None),
        _ => {
            if (0x40..=0x7E).contains(&buffer[buffer.len() - 1]) {
                Err(could_not_parse_event_error())
            } else {
                Ok(None)
            }
        }
    }
}

// LOCAL PATCH — see …LOCAL_PATCH.md: both mouse decoders below keep only a left-button press;
// releases, drags, motion, scrolling and the other two buttons are dropped, as are the mouse
// modifier bits (the game reads none of them).

/// A left-button press that is not a drag.
///
/// Bit layout of Cb, from low to high: button number, button number, shift, meta (alt), control,
/// dragging, button number, button number — so a plain left press leaves the two button-number
/// fields and the drag bit clear.
fn is_left_press(cb: u8) -> bool {
    cb & 0b1110_0011 == 0
}

fn left_press_at(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent { column, row })
}

/// One `;`-separated decimal parameter, read straight from the bytes: `str::parse` would pull in
/// the whole `FromStr` machinery for three small numbers.
fn decimal(param: Option<&[u8]>) -> io::Result<u16> {
    let bytes = param.ok_or_else(could_not_parse_event_error)?;
    if bytes.is_empty() {
        return Err(could_not_parse_event_error());
    }
    let mut n: u16 = 0;
    for &b in bytes {
        let digit = b.wrapping_sub(b'0');
        if digit > 9 {
            return Err(could_not_parse_event_error());
        }
        // Wrapping, like the coordinate arithmetic below: a bogus parameter yields a nonsense
        // position, which hit-testing rejects. It cannot panic.
        n = n.wrapping_mul(10).wrapping_add(digit as u16);
    }
    Ok(n)
}

fn parse_csi_normal_mouse(buffer: &[u8]) -> io::Result<Option<Event>> {
    // Normal mouse encoding: ESC [ M CB Cx Cy (6 characters only).

    assert!(buffer.starts_with(b"\x1B[M")); // ESC [ M

    if buffer.len() < 6 {
        return Ok(None);
    }

    if !is_left_press(buffer[3].wrapping_sub(32)) {
        return Err(could_not_parse_event_error());
    }

    // See http://www.xfree86.org/current/ctlseqs.html#Mouse%20Tracking
    // The upper left character position on the terminal is denoted as 1,1.
    // Subtract 1 to keep it synced with cursor
    let cx = u16::from(buffer[4].saturating_sub(32)) - 1;
    let cy = u16::from(buffer[5].saturating_sub(32)) - 1;

    Ok(Some(left_press_at(cx, cy)))
}

fn parse_csi_sgr_mouse(buffer: &[u8]) -> io::Result<Option<Event>> {
    // ESC [ < Cb ; Cx ; Cy (;) (M or m)

    assert!(buffer.starts_with(b"\x1B[<")); // ESC [ <

    // SGR ends with an uppercase M for a press and a lowercase m for a release; anything else
    // means the sequence is still incomplete.
    let last = match buffer.last() {
        Some(&b @ (b'm' | b'M')) => b,
        _ => return Ok(None),
    };

    let mut params = buffer[3..buffer.len() - 1].split(|&b| b == b';');
    let cb = decimal(params.next())?;
    let cx = decimal(params.next())?;
    let cy = decimal(params.next())?;

    if last != b'M' || cb > u16::from(u8::MAX) || !is_left_press(cb as u8) {
        return Err(could_not_parse_event_error());
    }

    // See http://www.xfree86.org/current/ctlseqs.html#Mouse%20Tracking
    // The upper left character position on the terminal is denoted as 1,1.
    // Subtract 1 to keep it synced with cursor
    Ok(Some(left_press_at(cx.wrapping_sub(1), cy.wrapping_sub(1))))
}

fn parse_utf8_char(buffer: &[u8]) -> io::Result<Option<char>> {
    match std::str::from_utf8(buffer) {
        Ok(s) => {
            let ch = s.chars().next().ok_or_else(could_not_parse_event_error)?;

            Ok(Some(ch))
        }
        Err(_) => {
            // from_utf8 failed, but we have to check if we need more bytes for code point
            // and if all the bytes we have no are valid

            let required_bytes = match buffer[0] {
                // https://en.wikipedia.org/wiki/UTF-8#Description
                (0x00..=0x7F) => 1, // 0xxxxxxx
                (0xC0..=0xDF) => 2, // 110xxxxx 10xxxxxx
                (0xE0..=0xEF) => 3, // 1110xxxx 10xxxxxx 10xxxxxx
                (0xF0..=0xF7) => 4, // 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx
                (0x80..=0xBF) | (0xF8..=0xFF) => return Err(could_not_parse_event_error()),
            };

            // More than 1 byte, check them for 10xxxxxx pattern
            if required_bytes > 1 && buffer.len() > 1 {
                for byte in &buffer[1..] {
                    if byte & !0b0011_1111 != 0b1000_0000 {
                        return Err(could_not_parse_event_error());
                    }
                }
            }

            if buffer.len() < required_bytes {
                // All bytes looks good so far, but we need more of them
                Ok(None)
            } else {
                Err(could_not_parse_event_error())
            }
        }
    }
}
