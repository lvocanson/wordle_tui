use std::io::{self, Stdout, Write};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal;
#[cfg(unix)]
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

mod app;
mod codec;
mod game;
mod ui;
mod words;

use app::App;
use game::Phase;

// Enable ANSI escape processing on the console, and report whether the escapes we emit will be
// honoured at all. crossterm normally does this as a side effect of its `supports_ansi()` probe (a
// `parking_lot::Once` + a `TERM` env read); we call the three console entry points directly, which
// also keeps crossterm_winapi's Handle/ConsoleMode wrappers (an Arc and two Drop flavours) out of
// the binary. The handle comes from GetStdHandle rather than CONOUT$ on purpose: a stdout
// redirected to a file or a pipe has no screen to draw on, and fails the probe here. Windows-only
// — other platforms interpret escapes natively.
#[cfg(windows)]
fn enable_vt() -> bool {
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> isize;
        fn GetConsoleMode(handle: isize, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: isize, mode: u32) -> i32;
    }
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        GetConsoleMode(handle, &mut mode) != 0
            && SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

fn init_terminal() -> io::Result<Stdout> {
    // Probed first so there is nothing to undo when it fails.
    #[cfg(windows)]
    if !enable_vt() {
        return Err(io::ErrorKind::Unsupported.into());
    }
    // Windows has no `enable_raw_mode()` here on purpose: on that platform raw mode is a set of
    // bits in the console *input* mode, and `EnableMouseCapture` below assigns that mode whole
    // (mouse + window + extended flags, everything else cleared) rather than OR-ing into it. The
    // raw-mode call three lines earlier would be overwritten by it — the pair costs two handle
    // opens and two mode round-trips for an effect that never survives. Unix raw mode is termios,
    // untouched by the mouse sequences, so it stays.
    #[cfg(unix)]
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Enter the alternate screen (?1049h), set the window title (OSC 0), and hide the cursor
    // (?25l) as raw ANSI rather than through crossterm's commands: those route via `supports_ansi`
    // (a `Once` + env read on Windows) to choose ANSI-vs-WinAPI, pure overhead once we commit to
    // ANSI. No SetSize: the layout adapts to whatever size the terminal is (see ui::build_grid),
    // and forcing a resize corrupts the alternate screen's cursor restore. ?1049 keeps it clean.
    stdout.write_all(b"\x1b[?1049h\x1b]0;Wordle\x07\x1b[?25l")?;
    // Mouse capture stays on crossterm: on Windows it must set ENABLE_MOUSE_INPUT via WinAPI
    // (the console event source reads mouse from the input buffer, not from ANSI reports), which
    // the ANSI `?1000h`… sequences would not do — and that same call is what puts the console in
    // raw mode, see above. On Unix crossterm emits those sequences.
    execute!(stdout, EnableMouseCapture)?;
    stdout.flush()?;
    Ok(stdout)
}

fn restore_terminal() {
    #[cfg(unix)]
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    // On Windows this restores the console input mode captured before mouse capture — which is
    // the mode the process started with, raw-mode bits included.
    let _ = execute!(stdout, DisableMouseCapture);
    // Show the cursor (?25h) and leave the alternate screen (?1049l): mirror of init_terminal.
    let _ = stdout.write_all(b"\x1b[?25h\x1b[?1049l");
    let _ = stdout.flush();
}

fn run(out: &mut Stdout) -> io::Result<()> {
    let mut app = App::new();
    let mut size = terminal::size()?;
    let mut dirty = true;
    let mut grid = ui::build_grid(
        size.0 as usize,
        size.1 as usize,
        &app.game,
        app.message(),
        app.controls,
    ); // kept for hit-testing

    loop {
        if dirty {
            grid = ui::build_grid(
                size.0 as usize,
                size.1 as usize,
                &app.game,
                app.message(),
                app.controls,
            );
            ui::render(out, &grid)?;
            dirty = false;
        }

        // Blocks: nothing here runs on a clock, so there is no reason to wake without an event.
        match event::next()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                dirty = true;
                if handle_key(&mut app, key) {
                    break;
                }
            }
            Event::Mouse(m) => {
                // A click on a key/button hit-tests to the keypress it stands for, then
                // runs through the exact same path as a real keypress.
                if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                    if let Some(key) = grid.hit_test(m.column as usize, m.row as usize) {
                        dirty = true;
                        if handle_key(&mut app, key) {
                            break;
                        }
                    }
                }
            }
            Event::Resize(w, h) => {
                size = (w, h);
                out.write_all(b"\x1b[2J")?; // clear screen (crossterm::terminal::Clear(All))
                dirty = true;
            }
            _ => {}
        }
    }

    Ok(())
}

// Apply one keypress (typed or replayed from a click). Returns true when it asks to quit.
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    match app.phase() {
        Phase::Playing => match key.code {
            KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char(c) if c.is_ascii_alphabetic() => {
                app.type_letter(c.to_ascii_lowercase() as u8)
            }
            KeyCode::Backspace => app.backspace(),
            KeyCode::Enter => app.submit(),
            _ => {}
        },
        Phase::Won | Phase::Lost => match key.code {
            KeyCode::Enter => *app = App::new(),
            KeyCode::Esc => return true,
            _ => {}
        },
    }
    false
}

fn main() {
    // Minimal hook: restore the terminal, then exit. We `exit(101)` rather than return
    // (which would let panic = "abort" call abort()): on Windows abort() hands off
    // to Windows Error Reporting, a ~1s stall before the shell returns. exit() skips it.
    // Gated out under `--cfg immediate_abort`: that build compiles every panic to a bare
    // abort that bypasses the panic runtime (and thus this hook) entirely, so registering
    // one there is pure dead weight. See OPTIMIZATION.md "immediate-abort safety".
    #[cfg(not(immediate_abort))]
    std::panic::set_hook(Box::new(|_| {
        restore_terminal();
        std::process::exit(101);
    }));

    if let Ok(mut terminal) = init_terminal() {
        let _ = run(&mut terminal);
        restore_terminal();
    }
}
