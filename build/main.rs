// Build script: compress the two word lists into a single arithmetic-coded blob plus a few
// generated constants, so the runtime carries the data in the smallest form and no tuning
// logic. It is the only place that panics — every step below returns an error the orchestration
// here turns into a readable build failure.

#[allow(dead_code)] // the decoder half of the codec is exercised by the game, not the build
#[path = "../src/codec.rs"]
mod codec;
mod encode;
mod words;

use std::env;
use std::fs;
use std::path::Path;

use encode::best_model;
use words::{merge_words, read_words, word_length, WordKind};

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let res = Path::new(&manifest).join("res");
    let out = Path::new(&out_dir);

    // `--cfg immediate_abort` (set alongside -Cpanic=immediate-abort) gates the now-unreachable
    // panic hook out of main.rs; declare it here so the unexpected-cfg lint stays quiet.
    println!("cargo::rustc-check-cfg=cfg(immediate_abort)");

    println!("cargo:rerun-if-changed=res/answer_words.txt");
    println!("cargo:rerun-if-changed=res/valid_words.txt");
    println!("cargo:rerun-if-changed=src/codec.rs");
    println!("cargo:rerun-if-changed=build");

    let answers_txt = res.join("answer_words.txt");
    let valid_txt = res.join("valid_words.txt");
    let mut answers = read_words(&answers_txt).unwrap_or_else(|e| panic!("answer words: {e}"));
    let mut valid = read_words(&valid_txt).unwrap_or_else(|e| panic!("valid words: {e}"));

    // One length governs the whole game; infer it from every word in both lists.
    let word_len = word_length(answers.iter().chain(&valid).map(|w| w.len()))
        .unwrap_or_else(|e| panic!("{e}"));

    answers.sort_unstable();
    answers.dedup();
    valid.sort_unstable();
    valid.dedup();
    let answer_count = answers.len();

    // Merge into one sorted corpus, tagging each word as an answer (colour A) or valid-only.
    let corpus: Vec<(Vec<u8>, bool)> = merge_words(answers, valid)
        .map(|w| (w.word.symbols(), matches!(w.kind, WordKind::Answer)))
        .collect();

    let (order, inc, blob) = best_model(&corpus, answer_count as u32, word_len);

    fs::write(out.join("corpus.bin"), &blob)
        .unwrap_or_else(|e| panic!("cannot write corpus.bin: {e}"));
    fs::write(
        out.join("constants.rs"),
        format!(
            "pub const WORD_LEN: usize = {word_len};\n\
             pub const WORD_COUNT: usize = {};\n\
             pub const ANSWER_COUNT: usize = {answer_count};\n\
             pub const ORDER: usize = {order};\n\
             pub const INC: u16 = {inc};\n",
            corpus.len(),
        ),
    )
    .unwrap_or_else(|e| panic!("cannot write constants.rs: {e}"));
}
