// The encoder side of the codec: turn the tagged word corpus into the compressed blob, and
// search the model space for the setting that compresses it smallest. Everything arithmetic
// is shared with the decoder through the `codec` module, so encode and decode agree bit for
// bit.

use crate::codec;
use crate::codec_enc::{self, common_prefix, RangeEncoder};

// Model-search bounds. Order-3 (27^3 = 19,683 contexts) already exceeds the decoder's stack table
// limit (`codec::MAX_STACK_CTX`), so it can never be baked — cap at 2 and let the per-scheme filter
// in `best_model` drop the rest (e.g. order-2 + position). inc past ~32 leaves smoothing behind.
pub const MAX_ORDER: usize = 2;
pub const MAX_INC: u16 = 32;

// The encoder's adaptive model: the `Vec`-backed counterpart of the decoder's `DecodeModel`
// (src/words.rs). The build searches `order`/`inc`/`word_len`, so its storage must be sized at
// runtime — hence `Vec`s here, fixed arrays there. Both delegate to the same storage-agnostic math
// in codec.rs, so encode and decode agree bit for bit. This side is a build script, not the shipped
// binary, so its allocations cost nothing we measure.
struct Model {
    // Character counts per context, `n_ctx` rows of `[u16; 26]`.
    counts: Vec<[u16; 26]>,
    // Prefix-length counts over 0..word_len.
    pref: Vec<u16>,
    pref_total: u32,
    order: usize,
    use_pos: bool,
    word_len: usize,
    inc: u16,
}

impl Model {
    fn new(word_len: usize, order: usize, use_pos: bool, inc: u16) -> Self {
        Model {
            counts: vec![[0u16; 26]; codec::n_ctx(order, use_pos, word_len)],
            pref: vec![0u16; word_len],
            pref_total: 0,
            order,
            use_pos,
            word_len,
            inc,
        }
    }

    fn ctx(&self, word: &[u8], i: usize) -> usize {
        codec::ctx(self.order, self.use_pos, self.word_len, word, i)
    }

    fn char_freq(&self, ctx: usize, sym: usize) -> u32 {
        codec_enc::char_freq(&self.counts[ctx], sym)
    }

    fn char_tot(&self, ctx: usize, lo: usize, hi: usize) -> u32 {
        codec::char_tot(&self.counts[ctx], lo, hi)
    }

    fn char_cum(&self, ctx: usize, lo: usize, sym: usize) -> u32 {
        codec_enc::char_cum(&self.counts[ctx], lo, sym)
    }

    fn update(&mut self, ctx: usize, sym: usize) {
        codec::count_update(&mut self.counts[ctx], self.inc, sym);
    }

    fn pref_freq(&self, p: usize) -> u32 {
        codec_enc::pref_freq(&self.pref, p)
    }

    fn pref_tot(&self) -> u32 {
        codec::pref_tot(&self.pref, self.pref_total)
    }

    fn pref_cum(&self, p: usize) -> u32 {
        codec_enc::pref_cum(&self.pref, p)
    }

    fn pref_update(&mut self, p: usize) {
        codec::pref_update(&mut self.pref, &mut self.pref_total, self.inc, p);
    }
}

// A word paired with its colour bit: true = answer, false = valid-only. Held as 0..25 symbols.
pub type TaggedWord = (Vec<u8>, bool);

// Encode one word given the previous word, mirroring decode_word in src/words.rs. `descending` is
// the stored sort direction: it flips which side of the previous word bounds the first differing
// character (the sort proves w[p] > prev[p] ascending / w[p] < prev[p] descending).
fn encode_word(
    enc: &mut RangeEncoder,
    m: &mut Model,
    prev: Option<&[u8]>,
    w: &[u8],
    word_len: usize,
    descending: bool,
) {
    let p = match prev {
        Some(pv) => common_prefix(pv, w),
        None => 0,
    };
    enc.encode(m.pref_cum(p), m.pref_freq(p), m.pref_tot());
    m.pref_update(p);

    for i in p..word_len {
        let ctx = m.ctx(w, i);
        // The first differing character is restricted to one side of prev[p]; all others span 0..26.
        let (lo, hi) = match prev {
            Some(pv) if i == p => {
                let f = pv[p] as usize;
                if descending {
                    (0, f)
                } else {
                    (f + 1, 26)
                }
            }
            _ => (0, 26),
        };
        let sym = w[i] as usize;
        enc.encode(
            m.char_cum(ctx, lo, sym),
            m.char_freq(ctx, sym),
            m.char_tot(ctx, lo, hi),
        );
        m.update(ctx, sym);
    }
}

