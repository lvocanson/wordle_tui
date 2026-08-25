// The game itself: the rules of Wordle over a single hidden word. `Game` holds strictly the data
// of a game in progress — the hidden target, the guesses committed so far, and the word currently
// being typed (the draft) — and nothing about how it is drawn (ui.rs) or the chrome around it
// (app.rs). Which strings count as words is the word database's job (words.rs).

use crate::words::{self, WORD_LEN};

// Re-exported so callers keep importing it from the rules module; the value itself is generated
// into constants.rs from the WORDLE_MAX_GUESSES build-time env var (default 6). See words.rs.
pub use crate::words::MAX_GUESSES;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Score {
    Correct,
    Misplaced,
    Absent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LetterState {
    Empty,
    Draft,
    Submitted(Score),
}

// A committed guess: the word played and, per letter, how it scored against the target.
#[derive(Clone, Copy)]
pub struct Guess {
    pub word: [u8; WORD_LEN],
    pub result: [LetterState; WORD_LEN],
}

impl Guess {
    fn new_empty() -> Self {
        Self {
            word: [b' '; WORD_LEN],
            result: [LetterState::Empty; WORD_LEN],
        }
    }
}

// Derived from the guesses, never stored: won once a guess scores all-Correct, lost once the
// guess budget is spent without a win, playing otherwise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Playing,
    Won,
    Lost,
}

#[derive(Debug, PartialEq)]
pub enum SubmitError {
    TooShort,
    InvalidWord,
}

pub struct Game {
    target: [u8; WORD_LEN],
    guesses: [Guess; MAX_GUESSES],
    draft_idx: usize,
    draft_len: usize,
    // Best score each of the 26 letters has earned, kept in sync at submit so the keyboard
    // can be colored without rescanning the guesses. Indexed by (letter - b'a'); None until
    // the letter is first played.
    keyboard: [Option<Score>; 26],
}

impl Game {
    pub fn new() -> Self {
        Game::with_target(words::pick_target(words::random_seed()))
    }

    fn with_target(target: [u8; WORD_LEN]) -> Self {
        Game {
            target,
            guesses: [Guess::new_empty(); MAX_GUESSES],
            keyboard: [None; 26],
            draft_idx: 0,
            draft_len: 0,
        }
    }

    pub fn board(&self) -> &[Guess; MAX_GUESSES] {
        &self.guesses
    }

    pub fn nb_guesses(&self) -> usize {
        self.draft_idx
    }

    pub fn target(&self) -> &[u8; WORD_LEN] {
        &self.target
    }

    // The best state a letter has earned across the committed guesses,
    // or None if it has not been played yet.
    pub fn get_letter_state(&self, letter: u8) -> Option<Score> {
        self.keyboard[(letter - b'a') as usize]
    }

    pub fn phase(&self) -> Phase {
        if self.draft_idx > 0
            && self.guesses[self.draft_idx - 1].result
                == [LetterState::Submitted(Score::Correct); WORD_LEN]
        {
            Phase::Won
        } else if self.draft_idx >= MAX_GUESSES {
            Phase::Lost
        } else {
            Phase::Playing
        }
    }

    fn set_current_draft_letter(&mut self, letter: u8, state: LetterState) {
        let draft = &mut self.guesses[self.draft_idx];
        draft.word[self.draft_len] = letter;
        draft.result[self.draft_len] = state;
    }

    // Append a letter to the draft; returns whether it fit (the draft is capped at WORD_LEN).
    pub fn type_letter(&mut self, letter: u8) -> bool {
        if self.draft_len < WORD_LEN {
            self.set_current_draft_letter(letter, LetterState::Draft);
            self.draft_len += 1;
            true
        } else {
            false
        }
    }

    // Remove the last drafted letter; returns whether there was one to remove.
    pub fn backspace(&mut self) -> bool {
        if self.draft_len > 0 {
            self.draft_len -= 1;
            self.set_current_draft_letter(b' ', LetterState::Empty);
            true
        } else {
            false
        }
    }

