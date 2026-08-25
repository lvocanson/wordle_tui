//! Build & binary stats reporter — run with `cargo run --example stats`.
//!
//! Two reports in one place, both kept off the build path so a normal `cargo build` stays quiet:
//!
//! 1. **Compression** — reads the build script's own outputs (the generated `constants.rs` and the
//!    packed `corpus.bin`, located through the package's `OUT_DIR` exactly as `src/words.rs` does)
//!    plus the `res/` sources.
//! 2. **Binary size** — the on-disk size of the built game plus every section's un-padded size
//!    (PE `VirtualSize`, ELF `sh_size`) and their total: the number to compare between size
//!    changes. The format is sniffed from the file itself, not from the host, so either
//!    platform's binary can be measured from either host. Each run records its section sizes in
//!    a `.sections` sidecar next to the measured binary and prints the delta against the
//!    previous run of that same binary.
//!
//! Being a separate target, none of this links into the game binary. It only *measures* — build
//! the game first, then run this (optionally passing an explicit path to the binary).

use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));

const BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/corpus.bin"));

fn main() {
    print_compression();
    print_binary_size();
}

// --- Compression report -------------------------------------------------------------------------

/// On-disk byte size of a source file (0 if it cannot be read).
fn file_len(path: &str) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn print_compression() {
    // The raw word-list sources the build compressed; the length suffix mirrors the file the
    // build selected via WORDLE_WORD_LEN, read back from the generated WORD_LEN.
    let res = concat!(env!("CARGO_MANIFEST_DIR"), "/res");
    let answers = file_len(&format!("{res}/answer_words_{WORD_LEN}.txt"));
    let valid = file_len(&format!("{res}/valid_words_{WORD_LEN}.txt"));
    let source = answers + valid;
    let packed = BLOB.len() as u64;

    println!("Wordle data");
    println!(
        "  {} words = {} answers + {} valid extensions (all {WORD_LEN} letters)",
        commas(WORD_COUNT as u64),
        commas(ANSWER_COUNT as u64),
        commas((WORD_COUNT - ANSWER_COUNT) as u64),
    );
    println!();
    print_table(
        ["stage", "bytes", "KiB", "% src"],
        &[
            ("answers", answers),
            ("valid", valid),
            ("source", source),
            ("packed", packed),
        ],
        source,
    );
    println!(
        "-> {:.2} B/word ({:.2} b/letter) - encoder order {ORDER}{}, inc {INC}, colour {}",
        packed as f64 / WORD_COUNT as f64,
        (packed * 8) as f64 / (WORD_COUNT * WORD_LEN) as f64,
        if USE_POS { "+pos" } else { "" },
        if USE_COLOR {
            format!("@{COLOR_POS}")
        } else {
            "global".to_string()
        },
    );
}

// --- Binary size report -------------------------------------------------------------------------

fn print_binary_size() {
    let Some(path) = game_binary() else {
        println!(
            "\n(game binary not found — build it first (see BUILD.md), or pass a path:\n \
             cargo run --example stats -- <path-to-binary>)"
        );
        return;
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            println!("\n(cannot read {}: {e})", path.display());
            return;
        }
    };
    // Show the path (relative to the crate when possible): both profiles produce a `wordle_tui.exe`,
    // so the bare file name would not say which one was measured.
    let shown = path
        .strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(&path);
    println!("\n{}", shown.display());
    println!(
        "  {} B on disk (file-aligned)\n",
        commas(bytes.len() as u64)
    );

    let Some((sections, uninit)) = parse_sections(&bytes) else {
        println!("  (unrecognized format — expected PE or 64-bit little-endian ELF)");
        print_blob_share(bytes.len() as u64);
        return;
    };
    let total: u64 = sections.iter().map(|(_, s)| s).sum();
    let rows: Vec<(&str, u64)> = sections.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    let w = print_table(["section", "bytes", "KiB", "% total"], &rows, total);
    println!("{}   <- compare", size_line(w, "total", total, total));
    if uninit > 0 {
        // Zero-initialized data occupies VirtualSize/sh_size but no file bytes, so it inflates
        // the compare total relative to what actually ships. Flagged, not subtracted: the total
        // must stay comparable with every previously recorded measurement.
        println!(
            "  (!) {} B of that total is zero-initialized data: memory, not shipped bytes",
            commas(uninit)
        );
    }
    print_delta(&path, &sections, total);
    print_blob_share(total);
}

