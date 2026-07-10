// The encoder side of the codec: turn the tagged word union into the compressed blob, and
// search the model space for the setting that compresses it smallest. Everything arithmetic
// is shared with the decoder through the `codec` module, so encode and decode agree bit for
// bit.

use crate::codec::{common_prefix, Model, RangeEncoder};

// Model-search bounds. Order beyond ~3 only makes the 27^order context table sparser than a
// list of a few thousand words can populate; inc past ~32 leaves smoothing behind entirely.
pub const MAX_ORDER: usize = 3;
pub const MAX_INC: u16 = 32;

// A word paired with its colour bit: true = answer, false = valid-only. Held as 0..25 symbols.
pub type TaggedWord = (Vec<u8>, bool);

// Encode one word given the previous word, mirroring decode_word in the codec.
fn encode_word(enc: &mut RangeEncoder, m: &mut Model, prev: Option<&[u8]>, w: &[u8], word_len: usize) {
    let p = match prev {
        Some(pv) => common_prefix(pv, w),
        None => 0,
    };
    enc.encode(m.pref_cum(p), m.pref_freq(p), m.pref_tot());
    m.pref_update(p);

    // The sort proves w[p] > prev[p], so the first differing character is coded over the
    // restricted range (prev[p], 'z']; every later character is unrestricted.
    let floor: i32 = match prev {
        Some(pv) => pv[p] as i32,
        None => -1,
    };
    for i in p..word_len {
        let ctx = m.ctx(w, i);
        let lo = if i == p { (floor + 1) as usize } else { 0 };
        let sym = w[i] as usize;
        enc.encode(m.char_cum(ctx, lo, sym), m.char_freq(ctx, sym), m.char_tot(ctx, lo));
        m.update(ctx, sym);
    }
}

// Encode the whole union under one (order, inc) setting and return the resulting blob.
fn encode_union(union: &[TaggedWord], answers: u32, word_len: usize, order: usize, inc: u16) -> Vec<u8> {
    let mut enc = RangeEncoder::new();
    let mut model = Model::new(word_len, order, inc);
    let mut prev: Option<&[u8]> = None;
    let (mut remaining, mut remaining_a) = (union.len() as u32, answers);
    for (w, is_answer) in union {
        encode_word(&mut enc, &mut model, prev, w, word_len);
        if *is_answer {
            enc.encode(0, remaining_a, remaining);
            remaining_a -= 1;
        } else {
            enc.encode(remaining_a, remaining - remaining_a, remaining);
        }
        remaining -= 1;
        prev = Some(w);
    }
    enc.finish()
}

// Search the whole model space and keep whatever compresses the union smallest. The winning
// (order, inc) are baked into the decoder as constants, so all tuning cost stays here at build
// time and the runtime just carries the answer. Order is capped where more context only makes
// the table sparser than the data can fill; inc sweeps the smoothing balance.
pub fn best_model(union: &[TaggedWord], answers: u32, word_len: usize) -> (usize, u16, Vec<u8>) {
    let mut best_order = 1;
    let mut best_inc = 1;
    let mut best_blob = encode_union(union, answers, word_len, best_order, best_inc);
    for order in 1..=MAX_ORDER {
        for inc in 1..=MAX_INC {
            let blob = encode_union(union, answers, word_len, order, inc);
            if blob.len() < best_blob.len() {
                (best_order, best_inc, best_blob) = (order, inc, blob);
            }
        }
    }
    (best_order, best_inc, best_blob)
}