// Re-order the distinct tagged corpus into the stored form for one ordering scheme: words kept
// forward or reversed (share prefixes vs. suffixes), sorted ascending or descending.
fn ordered(corpus: &[TaggedWord], reverse_word: bool, descending: bool) -> Vec<TaggedWord> {
    let mut v: Vec<TaggedWord> = corpus
        .iter()
        .map(|(w, a)| {
            let mut k = w.clone();
            if reverse_word {
                k.reverse();
            }
            (k, *a)
        })
        .collect();
    v.sort_by(|x, y| {
        if descending {
            y.0.cmp(&x.0)
        } else {
            x.0.cmp(&y.0)
        }
    });
    v
}

// Encode one already-ordered corpus under one full scheme and return the blob. The colour bit after
// each word is coded by an adaptive binary model conditioned on the letter at `color_pos` (or a
// single shared model when `!use_color`), mirroring decode_color/Corpus in src/words.rs.
fn encode_corpus(corpus: &[TaggedWord], word_len: usize, s: Scheme) -> Vec<u8> {
    let mut enc = RangeEncoder::new();
    let mut model = Model::new(word_len, s.order, s.use_pos, s.inc);
    let mut color = vec![[0u32; 2]; codec::color_nctx(s.use_color)];
    let mut prev: Option<&[u8]> = None;
    for (w, is_answer) in corpus {
        encode_word(&mut enc, &mut model, prev, w, word_len, s.descending);
        let cc = codec::color_ctx(w, s.use_color, s.color_pos);
        let (f0, f1) = (color[cc][0] + 1, color[cc][1] + 1);
        if *is_answer {
            enc.encode(f0, f1, f0 + f1);
            color[cc][1] += 1;
        } else {
            enc.encode(0, f0, f0 + f1);
            color[cc][0] += 1;
        }
        prev = Some(w);
    }
    enc.finish()
}

// One point in the model space: every knob the search tries, each of which the decoder bakes as a
// constant. Grouped so the search passes it as one value — five consecutive bools and usizes as
// positional arguments is an inversion waiting to happen, and an inverted knob still compiles.
#[derive(Clone, Copy)]
pub struct Scheme {
    pub order: usize,
    pub use_pos: bool,
    pub inc: u16,
    pub reverse_word: bool,
    pub descending: bool,
    pub use_color: bool,
    pub color_pos: usize,
}

// The winning scheme: which knobs compressed this corpus smallest, plus its blob.
pub struct Best {
    pub scheme: Scheme,
    pub blob: Vec<u8>,
}

// Search the whole model space and keep whatever compresses the corpus smallest. Every knob is
// baked into the decoder as a constant, so all tuning cost stays here at build time and the runtime
// just carries the answer. Order is capped where more context only makes the table sparser than the
// data can fill; `use_pos` folds the word position into the context (a strong predictor for
// fixed-length words); inc sweeps the smoothing balance; `reverse_word`/`descending` pick the stored
// order (share prefixes vs. suffixes, ascending vs. descending); `color_pos` conditions the colour
// bit on one letter (answers avoid some endings) — the data decides every one, never assumed.
pub fn best_model(corpus: &[TaggedWord], word_len: usize) -> Best {
    // Colour context candidates: a single shared model, or condition on each letter position.
    let color_schemes = std::iter::once((false, 0)).chain((0..word_len).map(|p| (true, p)));
    let mut best: Option<Best> = None;
    for reverse_word in [false, true] {
        for descending in [false, true] {
            let ord = ordered(corpus, reverse_word, descending);
            for use_pos in [false, true] {
                for order in 1..=MAX_ORDER {
                    // Skip any scheme the decoder could not hold on the stack (see MAX_STACK_CTX).
                    if codec::n_ctx(order, use_pos, word_len) > codec::MAX_STACK_CTX {
                        continue;
                    }
                    for inc in 1..=MAX_INC {
                        for (use_color, color_pos) in color_schemes.clone() {
                            let scheme = Scheme {
                                order,
                                use_pos,
                                inc,
                                reverse_word,
                                descending,
                                use_color,
                                color_pos,
                            };
                            let blob = encode_corpus(&ord, word_len, scheme);
                            if best.as_ref().is_none_or(|b| blob.len() < b.blob.len()) {
                                best = Some(Best { scheme, blob });
                            }
                        }
                    }
                }
            }
        }
    }
    best.expect("model space is non-empty")
}
