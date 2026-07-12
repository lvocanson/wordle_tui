use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use crossterm::{cursor, execute, queue};

mod app;
mod codec;
mod game;
mod ui;
mod words;

use app::App;
use game::Phase;

fn init_terminal() -> io::Result<Stdout> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // No SetSize: the layout adapts to whatever size the terminal is (see ui::build_grid), and
    // forcing a resize corrupts the alternate screen's cursor restore, leaving the shell prompt
    // drawn over old output on exit. Not touching the size keeps ?1049 save/restore clean.
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        SetTitle("Wordle"),
        cursor::Hide
    )?;
    Ok(stdout)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        cursor::Show,
        LeaveAlternateScreen
    );
}

fn run(out: &mut Stdout) -> io::Result<()> {
    let mut app = App::new();
    let mut size = terminal::size()?;
    let mut dirty = true;
    let mut grid = ui::build_grid(size.0 as usize, size.1 as usize, &app.game, app.message.as_deref(), app.controls); // kept for hit-testing

    loop {
        if dirty {
            grid = ui::build_grid(size.0 as usize, size.1 as usize, &app.game, app.message.as_deref(), app.controls);
            ui::render(out, &grid)?;
            dirty = false;
        }

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
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
                    queue!(out, terminal::Clear(terminal::ClearType::All))?;
                    dirty = true;
                }
                _ => {}
            }
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
