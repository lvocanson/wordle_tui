use crossterm_winapi::{Console, Handle, InputRecord};

use crate::event::{
    sys::windows::parse::{handle_key_event, handle_mouse_event, MouseButtonsPressed},
    Event,
};

pub(crate) struct WindowsEventSource {
    console: Console,
    mouse_buttons_pressed: MouseButtonsPressed,
}

impl WindowsEventSource {
    pub(crate) fn new() -> std::io::Result<WindowsEventSource> {
        let console = Console::from(Handle::current_in_handle()?);
        Ok(WindowsEventSource {
            console,
            mouse_buttons_pressed: MouseButtonsPressed::default(),
        })
    }

    /// Blocks until the console produces an event this app reads.
    ///
    /// LOCAL PATCH — see …LOCAL_PATCH.md: `ReadConsoleInputW` already parks the thread until a
    /// record is available, so a caller that never wants a deadline needs nothing else. Upstream
    /// wraps that read in `WaitForMultipleObjects` + `GetNumberOfConsoleInputEvents` to be able to
    /// give up early, which is the one thing this app never asks for. Records the app does not
    /// read (key releases, other mouse kinds) simply cost another blocking read.
    pub(crate) fn read_blocking(&mut self) -> std::io::Result<Event> {
        loop {
            if let Some(event) = self.to_event(self.console.read_single_input_event()?) {
                return Ok(event);
            }
        }
    }

    fn to_event(&mut self, record: InputRecord) -> Option<Event> {
        // LOCAL PATCH — see …LOCAL_PATCH.md: no surrogate pairing, left-button-only
        // mouse tracking, and no FocusGained/FocusLost events (the app ignores them).
        match record {
            InputRecord::KeyEvent(record) => handle_key_event(record),
            InputRecord::MouseEvent(record) => {
                let mouse_event = handle_mouse_event(record, &self.mouse_buttons_pressed);
                self.mouse_buttons_pressed = MouseButtonsPressed {
                    left: record.button_state.left_button(),
                };

                mouse_event
            }
            InputRecord::WindowBufferSizeEvent(record) => {
                // windows starts counting at 0, unix at 1, add one to replicate unix behaviour.
                Some(Event::Resize(
                    (record.size.x as i32 + 1) as u16,
                    (record.size.y as i32 + 1) as u16,
                ))
            }
            _ => None,
        }
    }
}
