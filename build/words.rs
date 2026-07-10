// Reading, validating and merging the two source word lists.
//
// A word list is one lowercase a-z word per non-empty line. Nothing here asserts a length;
// the single length shared by the whole game is inferred from the data (see `word_length`).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::{fs, io};

// A length-L word packs into the base-26 range 0..26^L, the space the model's contexts live
// in. That fits the arithmetic model comfortably; the practical limit is only that a single
// character index stays in 0..25, so any fixed length is fine. We still cap the length to
// keep the u64 base-26 identity (used for the disjointness/sort reasoning) unambiguous.
const MAX_WORD_LEN: usize = 13;

// Everything that can go wrong while turning the source files into a clean word set. The
// build script surfaces these from `main`, so they only need to render a readable message.
pub enum WordsError {
    Io(io::Error),
    NotLowercaseAscii(Box<str>),
    MixedLengths(BTreeMap<usize, usize>),
    TooLong { len: usize, max: usize },
}

impl fmt::Display for WordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WordsError::Io(e) => write!(f, "{e}"),
            WordsError::NotLowercaseAscii(w) => {
                write!(f, "word {w:?} is not made of lowercase a-z letters only")
            }
            WordsError::MixedLengths(by_len) => {
                write!(f, "words are not all the same length:")?;
                for (len, count) in by_len {
                    write!(f, "\n  {count} word(s) of length {len}")?;
                }
                Ok(())
            }
            WordsError::TooLong { len, max } => write!(
                f,
                "all words are length {len}, but the base-26/u64 identity used to sort and dedup \
                 cannot encode words longer than {max} (26^{len} overflows u64)"
            ),
        }
    }
}

// A word proven to be lowercase ASCII, the only shape the rest of the pipeline accepts.
// Ordering is the plain lexicographic order the encoder relies on to front-code and dedup.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct LowercaseAsciiWord(Box<str>);

impl LowercaseAsciiWord {
    pub fn new(word: Box<str>) -> Result<Self, Box<str>> {
        if word.bytes().all(|b| b.is_ascii_lowercase()) {
            Ok(Self(word))
        } else {
            Err(word)
        }
    }

    // The word as 0..25 symbol values, the alphabet the model works in.
    pub fn symbols(&self) -> Vec<u8> {
        self.bytes().map(|b| b - b'a').collect()
    }
}

impl std::ops::Deref for LowercaseAsciiWord {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Read a word list, validating every non-empty line as a lowercase ASCII word.
pub fn read_words(path: &Path) -> Result<Vec<LowercaseAsciiWord>, WordsError> {
    let text = fs::read_to_string(path).map_err(WordsError::Io)?;
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|word| LowercaseAsciiWord::new(word.into()).map_err(WordsError::NotLowercaseAscii))
        .collect()
}

// Infer the single word length shared by every word, or fail with a readable breakdown of
// how many words fall in each length.
pub fn word_length(lengths: impl Iterator<Item = usize>) -> Result<usize, WordsError> {
    let mut by_len: BTreeMap<usize, usize> = BTreeMap::new();
    for len in lengths {
        *by_len.entry(len).or_default() += 1;
    }

    let mut keys = by_len.keys().copied();
    match (keys.next(), keys.next()) {
        (Some(len), None) if len <= MAX_WORD_LEN => Ok(len),
        (Some(len), None) => Err(WordsError::TooLong { len, max: MAX_WORD_LEN }),
        _ => Err(WordsError::MixedLengths(by_len)),
    }
}

// How a word may be used: an answer can be picked as the secret, a valid word only counts as
// a legal guess. In the encoded stream this is the per-word colour bit.
pub enum WordKind {
    Answer,
    Valid,
}

pub struct WordleWord {
    pub word: LowercaseAsciiWord,
    pub kind: WordKind,
}

// Merge two individually sorted, deduplicated lists into one sorted stream, tagging each word
// as an answer or valid-only. A word present in both lists is emitted once, as an answer, so
// the output stays strictly ascending — exactly the invariant the encoder depends on.
pub struct MergedWordsIter<A, V>
where
    A: Iterator<Item = LowercaseAsciiWord>,
    V: Iterator<Item = LowercaseAsciiWord>,
{
    answers: A,
    valids: V,
    next_answer: Option<LowercaseAsciiWord>,
    next_valid: Option<LowercaseAsciiWord>,
}

impl<A, V> Iterator for MergedWordsIter<A, V>
where
    A: Iterator<Item = LowercaseAsciiWord>,
    V: Iterator<Item = LowercaseAsciiWord>,
{
    type Item = WordleWord;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.next_answer.take(), self.next_valid.take()) {
            (None, None) => None,

            (Some(ans), None) => {
                self.next_answer = self.answers.next();
                Some(WordleWord { word: ans, kind: WordKind::Answer })
            }

            (None, Some(val)) => {
                self.next_valid = self.valids.next();
                Some(WordleWord { word: val, kind: WordKind::Valid })
            }

            (Some(ans), Some(val)) => match ans.cmp(&val) {
                Ordering::Less => {
                    self.next_answer = self.answers.next();
                    self.next_valid = Some(val);
                    Some(WordleWord { word: ans, kind: WordKind::Answer })
                }
                Ordering::Equal => {
                    self.next_answer = self.answers.next();
                    self.next_valid = self.valids.next();
                    Some(WordleWord { word: ans, kind: WordKind::Answer })
                }
                Ordering::Greater => {
                    self.next_answer = Some(ans);
                    self.next_valid = self.valids.next();
                    Some(WordleWord { word: val, kind: WordKind::Valid })
                }
            },
        }
    }
}

pub fn merge_words<A, V>(answer_words: A, valid_words: V) -> MergedWordsIter<A::IntoIter, V::IntoIter>
where
    A: IntoIterator<Item = LowercaseAsciiWord>,
    V: IntoIterator<Item = LowercaseAsciiWord>,
{
    let mut answers = answer_words.into_iter();
    let mut valids = valid_words.into_iter();
    let next_answer = answers.next();
    let next_valid = valids.next();

    MergedWordsIter { answers, valids, next_answer, next_valid }
}
