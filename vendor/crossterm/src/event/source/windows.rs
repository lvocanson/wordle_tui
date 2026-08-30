use crate::event::{
    sys::windows::{
        console_input, read_input_record,
        parse::{handle_key_event, handle_mouse_event, left_button, MouseButtonsPressed},
        Handle, InputRecord, KEY_EVENT, MOUSE_EVENT, WINDOW_BUFFER_SIZE_EVENT,
    },
    Event,
};

pub(crate) struct WindowsEventSource {
    /// The console input buffer. Never closed: the source lives in a `static` for as long as the
    /// process does, and the handle goes back to the system with it.
    input: Handle,
    mouse_buttons_pressed: MouseButtonsPressed,
}

impl WindowsEventSource {
    pub(crate) fn new() -> std::io::Result<WindowsEventSource> {
        Ok(WindowsEventSource {
            input: console_input()?,
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
            let record = read_input_record(self.input)?;
            if let Some(event) = self.to_event(&record) {
                return Ok(event);
            }
        }
    }

    fn to_event(&mut self, record: &InputRecord) -> Option<Event> {
        // LOCAL PATCH — see …LOCAL_PATCH.md: no surrogate pairing, left-button-only
        // mouse tracking, and no FocusGained/FocusLost events (the app ignores them).
        //
        // SAFETY: each arm reads the union member that `event_type` declares live.
        match record.event_type {
            KEY_EVENT => handle_key_event(unsafe { &record.event.key }),
            MOUSE_EVENT => {
                let record = unsafe { &record.event.mouse };
                let mouse_event = handle_mouse_event(record, &self.mouse_buttons_pressed);
                self.mouse_buttons_pressed = MouseButtonsPressed {
                    left: left_button(record.button_state),
                };

                mouse_event
            }
            WINDOW_BUFFER_SIZE_EVENT => {
                // LOCAL PATCH — see …LOCAL_PATCH.md: no +1. `dwSize` is already a count of
                // columns and rows, the same thing `terminal::size()` reports on both platforms;
                // upstream adds one, which makes the grid a column and a row too large for the
                // window it was told about.
                let size = unsafe { record.event.window.size };
                Some(Event::Resize(size.x as u16, size.y as u16))
            }
            _ => None,
        }
    }
}
