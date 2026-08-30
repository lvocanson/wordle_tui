// Build script: compress the two word lists into a single arithmetic-coded blob plus a few
// generated constants, so the runtime carries the data in the smallest form and no tuning
// logic. It is the only place that panics — every step below returns an error the orchestration
// here turns into a readable build failure.

#[allow(dead_code)]
#[path = "../src/codec.rs"]
mod codec;
mod codec_enc;
mod encode;
mod words;

use std::env;
use std::fs;
use std::path::Path;

use encode::best_model;
use words::{merge_words, read_words, word_length, WordKind};

fn main() {
    emit_entry_point_link_args();

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let res = Path::new(&manifest).join("res");
    let out = Path::new(&out_dir);

    // Guess count is a pure game rule — it does not touch the compressed corpus — so it is made
    // configurable at build time without editing code: set WORDLE_MAX_GUESSES. Defaults to 6.
    let max_guesses: usize = match env::var("WORDLE_MAX_GUESSES") {
        Ok(s) => s
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("WORDLE_MAX_GUESSES must be a positive integer, got {s:?}")),
        Err(_) => 6,
    };
    assert!(max_guesses >= 1, "WORDLE_MAX_GUESSES must be at least 1");

    // Word length selects which res/{answer,valid}_words_N.txt pair to compress: set
    // WORDLE_WORD_LEN. Defaults to 5. The value only picks the source files here; the authoritative
    // length is still inferred from the data below and cross-checked against this choice.
    let selected_len: usize = match env::var("WORDLE_WORD_LEN") {
        Ok(s) => s
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("WORDLE_WORD_LEN must be a positive integer, got {s:?}")),
        Err(_) => 5,
    };
    assert!(selected_len >= 1, "WORDLE_WORD_LEN must be at least 1");

    // `--cfg immediate_abort` (set alongside -Cpanic=immediate-abort) gates the now-unreachable
    // panic hook out of main.rs; declare it here so the unexpected-cfg lint stays quiet.
    println!("cargo::rustc-check-cfg=cfg(immediate_abort)");

    let answers_name = format!("answer_words_{selected_len}.txt");
    let valid_name = format!("valid_words_{selected_len}.txt");

    println!("cargo:rerun-if-changed=res/{answers_name}");
    println!("cargo:rerun-if-changed=res/{valid_name}");
    println!("cargo:rerun-if-changed=src/codec.rs");
    println!("cargo:rerun-if-changed=build");
    println!("cargo:rerun-if-env-changed=WORDLE_MAX_GUESSES");
    println!("cargo:rerun-if-env-changed=WORDLE_WORD_LEN");

    let answers_txt = res.join(&answers_name);
    let valid_txt = res.join(&valid_name);
    let mut answers = read_words(&answers_txt).unwrap_or_else(|e| panic!("answer words: {e}"));
    let mut valid = read_words(&valid_txt).unwrap_or_else(|e| panic!("valid words: {e}"));

    let word_len = word_length(answers.iter().chain(&valid).map(|w| w.len()))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        word_len, selected_len,
        "WORDLE_WORD_LEN={selected_len} but res/{answers_name} + res/{valid_name} hold \
         {word_len}-letter words",
    );

    answers.sort_unstable();
    answers.dedup();
    valid.sort_unstable();
    valid.dedup();
    let answer_count = answers.len();

    // Merge into one sorted corpus, tagging each word as an answer (colour A) or valid-only.
    let corpus: Vec<(Vec<u8>, bool)> = merge_words(answers, valid)
        .map(|w| (w.word.symbols(), matches!(w.kind, WordKind::Answer)))
        .collect();

    let best = best_model(&corpus, word_len);

    fs::write(out.join("corpus.bin"), &best.blob)
        .unwrap_or_else(|e| panic!("cannot write corpus.bin: {e}"));
    fs::write(
        out.join("constants.rs"),
        format!(
            "pub const WORD_LEN: usize = {word_len};\n\
             pub const WORD_COUNT: usize = {};\n\
             pub const ANSWER_COUNT: usize = {answer_count};\n\
             pub const ORDER: usize = {};\n\
             pub const USE_POS: bool = {};\n\
             pub const INC: u16 = {};\n\
             pub const REVERSE_WORD: bool = {};\n\
             pub const DESCENDING: bool = {};\n\
             pub const USE_COLOR: bool = {};\n\
             pub const COLOR_POS: usize = {};\n\
             pub const MAX_GUESSES: usize = {max_guesses};\n",
            corpus.len(),
            best.scheme.order,
            best.scheme.use_pos,
            best.scheme.inc,
            best.scheme.reverse_word,
            best.scheme.descending,
            best.scheme.use_color,
            best.scheme.color_pos,
        ),
    )
    .unwrap_or_else(|e| panic!("cannot write constants.rs: {e}"));
}

/// `main.rs` is `#![no_main]` and, on MSVC, defines `mainCRTStartup` itself in place of the CRT's
/// own startup. The linker reads the entry point and the subsystem off the names `main`/`WinMain`,
/// and neither is present, so both have to be stated; `vcruntime` then supplies the
/// `memcpy`/`memmove` the CRT startup object would have brought in. Without the three the link
/// fails outright (LNK1561, LNK1221, unresolved `memcpy`).
///
/// These go through `rustc-link-arg-bins` rather than `.cargo/config.toml`: rustflags reach every
/// crate built for the triple, and `/ENTRY` breaks the link of the proc-macro DLLs among them.
/// The directive is also immune to an env `RUSTFLAGS` overriding the `[target]` block, so the
/// hand-typed build commands in BUILD.md do not have to repeat it.
fn emit_entry_point_link_args() {
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/ENTRY:mainCRTStartup");
        println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:CONSOLE");
        println!("cargo:rustc-link-arg-bins=/defaultlib:vcruntime");
        // Nothing silences LNK4210 (`.CRT` present, its initializers possibly unrun, because
        // bypassing the CRT startup means nothing walks that section): the binary has no `.CRT`
        // at all. No `thread_local!` reaches it, so `tlssup.obj` and its `__xl_a`/`__xl_z` bounds
        // markers are never linked, and there is no C/C++ initializer (`.CRT$XI*`/`XC*`) or TLS
        // callback (`.CRT$XL[B-Y]`) to miss. A `thread_local!` with a destructor is the likely
        // door back in; it registers a real callback that would then never run, and the warning
        // that says so is worth reading rather than suppressing.
    }
}
