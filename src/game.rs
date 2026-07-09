use std::time::{SystemTime, UNIX_EPOCH};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));
pub const MAX_GUESSES: usize = 6;

// Word lists as LEB128 gap streams (see build.rs): a sorted list of base-26 word codes
// stored as deltas. Scanned in a single streaming pass — no decode buffer, no allocation.
static ANSWERS_RAW: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/answers.bin"));
static VALID_RAW: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/valid.bin"));

// Read one LEB128 value starting at `data[*i]`, advancing `*i`.
fn read_varint(data: &[u8], i: &mut usize) -> u32 {
    let (mut v, mut shift) = (0u32, 0);
    loop {
        let b = data[*i];
        *i += 1;
        v |= ((b & 0x7f) as u32) << shift;
        if b & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

// Is `target` present in the sorted delta stream? Stops early once codes exceed it.
fn stream_contains(data: &[u8], target: u32) -> bool {
    let mut acc = 0u32;
    let mut i = 0;
    while i < data.len() {
        acc = acc.wrapping_add(read_varint(data, &mut i));
        if acc == target {
            return true;
        }
        if acc > target {
            return false;
        }
    }
    false
}

// Encode a 5-letter lowercase word into its base-26 code.
fn pack(word: &[u8]) -> u32 {
    let mut v: u32 = 0;
    for &c in word {
        v = v * 26 + (c - b'a') as u32;
    }
    v
}

// Decode a base-26 code back into a 5-letter lowercase word.
fn unpack(mut v: u32) -> [u8; WORD_LEN] {
    let mut out = [0u8; WORD_LEN];
    let mut i = WORD_LEN;
    while i > 0 {
        i -= 1;
        out[i] = b'a' + (v % 26) as u8;
        v /= 26;
    }
    out
}

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

pub fn is_valid(word: &[u8]) -> bool {
    let rec = pack(word);
    stream_contains(VALID_RAW, rec) || stream_contains(ANSWERS_RAW, rec)
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

pub fn pick_target(seed: u64) -> [u8; WORD_LEN] {
    // Count answers, pick an index, then walk to it (streaming, no buffer).
    let mut count = 0usize;
    let mut i = 0;
    while i < ANSWERS_RAW.len() {
        read_varint(ANSWERS_RAW, &mut i);
        count += 1;
    }
    let idx = xorshift64(seed) as usize % count;
    let (mut acc, mut i) = (0u32, 0);
    for _ in 0..=idx {
        acc = acc.wrapping_add(read_varint(ANSWERS_RAW, &mut i));
    }
    unpack(acc)
}
