// The game itself: the rules of Wordle over a single hidden word. `Game` holds strictly the
// data of a game in progress — the target and the guesses committed so far — and nothing about
// how it is drawn or how input is edited (that lives in app.rs / ui.rs). Which strings count as
// words is the word database's job (words.rs).

use crate::words::{self, WORD_LEN};

pub const MAX_GUESSES: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LetterState {
    Correct,
    Misplaced,
    Absent,
}

// A committed guess: the word played and, per letter, how it scored against the target.
#[derive(Clone, Copy)]
pub struct Guess {
    pub word: [u8; WORD_LEN],
    pub result: [LetterState; WORD_LEN],
}

// Derived from the guesses, never stored: won once a guess scores all-Correct, lost once the
// guess budget is spent without a win, playing otherwise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Playing,
    Won,
    Lost,
}

pub struct Game {
    target: [u8; WORD_LEN],
    guesses: Vec<Guess>,
}

impl Game {
    pub fn new() -> Self {
        Game::with_target(words::pick_target(words::random_seed()))
    }

    fn with_target(target: [u8; WORD_LEN]) -> Self {
        Game {
            target,
            guesses: Vec::new(),
        }
    }

    // The guesses committed so far, oldest first.
    pub fn guesses(&self) -> &[Guess] {
        &self.guesses
    }

    pub fn target(&self) -> &[u8; WORD_LEN] {
        &self.target
    }

    pub fn phase(&self) -> Phase {
        if self.guesses.last().is_some_and(|g| g.result == [LetterState::Correct; WORD_LEN]) {
            Phase::Won
        } else if self.guesses.len() >= MAX_GUESSES {
            Phase::Lost
        } else {
            Phase::Playing
        }
    }

    // Validate `word` and, if it is a real word, record it as a guess. Returns whether it was
    // accepted; a rejected word costs no guess. Caller guarantees the phase is still Playing.
    pub fn submit(&mut self, word: &[u8; WORD_LEN]) -> bool {
        if !words::is_valid(word) {
            return false;
        }
        self.guesses.push(Guess {
            word: *word,
            result: check(&self.target, word),
        });
        true
    }
}

fn check(target: &[u8], guess: &[u8]) -> [LetterState; WORD_LEN] {
    let mut res = [LetterState::Absent; WORD_LEN];
    let mut used = [false; WORD_LEN];
    for i in 0..WORD_LEN {
        if guess[i] == target[i] {
            res[i] = LetterState::Correct;
            used[i] = true;
        }
    }
    for i in 0..WORD_LEN {
        if res[i] == LetterState::Correct {
            continue;
        }
        for j in 0..WORD_LEN {
            if !used[j] && guess[i] == target[j] {
                res[i] = LetterState::Misplaced;
                used[j] = true;
                break;
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(game: &mut Game, word: &str) -> bool {
        let mut w = [0u8; WORD_LEN];
        w.copy_from_slice(word.as_bytes());
        game.submit(&w)
    }

    #[test]
    fn invalid_word_is_rejected_and_costs_no_guess() {
        let mut game = Game::with_target(*b"crane");
        assert!(!play(&mut game, "zzzzz"));
        assert_eq!(game.guesses().len(), 0);
        assert_eq!(game.phase(), Phase::Playing);
    }

    #[test]
    fn valid_wrong_word_advances_without_winning() {
        let mut game = Game::with_target(*b"crane");
        assert!(play(&mut game, "slate"));
        assert_eq!(game.guesses().len(), 1);
        assert_eq!(game.phase(), Phase::Playing);
    }

    #[test]
    fn matching_word_wins() {
        let mut game = Game::with_target(*b"crane");
        assert!(play(&mut game, "crane"));
        assert_eq!(game.phase(), Phase::Won);
        assert_eq!(game.guesses().last().unwrap().result, [LetterState::Correct; WORD_LEN]);
    }

    #[test]
    fn max_wrong_guesses_loses() {
        let mut game = Game::with_target(*b"crane");
        for _ in 0..MAX_GUESSES {
            play(&mut game, "slate");
        }
        assert_eq!(game.phase(), Phase::Lost);
        assert_eq!(game.guesses().len(), MAX_GUESSES);
    }
}
