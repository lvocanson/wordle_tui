// The word database: everything about *which* five-letter strings exist, decoded on demand
// from the compressed corpus (see codec.rs). This layer knows nothing about a game in progress
// — it only answers "is this a word?" (`is_valid`) and "give me an answer to guess" (`pick_target`).

use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codec::{decode_color, decode_word, Model, RangeDecoder};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));

// The merged, sorted corpus of both word lists as one arithmetic-coded stream (see codec.rs).
// It is decoded on demand — every lookup walks the stream from the start, rebuilding the
// adaptive model as it goes. No word is ever held in a table: the trade is CPU for size.
static CORPUS_RAW: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/corpus.bin"));

// One left-to-right pass over the corpus, yielding each `(word, is_answer)` in sorted order.
// Every consumer is just a walk, so the decoder/model/count bookkeeping lives here once; the
// counts feed the colour bit's sampling-without-replacement, `is_answer` being colour A.
struct Corpus {
    dec: RangeDecoder<'static>,
    model: Model,
    prev: Option<[u8; WORD_LEN]>,
    remaining: u32,
    remaining_answers: u32,
    left: usize,
}

impl Corpus {
    fn new() -> Self {
        Corpus {
            dec: RangeDecoder::new(CORPUS_RAW),
            model: Model::new(WORD_LEN, ORDER, INC),
            prev: None,
            remaining: WORD_COUNT as u32,
            remaining_answers: ANSWER_COUNT as u32,
            left: WORD_COUNT,
        }
    }
}

impl Iterator for Corpus {
    type Item = ([u8; WORD_LEN], bool);

    fn next(&mut self) -> Option<Self::Item> {
        self.left = self.left.checked_sub(1)?;
        let mut word = [0u8; WORD_LEN];
        decode_word(&mut self.dec, &mut self.model, self.prev.as_ref().map(|w| &w[..]), &mut word);
        let is_answer = decode_color(&mut self.dec, &mut self.remaining, &mut self.remaining_answers);
        self.prev = Some(word);
        Some((word, is_answer))
    }
}

fn xorshift64(x: u64) -> u64 {
    let x = x ^ (x << 13);
    let x = x ^ (x >> 7);
    x ^ (x << 17)
}

// Membership test over the corpus (colour ignored: every word, answer or valid, counts).
// The corpus is sorted, so the scan stops as soon as it passes where `word` would be.
pub fn is_valid(word: &[u8]) -> bool {
    let target: [u8; WORD_LEN] = std::array::from_fn(|i| word[i] - b'a');
    for (w, _) in Corpus::new() {
        match w.cmp(&target) {
            Ordering::Equal => return true,
            Ordering::Greater => return false,
            Ordering::Less => {}
        }
    }
    false
}

pub fn random_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(12345)
}

// Pick a random answer: the answers are exactly the colour-A words, known to number
// ANSWER_COUNT, so choose an index and walk the stream to the idx-th colour-A word.
pub fn pick_target(seed: u64) -> [u8; WORD_LEN] {
    let idx = xorshift64(seed) as usize % ANSWER_COUNT;
    let mut seen = 0;
    for (w, is_answer) in Corpus::new() {
        if is_answer {
            if seen == idx {
                return std::array::from_fn(|i| b'a' + w[i]);
            }
            seen += 1;
        }
    }
    // Reachable ONLY if `corpus.bin` is corrupt: with intact data the stream holds exactly
    // ANSWER_COUNT colour-A words (guaranteed by the encoder, checked by `corpus_round_trips`),
    // so `seen` hits `idx < ANSWER_COUNT` before the walk ends and we return above. Under
    // immediate-abort this compiles to a bare abort. See OPTIMIZATION.md "immediate-abort safety".
    unreachable!("colour-A words number ANSWER_COUNT, so idx is always reached")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Decode the whole stream once and check the structural invariants the encoder promises:
    // exactly WORD_COUNT words, strictly ascending, and exactly ANSWER_COUNT colour-A words.
    // Any range-coder or model divergence between the two ends would break one of these.
    #[test]
    fn corpus_round_trips() {
        let mut answers = 0;
        let mut prev: Option<[u8; WORD_LEN]> = None;
        for (n, (w, is_answer)) in Corpus::new().enumerate() {
            assert!(w.iter().all(|&c| c < 26), "word {n} has a non-letter symbol");
            if let Some(p) = &prev {
                assert!(w > *p, "word {n} is not strictly after the previous");
            }
            if is_answer {
                answers += 1;
            }
            prev = Some(w);
        }
        assert_eq!(answers, ANSWER_COUNT);
    }

    #[test]
    fn known_words_are_valid_and_junk_is_not() {
        assert!(is_valid(b"crane"));
        assert!(is_valid(b"slate"));
        assert!(!is_valid(b"zzzzz"));
    }

    #[test]
    fn picked_targets_are_valid_answers() {
        for seed in 0..50 {
            let t = pick_target(seed);
            assert!(is_valid(&t), "picked target {:?} is not a valid word", t);
        }
    }
}
