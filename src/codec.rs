// Arithmetic-coding codec shared by both ends of the pipeline: the build script pulls this
// file in as a module for the encoder (`#[path = "../src/codec.rs"] mod codec`), and the crate
// compiles it as a module for the decoder. Sharing the exact model and range-coder arithmetic
// is what guarantees encode and decode agree bit for bit — any divergence would silently
// corrupt the stream.
//
// The two word lists are merged into one sorted, distinct corpus (answers + valid extensions)
// and encoded in a single pass. Three sources of redundancy are each targeted directly:
//   * the sort makes the ordering free, and lets consecutive words share a prefix
//     (front-coding: we only encode the suffix that differs);
//   * the first differing character is bounded below by the previous word (the sort proves
//     it is strictly greater), so its distribution is restricted;
//   * remaining characters are predicted by an adaptive order-2 model over the alphabet,
//     which captures most of the linguistic redundancy.
// A per-word colour bit (answer vs. valid-only) is coded by sampling without replacement,
// which spends exactly log2 C(n, n_answers) bits on the partition.

// --- Range coder (LZMA-style, 32-bit range, byte carry via a cache) --------------------

const TOP: u32 = 1 << 24;

// Encoder side. Only used by build.rs; the crate never instantiates it (LTO drops it).
#[allow(dead_code)]
pub struct RangeEncoder {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    pub out: Vec<u8>,
}

#[allow(dead_code)]
impl RangeEncoder {
    pub fn new() -> Self {
        // cache_size starts at 1 so the first shift emits one leading byte the decoder skips.
        RangeEncoder { low: 0, range: 0xFFFF_FFFF, cache: 0, cache_size: 1, out: Vec::new() }
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

// Decoder side. Used by game.rs; harmlessly dead in build.rs.
pub struct RangeDecoder<'a> {
    range: u32,
    code: u32,
    data: &'a [u8],
    pos: usize,
}

impl<'a> RangeDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let mut d = RangeDecoder { range: 0xFFFF_FFFF, code: 0, data, pos: 1 }; // skip leading byte
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

// --- Adaptive model --------------------------------------------------------------------

// Each context character is 0..26, where 26 marks "before the word start". The two model
// knobs — the context length (`order`) and the count increment (`inc`, which sets how fast
// the model trusts what it has seen versus add-one smoothing) — are not fixed here: build.rs
// searches for the pair that compresses this particular data smallest and bakes them in as
// constants. So the runtime carries no tuning logic, only the winning numbers.
const CTX_SYMS: usize = 27;
// A context is halved once its running total reaches LIMIT, keeping totals (plus the +26 from
// add-one smoothing) well under the coder's frequency ceiling regardless of `inc`.
const LIMIT: u32 = 1 << 13;

#[allow(dead_code)] // encoder side (build.rs)
pub fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    i
}

pub struct Model {
    // Character frequencies per context; effective frequency is count + 1 (add-one smoothing).
    counts: Vec<[u16; 26]>,
    total: Vec<u32>,
    // Prefix-length model over 0..word_len, same smoothing.
    pref: Vec<u16>,
    pref_total: u32,
    order: usize,
    inc: u16,
}

impl Model {
    pub fn new(word_len: usize, order: usize, inc: u16) -> Self {
        let n_ctx = CTX_SYMS.pow(order as u32);
        Model {
            counts: vec![[0u16; 26]; n_ctx],
            total: vec![0u32; n_ctx],
            pref: vec![0u16; word_len],
            pref_total: 0,
            order,
            inc,
        }
    }

    // Fold the `order` preceding characters into a base-27 context index.
    pub fn ctx(&self, word: &[u8], i: usize) -> usize {
        let mut idx = 0;
        for k in (1..=self.order).rev() {
            let c = if i >= k { word[i - k] as usize } else { 26 };
            idx = idx * CTX_SYMS + c;
        }
        idx
    }

    // Character model: frequencies are taken over symbols `lo..26`, where `lo` is 0 except at
    // the first differing position, where the sort guarantees the symbol exceeds the floor.
    #[allow(dead_code)] // encoder side (build.rs)
    pub fn char_freq(&self, ctx: usize, sym: usize) -> u32 {
        self.counts[ctx][sym] as u32 + 1
    }

