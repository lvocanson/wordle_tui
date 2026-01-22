use rand::SeedableRng;
use rand::prelude::{IndexedRandom, StdRng};

pub struct GameBuilder {
    word_length: usize,
    max_guesses: usize,
    word_pool: Vec<String>,
    dictionary: Dictionary,
}

#[derive(Debug)]
pub enum GameBuilderError {
    WordPoolIsEmpty,
}

impl GameBuilder {
    pub fn new(word_length: usize, max_guesses: usize) -> Self {
        Self {
            word_length,
            max_guesses,
            word_pool: vec![],
            dictionary: Dictionary::WordPool(vec![]),
        }
    }

    pub fn add_words_to_pool(mut self, words: &[&str]) -> Self {
        let words = words
            .iter()
            .filter(|w| w.chars().count() == self.word_length);
        self.word_pool.extend(words.map(|s| s.to_uppercase()));
        self
    }

    pub fn disable_dictionary(mut self) -> Self {
        self.dictionary = Dictionary::NoRestriction;
        self
    }

    pub fn reset_dictionary(mut self) -> Self {
        self.dictionary = Dictionary::WordPool(vec![]);
        self
    }

    pub fn add_words_to_dictionary(mut self, words: &[&str]) -> Self {
        if let Dictionary::WordPool(dico) = &mut self.dictionary {
            let words = words
                .iter()
                .filter(|w| w.chars().count() == self.word_length);
            dico.extend(words.map(|s| s.to_uppercase()));
        } else {
            self.dictionary = Dictionary::WordPool(words.iter().map(|s| s.to_string()).collect());
        }
        self
    }

    pub fn build(mut self) -> Result<Game, GameBuilderError> {
        let secret: String = self
            .word_pool
            .choose(&mut StdRng::from_os_rng())
            .ok_or(GameBuilderError::WordPoolIsEmpty)?
            .clone();

        if let Dictionary::WordPool(words) = &mut self.dictionary {
            words.append(&mut self.word_pool);
            words.sort();
            words.dedup();
        }

        Ok(Game {
            dictionary: self.dictionary,
            secret,
            guesses: Vec::with_capacity(self.max_guesses),
            state: GameState::Playing,
        })
    }
}

enum Dictionary {
    NoRestriction,
    WordPool(Vec<String>),
}

impl Dictionary {
    fn is_word_allowed(&self, word: &str) -> bool {
        match self {
            Dictionary::NoRestriction => true,
            Dictionary::WordPool(pool) => pool.binary_search(&word.to_string()).is_ok(),
        }
    }
}

pub struct Game {
    dictionary: Dictionary,
    secret: String,
    guesses: Vec<String>,
    state: GameState,
}

#[derive(Clone, Copy)]
pub enum CharHint {
    Correct,
    Misplaced,
    NotPresent,
}

pub enum GuessResult {
    NotAllowed,
    Correct,
    Incorrect(Vec<CharHint>),
}

#[derive(Clone, Copy)]
pub enum GameState {
    Playing,
    Won,
    Lost,
}

impl Game {
    pub fn guess(&mut self, guess: &str) -> GuessResult {
        if !matches!(self.state, GameState::Playing) {
            return GuessResult::NotAllowed;
        };

        let guess = guess.to_uppercase();
        if !self.dictionary.is_word_allowed(&guess) {
            return GuessResult::NotAllowed;
        }

        self.guesses.push(guess.to_string());

        if self.secret == guess {
            self.state = GameState::Won;
            return GuessResult::Correct;
        }

        if self.guesses.len() == self.guesses.capacity() {
            self.state = GameState::Lost;
        }

        let mut s: Vec<char> = self.secret.chars().collect();
        let mut c: Vec<char> = guess.chars().collect();
        let len = s.len();

        let mut hints = vec![CharHint::NotPresent; len];

        for i in 0..len {
            if c[i] == s[i] {
                s[i] = '\0';
                c[i] = '\0';
                hints[i] = CharHint::Correct;
            }
        }

        for i in 0..len {
            if c[i] == '\0' {
                continue;
            }
            if let Some(idx) = s.iter().position(|&x| x == c[i]) {
                s[idx] = '\0';
                hints[i] = CharHint::Misplaced;
            }
        }

        GuessResult::Incorrect(hints)
    }

    pub fn state(&self) -> GameState {
        self.state
    }

    pub fn get_answer(&self) -> Option<&str> {
        if matches!(self.state, GameState::Playing) {
            None
        } else {
            Some(&self.secret)
        }
    }

    pub fn guesses(&self) -> (usize, usize) {
        (self.guesses.len(), self.guesses.capacity())
    }
}
