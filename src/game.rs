use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codec::{decode_word, Model, RangeDecoder};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));
pub const MAX_GUESSES: usize = 6;

// The merged, sorted union of both word lists as one arithmetic-coded stream (see codec.rs).
// It is decoded on demand — every lookup walks the stream from the start, rebuilding the
// adaptive model as it goes. No word is ever held in a table: the trade is CPU for size.
static UNION_RAW: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/union.bin"));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LetterState {
    Correct,
    Misplaced,
    Absent,
}

// Decode the colour bit that follows each word: true = answer (colour A). Sampling without
// replacement, so the probability tracks the counts still to be placed.
fn decode_color(dec: &mut RangeDecoder, remaining: &mut u32, remaining_a: &mut u32) -> bool {
    let is_a = dec.decode_freq(*remaining) < *remaining_a;
    if is_a {
        dec.decode_update(0, *remaining_a);
        *remaining_a -= 1;
    } else {
        dec.decode_update(*remaining_a, *remaining - *remaining_a);
    }
    *remaining -= 1;
    is_a
}

fn xorshift64(x: u64) -> u64 {
    let x = x ^ (x << 13);
    let x = x ^ (x >> 7);
    x ^ (x << 17)
}

// Membership test over the union (colour ignored: every word, answer or valid, counts).
// The union is sorted, so the scan stops as soon as it passes where `word` would be.
pub fn is_valid(word: &[u8]) -> bool {
    let target: Vec<u8> = word.iter().map(|b| b - b'a').collect();
    let mut dec = RangeDecoder::new(UNION_RAW);
    let mut model = Model::new(WORD_LEN, ORDER, INC);
    let mut prev: Option<Vec<u8>> = None;
    let (mut remaining, mut remaining_a) = (WORD_COUNT as u32, ANSWER_COUNT as u32);
    for _ in 0..WORD_COUNT {
        let w = decode_word(&mut dec, &mut model, prev.as_deref(), WORD_LEN);
        decode_color(&mut dec, &mut remaining, &mut remaining_a);
        match w.cmp(&target) {
            Ordering::Equal => return true,
            Ordering::Greater => return false,
            Ordering::Less => prev = Some(w),
        }
    }
    false
}

pub fn check(target: &[u8], guess: &[u8]) -> [LetterState; WORD_LEN] {
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
    let mut dec = RangeDecoder::new(UNION_RAW);
    let mut model = Model::new(WORD_LEN, ORDER, INC);
    let mut prev: Option<Vec<u8>> = None;
    let (mut remaining, mut remaining_a) = (WORD_COUNT as u32, ANSWER_COUNT as u32);
    let mut seen = 0;
    for _ in 0..WORD_COUNT {
        let w = decode_word(&mut dec, &mut model, prev.as_deref(), WORD_LEN);
        if decode_color(&mut dec, &mut remaining, &mut remaining_a) {
            if seen == idx {
                return std::array::from_fn(|i| b'a' + w[i]);
            }
            seen += 1;
        }
        prev = Some(w);
    }
    unreachable!("colour-A words number ANSWER_COUNT, so idx is always reached")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Decode the whole stream once and check the structural invariants the encoder promises:
    // exactly WORD_COUNT words, strictly ascending, and exactly ANSWER_COUNT colour-A words.
    // Any range-coder or model divergence between the two ends would break one of these.
    #[test]
    fn union_round_trips() {
        let mut dec = RangeDecoder::new(UNION_RAW);
        let mut model = Model::new(WORD_LEN, ORDER, INC);
        let mut prev: Option<Vec<u8>> = None;
        let (mut remaining, mut remaining_a) = (WORD_COUNT as u32, ANSWER_COUNT as u32);
        let mut answers = 0;
        for n in 0..WORD_COUNT {
            let w = decode_word(&mut dec, &mut model, prev.as_deref(), WORD_LEN);
            assert_eq!(w.len(), WORD_LEN);
            assert!(w.iter().all(|&c| c < 26), "word {n} has a non-letter symbol");
            if let Some(p) = &prev {
                assert!(w.as_slice() > p.as_slice(), "word {n} is not strictly after the previous");
            }
            if decode_color(&mut dec, &mut remaining, &mut remaining_a) {
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