    pub fn char_tot(&self, ctx: usize, lo: usize) -> u32 {
        let mut t = 0;
        for s in lo..26 {
            t += self.counts[ctx][s] as u32 + 1;
        }
        t
    }

    #[allow(dead_code)] // encoder side (build.rs)
    pub fn char_cum(&self, ctx: usize, lo: usize, sym: usize) -> u32 {
        let mut c = 0;
        for s in lo..sym {
            c += self.counts[ctx][s] as u32 + 1;
        }
        c
    }

    // Inverse of char_cum: map a decoded frequency point back to (symbol, cum, freq).
    pub fn char_find(&self, ctx: usize, lo: usize, dv: u32) -> (usize, u32, u32) {
        let mut acc = 0;
        for s in lo..26 {
            let f = self.counts[ctx][s] as u32 + 1;
            if acc + f > dv {
                return (s, acc, f);
            }
            acc += f;
        }
        let f = self.counts[ctx][25] as u32 + 1;
        (25, acc - f, f)
    }

    pub fn update(&mut self, ctx: usize, sym: usize) {
        self.counts[ctx][sym] += self.inc;
        self.total[ctx] += self.inc as u32;
        if self.total[ctx] >= LIMIT {
            let mut t = 0;
            for c in self.counts[ctx].iter_mut() {
                *c >>= 1;
                t += *c as u32;
            }
            self.total[ctx] = t;
        }
    }

    #[allow(dead_code)] // encoder side (build.rs)
    pub fn pref_freq(&self, p: usize) -> u32 {
        self.pref[p] as u32 + 1
    }

    pub fn pref_tot(&self) -> u32 {
        self.pref_total + self.pref.len() as u32
    }

    #[allow(dead_code)] // encoder side (build.rs)
    pub fn pref_cum(&self, p: usize) -> u32 {
        let mut c = 0;
        for s in 0..p {
            c += self.pref[s] as u32 + 1;
        }
        c
    }

    pub fn pref_find(&self, dv: u32) -> (usize, u32, u32) {
        let mut acc = 0;
        for s in 0..self.pref.len() {
            let f = self.pref[s] as u32 + 1;
            if acc + f > dv {
                return (s, acc, f);
            }
            acc += f;
        }
        let last = self.pref.len() - 1;
        let f = self.pref[last] as u32 + 1;
        (last, acc - f, f)
    }

    pub fn pref_update(&mut self, p: usize) {
        self.pref[p] += self.inc;
        self.pref_total += self.inc as u32;
        if self.pref_total >= LIMIT {
            let mut t = 0;
            for c in self.pref.iter_mut() {
                *c >>= 1;
                t += *c as u32;
            }
            self.pref_total = t;
        }
    }
}

// Decode one word (values 0..25) given the previous word, mirroring the encoder in build.rs.
pub fn decode_word(dec: &mut RangeDecoder, m: &mut Model, prev: Option<&[u8]>, word_len: usize) -> Vec<u8> {
    let dv = dec.decode_freq(m.pref_tot());
    let (p, cum, f) = m.pref_find(dv);
    dec.decode_update(cum, f);
    m.pref_update(p);

    let mut w = vec![0u8; word_len];
    let floor: i32 = match prev {
        Some(pv) => {
            w[..p].copy_from_slice(&pv[..p]);
            pv[p] as i32
        }
        None => -1,
    };

    for i in p..word_len {
        let ctx = m.ctx(&w, i);
        let lo = if i == p { (floor + 1) as usize } else { 0 };
        let dv = dec.decode_freq(m.char_tot(ctx, lo));
        let (sym, cum, f) = m.char_find(ctx, lo, dv);
        dec.decode_update(cum, f);
        w[i] = sym as u8;
        m.update(ctx, sym);
    }
    w
}

// Decode the colour bit that follows each word: true = colour A. Sampling without replacement,
// so the probability tracks the counts still to be placed — `remaining` words in all, of which
// `remaining_a` are colour A. Both counts are decremented as the caller walks the stream.
pub fn decode_color(dec: &mut RangeDecoder, remaining: &mut u32, remaining_a: &mut u32) -> bool {
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
