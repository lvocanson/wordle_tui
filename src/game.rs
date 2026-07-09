use std::time::{SystemTime, UNIX_EPOCH};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));
pub const MAX_GUESSES: usize = 6;

// Words packed back-to-back (word n = &DATA[n*WORD_LEN..(n+1)*WORD_LEN]), sorted for binary search.
pub static ANSWERS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/answers.bin"));
pub static VALID: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/valid.bin"));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LetterState {
    CorrectSpot,
    WrongSpot,
    NotInAnySpot,
}

fn xorshift64(x: u64) -> u64 {
    let x = x ^ (x << 13);
    let x = x ^ (x >> 7);
    x ^ (x << 17)
}

fn bsearch(dict: &[u8], word: &[u8]) -> bool {
    let mut lo = 0;
    let mut hi = dict.len() / WORD_LEN;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        match dict[mid * WORD_LEN..(mid + 1) * WORD_LEN].cmp(word) {
            std::cmp::Ordering::Equal => return true,
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
        }
    }
    false
}

pub fn is_valid(word: &[u8]) -> bool {
    bsearch(VALID, word) || bsearch(ANSWERS, word)
}

pub fn check(target: &[u8], guess: &[u8]) -> [LetterState; WORD_LEN] {
    let mut res = [LetterState::NotInAnySpot; WORD_LEN];
    let mut used = [false; WORD_LEN];
    for i in 0..WORD_LEN {
        if guess[i] == target[i] {
            res[i] = LetterState::CorrectSpot;
            used[i] = true;
        }
    }
    for i in 0..WORD_LEN {
        if res[i] == LetterState::CorrectSpot {
            continue;
        }
        for j in 0..WORD_LEN {
            if !used[j] && guess[i] == target[j] {
                res[i] = LetterState::WrongSpot;
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

pub fn pick_target(seed: u64) -> &'static [u8] {
    let count = ANSWERS.len() / WORD_LEN;
    let idx = xorshift64(seed) as usize % count;
    &ANSWERS[idx * WORD_LEN..(idx + 1) * WORD_LEN]
}
