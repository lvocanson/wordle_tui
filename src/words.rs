// The word database: everything about *which* five-letter strings exist, decoded on demand
// from the compressed corpus (see codec.rs). This layer knows nothing about a game in progress
// — it only answers "is this a word?" (`is_valid`) and "give me an answer to guess" (`pick_target`).

use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codec::{self, decode_color, RangeDecoder};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));

// The merged, sorted corpus of both word lists as one arithmetic-coded stream (see codec.rs).
// It is decoded on demand — every lookup walks the stream from the start, rebuilding the
// adaptive model as it goes. No word is ever held in a table: the trade is CPU for size.
static CORPUS_RAW: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/corpus.bin"));

// Number of contexts for the chosen scheme (`27^ORDER`, times `WORD_LEN` when `USE_POS`). On the
// decoder these are compile-time constants, so the count table is a fixed array rather than the
// encoder's `Vec` — no heap allocation, no `from_elem`/drop glue in the binary (this is the whole
// reason the decoder's model storage is split from the encoder's; the shared probability math lives
// in codec.rs). Guard the stack cost: the winning scheme here is pos+order1 = 27·5 = 135 contexts,
// a 135·26·2 = 7020 B table, fine per-lookup on the stack; a much larger scheme (e.g. order-3, ~1 MB)
// would blow the stack, so refuse to compile it silently — box the counts there instead.
const DEC_CTX: usize = codec::n_ctx(ORDER, USE_POS, WORD_LEN);
const _: () = assert!(
    DEC_CTX <= codec::MAX_STACK_CTX,
    "decoder count table too large for a stack array; box DecodeModel.counts for this scheme",
);

// The decoder's adaptive model: fixed-size counterpart of the encoder's `Model` (build/encode.rs),
// sized entirely by the `ORDER`/`WORD_LEN` constants. Both feed the same storage-agnostic math in
// codec.rs, so they stay bit-exact by construction.
struct DecodeModel {
    counts: [[u16; 26]; DEC_CTX],
    pref: [u16; WORD_LEN],
    pref_total: u32,
    // Adaptive colour model: [not-answer, answer] counts per context (one letter, or a single
    // shared model). See codec::decode_color. The table is on the stack, so its size is not binary.
    color: [[u32; 2]; codec::color_nctx(USE_COLOR)],
}

impl DecodeModel {
    fn new() -> Self {
        DecodeModel {
            counts: [[0u16; 26]; DEC_CTX],
            pref: [0u16; WORD_LEN],
            pref_total: 0,
            color: [[0u32; 2]; codec::color_nctx(USE_COLOR)],
        }
    }
}

// Decode one word (values 0..25) given the previous word, mirroring encode_word in build/encode.rs.
// The word is written into `out`; `out.len()` is the word length. No heap word buffer: the caller
// owns a fixed `[u8; WORD_LEN]`, so the whole per-word Vec/clone/grow machinery stays out of the
// decoder (the corpus is walked word by word thousands of times per lookup).
fn decode_word(dec: &mut RangeDecoder, m: &mut DecodeModel, prev: Option<&[u8]>, out: &mut [u8]) {
    let dv = dec.decode_freq(codec::pref_tot(&m.pref, m.pref_total));
    let (p, cum, f) = codec::pref_find(&m.pref, dv);
    dec.decode_update(cum, f);
    codec::pref_update(&mut m.pref, &mut m.pref_total, INC, p);

    let floor: Option<usize> = match prev {
        Some(pv) => {
            out[..p].copy_from_slice(&pv[..p]);
            Some(pv[p] as usize)
        }
        None => None,
    };

    for i in p..out.len() {
        let ctx = codec::ctx(ORDER, USE_POS, WORD_LEN, out, i);
        // The first differing character is bounded to one side of prev[p] by the stored sort
        // direction (`DESCENDING` const-folds this to the ascending `[floor+1, 26)` path here).
        let (lo, hi) = match floor {
            Some(f) if i == p => {
                if DESCENDING {
                    (0, f)
                } else {
                    (f + 1, 26)
                }
            }
            _ => (0, 26),
        };
        let dv = dec.decode_freq(codec::char_tot(&m.counts[ctx], lo, hi));
        let (sym, cum, f) = codec::char_find(&m.counts[ctx], lo, hi, dv);
        dec.decode_update(cum, f);
        out[i] = sym as u8;
        codec::count_update(&mut m.counts[ctx], INC, sym);
    }
}

// One left-to-right pass over the corpus, yielding each `(word, is_answer)` in sorted order.
// Every consumer is just a walk, so the decoder/model/count bookkeeping lives here once; the
// adaptive colour model in `model` decodes `is_answer` after each word.
struct Corpus {
    dec: RangeDecoder<'static>,
    model: DecodeModel,
    prev: Option<[u8; WORD_LEN]>,
    left: usize,
}

impl Corpus {
    fn new() -> Self {
        Corpus {
            dec: RangeDecoder::new(CORPUS_RAW),
            model: DecodeModel::new(),
            prev: None,
            left: WORD_COUNT,
        }
    }
}

impl Iterator for Corpus {
    type Item = ([u8; WORD_LEN], bool);

    fn next(&mut self) -> Option<Self::Item> {
        self.left = self.left.checked_sub(1)?;
        let mut word = [0u8; WORD_LEN];
        decode_word(
            &mut self.dec,
            &mut self.model,
            self.prev.as_ref().map(|w| &w[..]),
            &mut word,
        );
        let cc = codec::color_ctx(&word, USE_COLOR, COLOR_POS);
        let is_answer = decode_color(&mut self.dec, &mut self.model.color[cc]);
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
    // The stream stores keys (words reversed when `REVERSE_WORD`), sorted in the `DESCENDING`
    // direction; compare in that same key space and stop once the scan passes where the key sits.
    let target: [u8; WORD_LEN] = std::array::from_fn(|i| {
        let src = if REVERSE_WORD { WORD_LEN - 1 - i } else { i };
        word[src] - b'a'
    });
    for (w, _) in Corpus::new() {
        match w.cmp(&target) {
            Ordering::Equal => return true,
            Ordering::Greater if !DESCENDING => return false,
            Ordering::Less if DESCENDING => return false,
            _ => {}
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
                // `w` is the stored key; undo the `REVERSE_WORD` transform to recover the word.
                return std::array::from_fn(|i| {
                    let src = if REVERSE_WORD { WORD_LEN - 1 - i } else { i };
                    b'a' + w[src]
                });
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
            assert!(
                w.iter().all(|&c| c < 26),
                "word {n} has a non-letter symbol"
            );
            if let Some(p) = &prev {
                let ordered = if DESCENDING { w < *p } else { w > *p };
                assert!(
                    ordered,
                    "word {n} is not strictly ordered relative to the previous"
                );
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