/// The packed word corpus is embedded verbatim in the binary (see `src/words.rs`), so its share
/// of the total is a direct read on how much of the game is Wordle data.
fn print_blob_share(total: u64) {
    println!(
        "\n  word blob {} B = {:.1}% of the {} B total",
        commas(BLOB.len() as u64),
        BLOB.len() as f64 / total as f64 * 100.0,
        commas(total),
    );
}

/// Locate the built game binary: an explicit CLI argument wins, otherwise pick the most recently
/// built of the release outputs BUILD.md documents (plain `target/release` and the release triple),
/// so it tracks the last build rather than a stale sibling. `None` if none exists yet.
fn game_binary() -> Option<PathBuf> {
    if let Some(arg) = std::env::args().nth(1) {
        return Some(PathBuf::from(arg));
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .unwrap_or_else(|_| format!("{}/target", env!("CARGO_MANIFEST_DIR")));
    let exe = format!("wordle_tui{}", std::env::consts::EXE_SUFFIX);

    #[cfg(windows)]
    let triple = "x86_64-pc-windows-msvc";
    #[cfg(not(windows))]
    let triple = "x86_64-unknown-linux-gnu";

    let candidates = [
        Path::new(&target).join("release").join(&exe),
        Path::new(&target).join(triple).join("release").join(&exe),
    ];
    candidates
        .into_iter()
        .filter(|p| p.exists())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

// --- Section parsing ----------------------------------------------------------------------------

/// Section table of a binary, sniffed from its magic bytes: `(name, un-padded size)` rows plus
/// the byte count of zero-initialized data those sizes include (memory the file never stores).
/// `None` for anything that is neither PE nor 64-bit little-endian ELF (the only shapes the
/// project targets).
fn parse_sections(bytes: &[u8]) -> Option<(Vec<(String, u64)>, u64)> {
    if bytes.starts_with(b"MZ") {
        Some(pe_sections(bytes))
    } else if bytes.starts_with(b"\x7fELF") && bytes.get(4) == Some(&2) && bytes.get(5) == Some(&1)
    {
        Some(elf_sections(bytes))
    } else {
        None
    }
}

/// PE section table: each section's un-padded `VirtualSize` (the real byte count, vs. the
/// 512 B file-aligned `SizeOfRawData`). Zero-initialized data is the part of `VirtualSize` that
/// exceeds `SizeOfRawData`.
fn pe_sections(bytes: &[u8]) -> (Vec<(String, u64)>, u64) {
    let pe = u32le(bytes, 0x3C) as usize; // e_lfanew -> PE signature
    let num = u16le(bytes, pe + 6) as usize; // NumberOfSections
    let opt = u16le(bytes, pe + 20) as usize; // SizeOfOptionalHeader
    let tab = pe + 24 + opt; // section table follows the optional header

    let mut sections = Vec::with_capacity(num);
    let mut uninit = 0u64;
    for i in 0..num {
        let off = tab + i * 40; // 40 B per IMAGE_SECTION_HEADER
        let name = std::str::from_utf8(&bytes[off..off + 8])
            .unwrap_or("?")
            .trim_end_matches('\0');
        let vsize = u32le(bytes, off + 8) as u64; // VirtualSize (un-padded)
        let raw = u32le(bytes, off + 16) as u64; // SizeOfRawData (file-aligned)
        uninit += vsize.saturating_sub(raw);
        sections.push((name.to_string(), vsize));
    }
    (sections, uninit)
}

/// ELF section table: each section's `sh_size` (un-padded byte count) — the self-contained
/// equivalent of `size -A`, so no binutils dependency. Zero-initialized data is the `SHT_NOBITS`
/// (`.bss`) sections, whose `sh_size` occupies no file bytes.
fn elf_sections(bytes: &[u8]) -> (Vec<(String, u64)>, u64) {
    let shoff = u64le(bytes, 0x28) as usize; // e_shoff: section header table offset
    let shentsize = u16le(bytes, 0x3A) as usize; // e_shentsize
    let shnum = u16le(bytes, 0x3C) as usize; // e_shnum
    let shstrndx = u16le(bytes, 0x3E) as usize; // e_shstrndx: section holding the section names

    // Names are NUL-terminated strings at (.shstrtab file offset + sh_name).
    let strtab = u64le(bytes, shoff + shstrndx * shentsize + 24) as usize; // that section's sh_offset
    let name_at = |sh_name: u32| {
        let start = strtab + sh_name as usize;
        let end = bytes[start..]
            .iter()
            .position(|&b| b == 0)
            .map_or(bytes.len(), |n| start + n);
        String::from_utf8_lossy(&bytes[start..end]).into_owned()
    };

    let mut sections = Vec::new();
    let mut uninit = 0u64;
    for i in 1..shnum {
        // Skip the SHT_NULL entry at index 0, the section-name string table (which binutils
        // `size` consumes rather than counts) and empty sections, so the section list — and
        // therefore the total — matches `size -A` exactly.
        if i == shstrndx {
            continue;
        }
        let e = shoff + i * shentsize;
        let size = u64le(bytes, e + 32); // sh_size
        if size == 0 {
            continue;
        }
        if u32le(bytes, e + 4) == 8 {
            uninit += size; // sh_type == SHT_NOBITS
        }
        sections.push((name_at(u32le(bytes, e)), size)); // sh_name
    }
    (sections, uninit)
}

// --- Previous-run delta -------------------------------------------------------------------------

/// Compare this run against the previous run of the same binary — recorded as `name<TAB>bytes`
/// lines in a `.sections` sidecar next to it — print which sections moved, then rewrite the
/// sidecar so the next run compares against this one.
fn print_delta(bin: &Path, sections: &[(String, u64)], total: u64) {
    let sidecar = bin.with_extension("sections");
    if let Ok(prev) = std::fs::read_to_string(&sidecar) {
        let old: Vec<(&str, i64)> = prev
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .filter_map(|(n, s)| s.parse().ok().map(|s| (n, s)))
            .collect();
        let old_size = |name: &str| old.iter().find(|(n, _)| *n == name).map_or(0, |(_, s)| *s);
        // Delta of every section present now, plus the full size of any that disappeared.
        let mut moved: Vec<(&str, i64)> = sections
            .iter()
            .map(|(n, s)| (n.as_str(), *s as i64 - old_size(n)))
            .collect();
        moved.extend(
            old.iter()
                .filter(|(n, _)| *n != "total" && !sections.iter().any(|(m, _)| m == n))
                .map(|(n, s)| (*n, -s)),
        );
        moved.retain(|(_, d)| *d != 0);
        if moved.is_empty() {
            println!("  no change since the previous run");
        } else {
            let parts: Vec<String> = moved
                .iter()
                .map(|(n, d)| format!("{n} {}", signed(*d)))
                .collect();
            println!(
                "  since previous run: {} ({})",
                signed(total as i64 - old_size("total")),
                parts.join(", ")
            );
        }
    } else {
        println!("  (no previous run recorded here; the next run will print the delta)");
    }
    let mut out = format!("total\t{total}\n");
    for (n, s) in sections {
        out.push_str(&format!("{n}\t{s}\n"));
    }
    if let Err(e) = std::fs::write(&sidecar, out) {
        println!("  (could not record this run: {e})");
    }
}

// --- Formatting ---------------------------------------------------------------------------------

/// Print a table: a header line, then one row per `(label, bytes)` with the byte count, KiB and
/// share-of-`base` columns. Returns the label-column width (sized to the longest label) so
/// callers can append extra aligned lines via `size_line`.
fn print_table(head: [&str; 4], rows: &[(&str, u64)], base: u64) -> usize {
    let w = rows
        .iter()
        .map(|(l, _)| l.len())
        .chain([head[0].len()])
        .max()
        .unwrap_or(0);
    println!("{}", row(w, head[0], head[1], head[2], head[3]));
    for &(label, bytes) in rows {
        println!("{}", size_line(w, label, bytes, base));
    }
    w
}

/// One table line — a left-aligned label plus the right-aligned `bytes`, `KiB`, and percentage
/// columns. The single place the column layout lives.
fn row(w: usize, label: &str, bytes: &str, kib: &str, pct: &str) -> String {
    format!("  {label:<w$} {bytes:>11} {kib:>9} {pct:>8}")
}

/// A data row: label, byte count with thousands separators, the same size in KiB, and its share of
/// `base` (blank when `base` is zero, to avoid a divide-by-zero).
fn size_line(w: usize, label: &str, bytes: u64, base: u64) -> String {
    let pct = if base > 0 {
        format!("{:.1}%", bytes as f64 / base as f64 * 100.0)
    } else {
        String::new()
    };
    row(
        w,
        label,
        &commas(bytes),
        &format!("{:.2}", bytes as f64 / 1024.0),
        &pct,
    )
}

/// A signed byte delta with an explicit sign and thousands separators.
fn signed(n: i64) -> String {
    if n < 0 {
        format!("-{}", commas(n.unsigned_abs()))
    } else {
        format!("+{}", commas(n as u64))
    }
}

/// Little-endian header readers, shared by the PE and ELF parsers.
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Group an integer with thousands separators for readable columns.
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, d) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*d as char);
    }
    out
}