    // Validate the draft and, if it is a real word, commit it as a guess and clear the draft. A
    // rejected word keeps the draft untouched. The caller guarantees the phase is still Playing.
    pub fn submit(&mut self) -> Result<(), SubmitError> {
        if self.draft_len != WORD_LEN {
            return Err(SubmitError::TooShort);
        }
        if !words::is_valid(&self.guesses[self.draft_idx].word) {
            return Err(SubmitError::InvalidWord);
        }

        let guess = &mut self.guesses[self.draft_idx];
        let scores = check(&self.target, &guess.word);
        for ((res, &score), &ch) in guess.result.iter_mut().zip(&scores).zip(&guess.word) {
            *res = LetterState::Submitted(score);
            let key = &mut self.keyboard[(ch - b'a') as usize];
            *key = Some(better_score(*key, score));
        }
        self.draft_idx += 1;
        self.draft_len = 0;
        Ok(())
    }
}

// Merge a letter's already-known keyboard score with a fresh occurrence, keeping the
// stronger of the two: Correct > Misplaced > Absent.
fn better_score(a: Option<Score>, b: Score) -> Score {
    use Score::*;
    match (a, b) {
        (_, Correct) | (Some(Correct), _) => Correct,
        (_, Misplaced) | (Some(Misplaced), _) => Misplaced,
        _ => Absent,
    }
}

fn check(target: &[u8], guess: &[u8]) -> [Score; WORD_LEN] {
    let mut res = [Score::Absent; WORD_LEN];
    let mut used = [false; WORD_LEN];
    for i in 0..WORD_LEN {
        if guess[i] == target[i] {
            res[i] = Score::Correct;
            used[i] = true;
        }
    }
    for i in 0..WORD_LEN {
        if res[i] == Score::Correct {
            continue;
        }
        for j in 0..WORD_LEN {
            if !used[j] && guess[i] == target[j] {
                res[i] = Score::Misplaced;
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

    fn play(game: &mut Game, word: &str) -> Result<(), SubmitError> {
        for &b in word.as_bytes() {
            game.type_letter(b);
        }
        game.submit()
    }

    #[test]
    fn invalid_word_is_rejected_and_costs_no_guess() {
        let mut game = Game::with_target(*b"crane");
        assert_eq!(play(&mut game, "zzzzz"), Err(SubmitError::InvalidWord));
        assert_eq!(game.nb_guesses(), 0);
        assert_eq!(game.phase(), Phase::Playing);
    }

    #[test]
    fn valid_wrong_word_advances_without_winning() {
        let mut game = Game::with_target(*b"crane");
        assert_eq!(play(&mut game, "slate"), Ok(()));
        assert_eq!(game.nb_guesses(), 1);
        assert_eq!(game.phase(), Phase::Playing);
    }

    #[test]
    fn matching_word_wins() {
        let mut game = Game::with_target(*b"crane");
        assert_eq!(play(&mut game, "crane"), Ok(()));
        assert_eq!(game.phase(), Phase::Won);
        assert_eq!(
            game.guesses[0].result,
            [LetterState::Submitted(Score::Correct); WORD_LEN]
        );
    }

    #[test]
    fn max_wrong_guesses_loses() {
        let mut game = Game::with_target(*b"crane");
        for _ in 0..MAX_GUESSES {
            _ = play(&mut game, "slate");
        }
        assert_eq!(game.phase(), Phase::Lost);
        assert_eq!(game.nb_guesses(), MAX_GUESSES);
    }

    #[test]
    fn backspace_removes_last_drafted_letter() {
        let mut game = Game::with_target(*b"crane");
        for &b in b"cra" {
            game.type_letter(b);
        }
        assert!(game.backspace());
        assert_eq!(game.draft_len, 2);
        assert_eq!(&game.guesses[game.draft_idx].word[..2], b"cr");
    }

    #[test]
    fn draft_is_capped_at_word_len() {
        let mut game = Game::with_target(*b"crane");
        for &b in b"crane" {
            assert!(game.type_letter(b));
        }
        assert!(!game.type_letter(b'x')); // sixth letter does not fit
        assert_eq!(game.draft_len, WORD_LEN);
    }
}
