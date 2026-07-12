// The controller: it owns a `Game` (the pure game state) and adds everything around playing it
// that is not game data — the letters being typed but not yet submitted (`draft`), and the
// presentation chrome (`message`, `controls`). Input events land here; drawing reads from here.

use crate::game::{Game, Phase, MAX_GUESSES};
use crate::words::WORD_LEN;

const PLAYING_CONTROLS: &str = "<Enter> Submit  <Esc> Quit";
const END_CONTROLS: &str = "<Enter> Restart <Esc> Quit";

pub struct App {
    pub game: Game,
    // The word currently being typed: `draft[..draft_len]` are the entered letters, not yet
    // a guess. Editing state, deliberately kept out of `Game`.
    pub draft: [u8; WORD_LEN],
    pub draft_len: usize,
    pub message: Option<String>,
    pub controls: &'static str,
}

impl App {
    pub fn new() -> Self {
        App {
            game: Game::new(),
            draft: [0; WORD_LEN],
            draft_len: 0,
            message: None,
            controls: PLAYING_CONTROLS,
        }
    }

    pub fn phase(&self) -> Phase {
        self.game.phase()
    }

    pub fn type_letter(&mut self, letter: u8) {
        if self.draft_len < WORD_LEN {
            self.draft[self.draft_len] = letter;
            self.draft_len += 1;
            self.message = None;
        }
    }

    pub fn backspace(&mut self) {
        if self.draft_len > 0 {
            self.draft_len -= 1;
            self.draft[self.draft_len] = 0;
            self.message = None;
        }
    }

    pub fn submit(&mut self) {
        if self.draft_len != WORD_LEN {
            self.message = Some("Incorrect size.".to_string());
            return;
        }
        if !self.game.submit(&self.draft) {
            self.message = Some("Invalid word.".to_string());
            return;
        }

        self.draft = [0; WORD_LEN];
        self.draft_len = 0;
        self.message = None;

        match self.game.phase() {
            Phase::Won => {
                self.controls = END_CONTROLS;
                let n = self.game.guesses().len();
                self.message = Some(format!(
                    "Found in {n} guesses. {}",
                    match n {
                        1 => "Genius!",
                        2 => "Magnificent!",
                        3 => "Impressive!",
                        4 => "Splendid!",
                        5 => "Great!",
                        MAX_GUESSES => "Phew!",
                        _ => "Good!",
                    }
                ));
            }
            Phase::Lost => {
                self.controls = END_CONTROLS;
                let word = str::from_utf8(self.game.target()).map(|s| s.to_ascii_uppercase());
                self.message = Some(format!(
                    "The word was {}.",
                    word.unwrap_or_else(|_| "?????".to_string())
                ));
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
    fn wrong_size_sets_message_and_costs_no_guess() {
        let mut app = App::new();
        type_word(&mut app, "cra");
        app.submit();

        assert!(app.message.is_some());
        assert_eq!(app.game.guesses().len(), 0);
    }

    #[test]
    fn invalid_word_sets_message_and_costs_no_guess() {
        let mut app = App::new();
        type_word(&mut app, "zzzzz");
        app.submit();

        assert!(app.message.is_some());
        assert_eq!(app.game.guesses().len(), 0);
        assert_eq!(app.phase(), Phase::Playing);
    }

    #[test]
    fn backspace_removes_last_typed_char() {
        let mut app = App::new();
        type_word(&mut app, "cra");
        app.backspace();

        assert_eq!(app.draft_len, 2);
        assert_eq!(&app.draft[..2], b"cr");
    }
}
