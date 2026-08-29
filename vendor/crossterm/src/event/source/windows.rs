use std::time::Duration;

use crossterm_winapi::{Console, Handle, InputRecord};

use crate::event::{
    sys::windows::{parse::MouseButtonsPressed, poll::WinApiPoll},
    Event,
};

#[cfg(feature = "event-stream")]
use crate::event::sys::Waker;
use crate::event::{
    source::EventSource,
    sys::windows::parse::{handle_key_event, handle_mouse_event},
    timeout::PollTimeout,
    InternalEvent,
};

pub(crate) struct WindowsEventSource {
    console: Console,
    poll: WinApiPoll,
    mouse_buttons_pressed: MouseButtonsPressed,
}

impl WindowsEventSource {
    pub(crate) fn new() -> std::io::Result<WindowsEventSource> {
        let console = Console::from(Handle::current_in_handle()?);
        Ok(WindowsEventSource {
            console,

            #[cfg(not(feature = "event-stream"))]
            poll: WinApiPoll::new(),
            #[cfg(feature = "event-stream")]
            poll: WinApiPoll::new()?,

            mouse_buttons_pressed: MouseButtonsPressed::default(),
        })
    }
}

impl WindowsEventSource {
    /// Blocks until the console produces an event this app reads.
    ///
    /// LOCAL PATCH — see …LOCAL_PATCH.md: `ReadConsoleInputW` already parks the thread until a
    /// record is available, so a caller that never wants a deadline needs nothing else. The
    /// `try_read` below only wraps that read in `WaitForMultipleObjects` +
    /// `GetNumberOfConsoleInputEvents` to be able to give up early, which is the one thing this
    /// app never asks for.
    pub(crate) fn read_blocking(&mut self) -> std::io::Result<Option<Event>> {
        Ok(self.to_event(self.console.read_single_input_event()?))
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

impl EventSource for WindowsEventSource {
    fn try_read(&mut self, timeout: Option<Duration>) -> std::io::Result<Option<InternalEvent>> {
        let poll_timeout = PollTimeout::new(timeout);

        loop {
            if let Some(event_ready) = self.poll.poll(poll_timeout.leftover())? {
                let number = self.console.number_of_console_input_events()?;
                if event_ready && number != 0 {
                    if let Some(event) = self.to_event(self.console.read_single_input_event()?) {
                        return Ok(Some(InternalEvent::Event(event)));
                    }
                }
            }

            if poll_timeout.elapsed() {
                return Ok(None);
            }
        }
    }

    #[cfg(feature = "event-stream")]
    fn waker(&self) -> Waker {
        self.poll.waker()
    }
}
