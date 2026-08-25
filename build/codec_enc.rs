// The encoder-only half of the codec: the range encoder and the model's forward frequency lookups.
// It lives here rather than in `src/codec.rs` because nothing in it ever reaches the binary — the
// crate would compile it only to have LTO drop it again, and every item would need a `dead_code`
// exemption to say so.
//
// What stays in `src/codec.rs` is what has to agree bit for bit between the two ends: the context
// derivation, the count updates, and the running totals. Each function below is the forward
// direction of an inverse pair whose other half is a decoder-only lookup in `src/codec.rs`
// (`char_cum` ↔ `char_find`, `pref_cum` ↔ `pref_find`); the pairing is what `corpus_round_trips`
// exercises, and changing one side without the other corrupts the stream silently.

use crate::codec::TOP;

// --- Range coder, encoder side (mirror of codec::RangeDecoder) -------------------------

pub struct RangeEncoder {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    pub out: Vec<u8>,
}

impl RangeEncoder {
    pub fn new() -> Self {
        // cache_size starts at 1 so the first shift emits one leading byte the decoder skips.
        RangeEncoder {
            low: 0,
            range: 0xFFFF_FFFF,
            cache: 0,
            cache_size: 1,
            out: Vec::new(),
        }
    }

    fn shift_low(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut byte = self.cache;
            loop {
                self.out.push(byte.wrapping_add(carry));
                byte = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.cache_size += 1;
        self.low = (self.low << 8) & 0xFFFF_FFFF;
    }

    pub fn encode(&mut self, cum: u32, freq: u32, tot: u32) {
        self.range /= tot;
        self.low += cum as u64 * self.range as u64;
        self.range = self.range.wrapping_mul(freq);
        while self.range < TOP {
            self.range <<= 8;
            self.shift_low();
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        self.out
    }
}

// --- Front-coding ----------------------------------------------------------------------

// How many leading bytes two consecutive words share. The decoder never needs this: the shared
// length is what the stream carries, not something it recomputes.
pub fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    i
}

// --- Model, forward direction (add-one smoothing, as in codec::char_tot) ----------------

pub fn char_freq(row: &[u16; 26], sym: usize) -> u32 {
    row[sym] as u32 + 1
}

// Inverse of codec::char_find.
pub fn char_cum(row: &[u16; 26], lo: usize, sym: usize) -> u32 {
    row[lo..sym].iter().map(|&c| c as u32 + 1).sum()
}

pub fn pref_freq(pref: &[u16], p: usize) -> u32 {
    pref[p] as u32 + 1
}

// Inverse of codec::pref_find.
pub fn pref_cum(pref: &[u16], p: usize) -> u32 {
    pref[..p].iter().map(|&c| c as u32 + 1).sum()
}
