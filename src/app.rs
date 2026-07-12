// The controller: it owns a `Game` (the pure game state) and adds everything around playing it
// that is not game data — the letters being typed but not yet submitted (`draft`), and the
// presentation chrome (`message`, `controls`). Input events land here; drawing reads from here.

use crate::game::{Game, Phase, SubmitError, MAX_GUESSES};

const PLAYING_CONTROLS: &str = "<Enter> Submit  <Esc> Quit";
const END_CONTROLS: &str = "<Enter> Restart <Esc> Quit";

pub struct App {
    pub game: Game,
    pub message: Option<String>,
    pub controls: &'static str,
}

// Append `n` in decimal to `s`. A hand-rolled alternative to `write!`, which drags in the
// `fmt::Arguments`/`fmt::write` machinery. The caller only ever passes a guess count, so `n` is
// bounded by `MAX_GUESSES`: when that fits in one digit (the usual case) the whole general
// branch is dead code the compiler drops, leaving a single `push`. The `% 10` loop stays as a
// correct fallback should `MAX_GUESSES` ever exceed 9. `b'0' + d` is always in range since `d`
// is a single digit.
fn push_decimal(s: &mut String, mut n: usize) {
    if MAX_GUESSES <= 9 {
        s.push((b'0' + n as u8) as char);
        return;
    }
    const USIZE_MAX_DIGITS: usize = usize::MAX.ilog10() as usize + 1;
    let mut buf = [0u8; USIZE_MAX_DIGITS];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &b in &buf[i..] {
        s.push(b as char);
    }
}

impl App {
    pub fn new() -> Self {
        App {
            game: Game::new(),
            message: None,
            controls: PLAYING_CONTROLS,
        }
    }

    pub fn phase(&self) -> Phase {
        self.game.phase()
    }

    pub fn type_letter(&mut self, letter: u8) {
        if self.game.type_letter(letter) {
            self.message = None;
        }
    }

    pub fn backspace(&mut self) {
        if self.game.backspace() {
            self.message = None;
        }
    }

    pub fn submit(&mut self) {
        if let Err(error) = self.game.submit() {
            match error {
                SubmitError::TooShort => {
                    self.message = Some("Incorrect size.".to_string());
                }
                SubmitError::InvalidWord => {
                    self.message = Some("Invalid word.".to_string());
                }
            }
            return;
        }
        self.message = None;

        match self.game.phase() {
            Phase::Won => {
                self.controls = END_CONTROLS;
                let n = self.game.nb_guesses();
                let adjective = match n {
                    1 => "Genius!",
                    2 => "Magnificent!",
                    3 => "Impressive!",
                    4 => "Splendid!",
                    5 => "Great!",
                    MAX_GUESSES => "Phew!",
                    _ => "Good!",
                };
                let mut msg = String::from("Found in ");
                push_decimal(&mut msg, n);
                msg.push_str(" guesses. ");
                msg.push_str(adjective);
                self.message = Some(msg);
            }
            Phase::Lost => {
                self.controls = END_CONTROLS;
                let mut upper = *self.game.target();
                upper.make_ascii_uppercase();
                let word = str::from_utf8(&upper).unwrap_or("?????");
                let mut msg = String::from("The word was ");
                msg.push_str(word);
                msg.push('.');
                self.message = Some(msg);
            }
            Phase::Playing => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_word(app: &mut App, word: &str) {
        for c in word.chars() {
            app.type_letter(c as u8);
        }
    }

    #[test]
    fn wrong_size_sets_message() {
        let mut app = App::new();
        type_word(&mut app, "cra");
        app.submit();
        assert!(app.message.is_some());
    }

    #[test]
    fn invalid_word_sets_message() {
        let mut app = App::new();
        type_word(&mut app, "zzzzz");
        app.submit();
        assert!(app.message.is_some());
    }
}
