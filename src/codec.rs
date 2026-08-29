// Arithmetic-coding codec, decoder side plus everything both ends share: the crate compiles this
// file as a module for the decoder, and the build script pulls the same file in for the encoder
// (`#[path = "../src/codec.rs"] mod codec`). What is shared is the *arithmetic* — the context
// derivation, the count updates and the running totals — because that is what has to agree bit for
// bit; any divergence would silently corrupt the stream. The two ends differ only in *storage*: the
// encoder searches `order`/`inc`/`word_len` and so holds its model counts in `Vec`s
// (`build/encode.rs`), while the decoder knows them as compile-time constants and holds them in
// fixed arrays (`words.rs`). Both wrap the same storage-agnostic functions below, so there is one
// source of truth for the model and no way for the two sides to drift.
//
// The two word lists are merged into one sorted, distinct corpus (answers + valid extensions)
// and encoded in a single pass. Three sources of redundancy are each targeted directly:
//   * the sort makes the ordering free, and lets consecutive words share a prefix
//     (front-coding: we only encode the suffix that differs);
//   * the first differing character is bounded by the previous word (the sort proves it strictly
//     greater ascending / strictly smaller descending), so its distribution is restricted to one
//     side; the sort direction and whether words are stored reversed are build-searched constants;
//   * remaining characters are predicted by an adaptive order-`order` model over the alphabet,
//     optionally conditioned on the position in the word, which captures most of the linguistic
//     redundancy. The context scheme (`order`, `use_pos`) is chosen by the build's model search.
// A per-word colour bit (answer vs. valid-only) is coded by an adaptive binary model conditioned
// on one build-searched letter of the word (answers avoid certain endings, e.g. plurals in -s).

// --- Range coder (LZMA-style, 32-bit range, byte carry via a cache) --------------------

// Renormalization threshold, read by both coders (`build/codec_enc.rs` holds the encoder).
pub const TOP: u32 = 1 << 24;

// Decoder side. Used by game.rs; harmlessly dead in build.rs.
pub struct RangeDecoder<'a> {
    range: u32,
    code: u32,
    data: &'a [u8],
    pos: usize,
}

impl<'a> RangeDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let mut d = RangeDecoder {
            range: 0xFFFF_FFFF,
            code: 0,
            data,
            pos: 1,
        }; // skip leading byte
        for _ in 0..4 {
            let b = d.next();
            d.code = (d.code << 8) | b;
        }
        d
    }

    fn next(&mut self) -> u32 {
        let b = if self.pos < self.data.len() {
            self.data[self.pos] as u32
        } else {
            0
        };
        self.pos += 1;
        b
    }

    // Returns a cumulative-frequency point in 0..tot to be looked up in the model.
    // Both divisions below panic ONLY on a corrupt `corpus.bin`: `tot` is always > 0 with intact
    // data (every caller feeds a total whose add-one smoothing floors it at >= 1; `char_tot`'s
    // `lo` can reach 26 — an empty, zero total — only if a word's first differing char exceeds
    // 'z', which the sort forbids), and `range >= TOP` is held by the renorm loops while every
    // `tot <= WORD_COUNT < TOP`, so `range / tot >= 1`. See OPTIMIZATION.md "immediate-abort safety".
    pub fn decode_freq(&mut self, tot: u32) -> u32 {
        self.range /= tot;
        let dv = self.code / self.range;
        if dv >= tot {
            tot - 1
        } else {
            dv
        }
    }

    pub fn decode_update(&mut self, cum: u32, freq: u32) {
        self.code -= cum.wrapping_mul(self.range);
        self.range = self.range.wrapping_mul(freq);
        while self.range < TOP {
            let b = self.next();
            self.code = (self.code << 8) | b;
            self.range <<= 8;
        }
    }
}

// --- Adaptive model (storage-agnostic math) --------------------------------------------
//
// Each context character is 0..26, where 26 marks "before the word start". The two model
// knobs — the context length (`order`) and the count increment (`inc`, which sets how fast the
// model trusts what it has seen versus add-one smoothing) — are not fixed here: build.rs searches
// for the pair that compresses this particular data smallest and bakes them in as constants
// (`ORDER`/`INC`). So the runtime carries no tuning logic, only the winning numbers.
//
// The functions below take the count storage as plain slices — a single context's `&[u16; 26]`
// row, or the whole `&[u16]` prefix table — so the exact same math backs the encoder's `Vec`
// storage and the decoder's fixed arrays. `inc`/`order` are passed in rather than read from a
// struct, which lets the decoder pass its constants (and the compiler const-fold them).

pub const CTX_SYMS: usize = 27;
// A context is halved once its running total reaches LIMIT, keeping totals (plus the +26 from
// add-one smoothing) well under the coder's frequency ceiling regardless of `inc`.
const LIMIT: u32 = 1 << 13;

// Fold the `order` characters preceding position `i` — and, when `use_pos`, the position `i`
// itself — into a context index. Position is a strong predictor in fixed-length words (letter
// distributions differ sharply by slot), so `use_pos` is one of the knobs build.rs searches and
// bakes into constants.rs; when set it multiplies the context count by `word_len`.
pub fn ctx(order: usize, use_pos: bool, word_len: usize, word: &[u8], i: usize) -> usize {
    let mut idx = 0;
    for k in (1..=order).rev() {
        let c = if i >= k { word[i - k] as usize } else { 26 };
        idx = idx * CTX_SYMS + c;
    }
    if use_pos {
        idx = idx * word_len + i;
    }
    idx
}

