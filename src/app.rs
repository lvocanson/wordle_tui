use crate::game::{self, LetterState, MAX_GUESSES, WORD_LEN};

const PLAYING_CONTROLS: &str = "<Enter> Submit  <Esc> Quit";
const END_CONTROLS: &str = "<R/Enter> Restart  <Q/Esc> Quit";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Playing,
    Won,
    Lost,
}

pub struct App {
    pub target: &'static [u8],
    pub history: [([u8; WORD_LEN], [LetterState; WORD_LEN]); MAX_GUESSES],
    pub input_idx: usize,
    pub input_len: usize,
    pub phase: Phase,
    pub message: Option<String>,
    pub controls: &'static str,
}

impl App {
    pub fn new() -> Self {
        let target = game::pick_target(game::random_seed());
        App {
            target,
            history: [([0; WORD_LEN], [LetterState::WrongSpot; WORD_LEN]); MAX_GUESSES],
            input_idx: 0,
            input_len: 0,
            phase: Phase::Playing,
            message: None,
            controls: PLAYING_CONTROLS,
        }
    }

    pub fn type_letter(&mut self, l: u8) {
        if self.input_len < WORD_LEN {
            self.history[self.input_idx].0[self.input_len] = l;
            self.input_len += 1;
            self.message = None;
        }
    }

    pub fn backspace(&mut self) {
        if self.input_len > 0 {
            self.input_len -= 1;
            self.history[self.input_idx].0[self.input_len] = 0u8;
            self.message = None;
        }
    }

    pub fn submit(&mut self) {
        if self.input_len != WORD_LEN {
            self.message = Some("Incorrect size.".to_string());
            return;
        }

        let guess = &self.history[self.input_idx].0;
        if !game::is_valid(guess) {
            self.message = Some("Invalid word.".to_string());
            return;
        }

        self.history[self.input_idx].1 = game::check(self.target, guess);
        self.input_idx += 1;
        self.input_len = 0;
        self.message = None;

        let mut word = [0u8; WORD_LEN];
        word.copy_from_slice(guess);

        if &word[..] == self.target {
            self.phase = Phase::Won;
            self.controls = END_CONTROLS;
            self.message = Some(format!("Found in {} guesses. {}", self.input_idx, match self.input_idx {
                1 => "Genius!",
                2 => "Magnificent!",
                3 => "Impressive!",
                4 => "Splendid!",
                5 => "Great!",
                game::MAX_GUESSES => "Phew!",
                _ => "Good!",
            }));
        } else if self.input_idx >= game::MAX_GUESSES {
            self.phase = Phase::Lost;
            self.controls = END_CONTROLS;
            let word = str::from_utf8(self.target).map(|str| str.to_ascii_uppercase());
            self.message = Some(format!(
                "The word was {}.",
                word.unwrap_or_else(|_| "?????".to_string())
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_target(target: &'static [u8]) -> App {
        App {
            target,
            history: [([0; WORD_LEN], [LetterState::WrongSpot; WORD_LEN]); MAX_GUESSES],
            input_idx: 0,
            input_len: 0,
            phase: Phase::Playing,
            message: None,
            controls: PLAYING_CONTROLS,
        }
    }

    fn type_word(app: &mut App, word: &str) {
        for c in word.chars() {
            app.type_letter(c as u8);
        }
    }

    #[test]
    fn invalid_word_sets_message_and_does_not_consume_a_guess() {
        let mut app = app_with_target(b"crane");
        type_word(&mut app, "zzzzz");
        app.submit();

        assert!(app.message.is_some());
        assert!(app.input_idx == 0);
        assert_eq!(app.phase, Phase::Playing);
    }

    #[test]
    fn valid_wrong_word_advances_without_winning() {
        let mut app = app_with_target(b"crane");
        type_word(&mut app, "slate");
        app.submit();

        assert_eq!(app.input_idx, 1);
        assert_eq!(app.phase, Phase::Playing);
        assert_eq!(app.input_len, 0);
    }

    #[test]
    fn matching_word_wins() {
        let mut app = app_with_target(b"crane");
        type_word(&mut app, "crane");
        app.submit();

        assert_eq!(app.phase, Phase::Won);
        assert_eq!(
            app.history[app.input_idx - 1].1,
            [LetterState::CorrectSpot; 5]
        );
    }

    #[test]
    fn max_wrong_guesses_loses() {
        let mut app = app_with_target(b"crane");
        for _ in 0..game::MAX_GUESSES {
            type_word(&mut app, "slate");
            app.submit();
        }

        assert_eq!(app.phase, Phase::Lost);
        assert_eq!(app.history.len(), game::MAX_GUESSES);
    }

    #[test]
    fn backspace_removes_last_typed_char() {
        let mut app = app_with_target(b"crane");
        type_word(&mut app, "cra");
        app.backspace();

        assert_eq!(app.input_len, 2);
        assert_eq!(&app.history[app.input_idx].0[..2], b"cr");
    }
}
