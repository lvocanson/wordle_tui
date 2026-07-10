use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use crossterm::{cursor, execute, queue};

mod app;
mod codec;
mod game;
mod ui;

use app::{App, Phase};
use ui::Action;

fn init_terminal() -> io::Result<Stdout> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        terminal::SetSize(ui::PREFERRED_SIZE.0, ui::PREFERRED_SIZE.1),
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
    let mut grid = ui::build_grid(size.0 as usize, size.1 as usize, &app); // kept for hit-testing

    loop {
        if dirty {
            grid = ui::build_grid(size.0 as usize, size.1 as usize, &app);
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

                    match app.phase {
                        Phase::Playing => match key.code {
                            KeyCode::Esc => break,
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                break
                            }
                            KeyCode::Char(c) if c.is_ascii_alphabetic() => {
                                app.type_letter(c.to_ascii_lowercase() as u8)
                            }
                            KeyCode::Backspace => app.backspace(),
                            KeyCode::Enter => app.submit(),
                            _ => {}
                        },
                        Phase::Won | Phase::Lost => match key.code {
                            KeyCode::Enter => app = App::new(),
                            KeyCode::Esc => break,
                            _ => {}
                        },
                    }
                }
                Event::Mouse(m) => {
                    if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                        if let Some(action) = grid.hit_test(m.column as usize, m.row as usize) {
                            dirty = true;
                            handle_action(&mut app, action);
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

// Replay a click on a key or button as the equivalent keypress.
fn handle_action(app: &mut App, action: Action) {
    match app.phase {
        Phase::Playing => match action {
            Action::Back => app.backspace(),
            Action::Enter => app.submit(),
            Action::Letter(l) => app.type_letter(l),
        },
        Phase::Won | Phase::Lost => {
            if action == Action::Enter {
                *app = App::new();
            }
        }
    }
}

fn main() {
    // Minimal hook: restore the terminal, then exit. We `exit(101)` rather than return
    // (which would let panic = "abort" call abort()): on Windows abort() hands off
    // to Windows Error Reporting, a ~1s stall before the shell returns. exit() skips it.
    std::panic::set_hook(Box::new(|_| {
        restore_terminal();
        std::process::exit(101);
    }));

    if let Ok(mut terminal) = init_terminal() {
        let _ = run(&mut terminal);
        restore_terminal();
    }
}