// Number of contexts for a given scheme: `27^order`, times `word_len` when position is folded in.
pub const fn n_ctx(order: usize, use_pos: bool, word_len: usize) -> usize {
    CTX_SYMS.pow(order as u32) * if use_pos { word_len } else { 1 }
}

// The most contexts the decoder holds on the stack (`DecodeModel.counts` is `[[u16; 26]; n_ctx]`,
// zeroed per lookup). The build's model search must not exceed this or it would bake a scheme the
// decoder refuses to compile; both the search filter and the decoder's `const` assert use it. At
// 729 the table is 729·26·2 = 37,908 B, fine on the stack; a larger scheme should box the counts.
pub const MAX_STACK_CTX: usize = CTX_SYMS * CTX_SYMS;

// A context's running total is never stored: it is exactly the sum of its counts (the invariant
// holds through every update and halving), so it is recomputed on demand. Frequencies are taken
// over symbols `lo..hi`. For most positions `[lo, hi) = [0, 26)`; at the first differing character
// the sort bounds it to one side of the previous word's char — `[floor+1, 26)` ascending, `[0, floor)`
// descending (the sort direction is a build-searched constant). `hi` folds to 26 on the ascending
// path, leaving that hot path identical. Effective frequency is `count + 1` (add-one smoothing).
#[inline]
#[allow(clippy::needless_range_loop)] // Measured: the slice + `sum()` form costs 64 B.
pub fn char_tot(row: &[u16; 26], lo: usize, hi: usize) -> u32 {
    let mut t = 0;
    for s in lo..hi {
        t += row[s] as u32 + 1;
    }
    t
}

// Inverse of the encoder's `char_cum` (build/codec_enc.rs): map a decoded frequency point in
// `[lo, hi)` back to (symbol, cum, freq).
#[inline]
#[allow(clippy::needless_range_loop)] // Measured: `enumerate()` over `row[lo..hi]` costs 80 B.
pub fn char_find(row: &[u16; 26], lo: usize, hi: usize, dv: u32) -> (usize, u32, u32) {
    let mut acc = 0;
    for s in lo..hi {
        let f = row[s] as u32 + 1;
        if acc + f > dv {
            return (s, acc, f);
        }
        acc += f;
    }
    let f = row[hi - 1] as u32 + 1;
    (hi - 1, acc - f, f)
}

pub fn count_update(row: &mut [u16; 26], inc: u16, sym: usize) {
    row[sym] += inc;
    let t: u32 = row.iter().map(|&c| c as u32).sum();
    if t >= LIMIT {
        for c in row.iter_mut() {
            *c >>= 1;
        }
    }
}

// Prefix-length model over 0..pref.len(), same add-one smoothing. Its running total *is* cached
// (`total`) rather than recomputed: `pref` is summed in two hot spots, so caching a u32 there is a
// net win (see OPTIMIZATION.md).
pub fn pref_tot(pref: &[u16], total: u32) -> u32 {
    total + pref.len() as u32
}

// Inverse of the encoder's `pref_cum` (build/codec_enc.rs).
pub fn pref_find(pref: &[u16], dv: u32) -> (usize, u32, u32) {
    let mut acc = 0;
    for (s, &c) in pref.iter().enumerate() {
        let f = c as u32 + 1;
        if acc + f > dv {
            return (s, acc, f);
        }
        acc += f;
    }
    let last = pref.len() - 1;
    let f = pref[last] as u32 + 1;
    (last, acc - f, f)
}

pub fn pref_update(pref: &mut [u16], total: &mut u32, inc: u16, p: usize) {
    pref[p] += inc;
    *total += inc as u32;
    if *total >= LIMIT {
        let mut t = 0;
        for c in pref.iter_mut() {
            *c >>= 1;
            t += *c as u32;
        }
        *total = t;
    }
}

// Decode the colour bit that follows each word: true = answer (colour A). A small adaptive binary
// model conditioned on one letter of the word (see `color_ctx`): answers avoid certain endings
// (plurals in -s), a structural signal that exact sampling-without-replacement is blind to.
// `row` is [not-answer, answer] counts for the context; add-one smoothed, incremented by one, never
// halved (u32 counts cannot overflow for any word list the sort admits). `f0 + f1 >= 2 > 0`, so
// decode_freq never divides by zero here. Conditioning saves far more than the ~1 B of coding slack
// it gives up over exact SWOR.
pub fn decode_color(dec: &mut RangeDecoder, row: &mut [u32; 2]) -> bool {
    let (f0, f1) = (row[0] + 1, row[1] + 1);
    let is_a = dec.decode_freq(f0 + f1) >= f0;
    if is_a {
        dec.decode_update(f0, f1);
        row[1] += 1;
    } else {
        dec.decode_update(0, f0);
        row[0] += 1;
    }
    is_a
}

// The colour context: one letter of the (stored) word when `use_color`, else a single shared model.
// The position is build-searched and baked as a constant, so the decoder const-folds this.
pub fn color_ctx(word: &[u8], use_color: bool, pos: usize) -> usize {
    if use_color {
        word[pos] as usize
    } else {
        0
    }
}

// Number of colour contexts for the chosen scheme (26 letters, or one shared model).
pub const fn color_nctx(use_color: bool) -> usize {
    if use_color {
        26
    } else {
        1
    }
}
