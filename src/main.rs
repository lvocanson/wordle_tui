use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use crossterm::{cursor, execute, queue};

mod app;
mod game;
mod ui;

#[cfg(test)]
mod snapshot_tests;

#[cfg(feature = "ratatui-ref")]
mod ui_ref;

use app::{App, Phase};

fn init_terminal() -> io::Result<Stdout> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        terminal::SetSize(ui::PREFERRED_SIZE.0, ui::PREFERRED_SIZE.1),
        SetTitle("Wordle"),
        cursor::Hide
    )?;
    Ok(stdout)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
}

fn run(out: &mut Stdout) -> io::Result<()> {
    let mut app = App::new();
    let mut last_size = (0u16, 0u16);
    let mut dirty = true;

    loop {
        let size = terminal::size()?;
        if size != last_size {
            queue!(out, terminal::Clear(terminal::ClearType::All))?;
            last_size = size;
            dirty = true;
        }
        if dirty {
            let grid = ui::build_grid(size.0 as usize, size.1 as usize, &app);
            ui::render(out, &grid)?;
            dirty = false;
        }

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
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
                        KeyCode::Enter | KeyCode::Char('r') | KeyCode::Char('R') => {
                            app = App::new()
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => break,
                        _ => {}
                    },
                }
            }
        }
    }

    Ok(())
}

fn main() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    if let Ok(mut terminal) = init_terminal() {
        let _ = run(&mut terminal);
        restore_terminal();
    }
}
