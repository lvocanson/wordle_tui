use std::{cmp::Ordering, env, fs, path::Path, time::Instant};

const WORD_LEN: usize = 5;

fn read_and_sort(path: &Path, name: &str) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {name}: {e}"));

    let mut words: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, w)| {
            assert!(
                w.is_ascii() && w.len() == WORD_LEN,
                "{name} line {}: {w:?} — expected {WORD_LEN} ASCII chars, got {}",
                i + 1,
                w.len()
            );
            w.to_string()
        })
        .collect();

    words.sort_unstable();
    words.dedup();
    words
}

// Merge-scan O(n+m) on two sorted lists — verifies they are disjoint.
fn check_disjoint(answers: &[String], valid: &[String]) {
    let (mut i, mut j) = (0, 0);
    while i < answers.len() && j < valid.len() {
        match answers[i].cmp(&valid[j]) {
            Ordering::Equal => panic!(
                "duplicate: '{}' found in both answers and valid",
                answers[i]
            ),
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
        }
    }
}

// Each 5-letter word is a base-26 integer (< 26^5 = 11_881_376). The sorted list is
// stored as LEB128 gaps between consecutive words (first word absolute), which is much
// denser than 3 fixed bytes since typical gaps fit in 1-2 varint bytes. Decoded once at
// startup back into a sorted Vec<u32> for binary search.
fn pack(w: &str) -> u32 {
    let mut v: u32 = 0;
    for &c in w.as_bytes() {
        v = v * 26 + (c - b'a') as u32;
    }
    v
}

fn push_varint(buf: &mut Vec<u8>, mut v: u32) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    }
}

fn write_packed(words: &[String], path: &Path) {
    let mut buf = Vec::with_capacity(words.len() * 2);
    let mut prev = 0u32;
    for (i, w) in words.iter().enumerate() {
        let v = pack(w);
        push_varint(&mut buf, if i == 0 { v } else { v - prev });
        prev = v;
    }
    fs::write(path, buf).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn main() {
    let t0 = Instant::now();

    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    let res = Path::new(&manifest).join("res");
    let out = Path::new(&out_dir);

    println!("cargo:rerun-if-changed=res/answer_words.txt");
    println!("cargo:rerun-if-changed=res/valid_words.txt");

    let answers = read_and_sort(&res.join("answer_words.txt"), "answer_words.txt");
    let valid = read_and_sort(&res.join("valid_words.txt"), "valid_words.txt");

    check_disjoint(&answers, &valid);

    write_packed(&answers, &out.join("answers.bin"));
    write_packed(&valid, &out.join("valid.bin"));

    fs::write(
        out.join("constants.rs"),
        format!("pub const WORD_LEN: usize = {WORD_LEN};\n"),
    )
    .unwrap_or_else(|e| panic!("cannot write constants.rs: {e}"));

    println!(
        "cargo:warning=[wordle] {} answers  +  {} valid extensions  =  {} words  \
         (sorted, disjoint, all {} letters ✓)  [{:.1?}]",
        answers.len(),
        valid.len(),
        answers.len() + valid.len(),
        WORD_LEN,
        t0.elapsed()
    );
}
