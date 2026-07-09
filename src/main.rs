use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod app;
mod game;
mod ui;

use app::{App, Phase};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn init_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, SetTitle("Wordle"))?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

fn run(terminal: &mut Term) -> io::Result<()> {
    let mut app = App::new();

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

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

fn main() -> io::Result<()> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    let mut terminal = init_terminal()?;
    let result = run(&mut terminal);
    restore_terminal();
    result
}
