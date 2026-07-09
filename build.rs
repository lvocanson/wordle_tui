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

fn write_packed(words: &[String], path: &Path) {
    let mut buf = Vec::with_capacity(words.len() * WORD_LEN);
    for w in words {
        buf.extend_from_slice(w.as_bytes());
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
