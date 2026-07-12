//! Build & binary stats reporter — run with `cargo run --example stats`.
//!
//! Two reports in one place, both kept off the build path so a normal `cargo build` stays quiet:
//!
//! 1. **Compression** — reads the build script's own outputs (the generated `constants.rs` and the
//!    packed `corpus.bin`, located through the package's `OUT_DIR` exactly as `src/words.rs` does)
//!    plus the `res/` sources. This is what used to be the two `cargo::warning=` lines.
//! 2. **Binary size** — the on-disk size of the built game, and on Windows the un-padded
//!    `VirtualSize` of each PE section plus their total (the number to compare between size
//!    changes). This replaces `tools/size.ps1`; unlike that wrapper it only *measures*, so build
//!    the game first, then run this.
//!
//! Being a separate target, none of this links into the game binary.

use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));

const BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/corpus.bin"));

fn main() {
    print_compression();
    print_binary_size();
}

// --- Compression report -------------------------------------------------------------------------

/// The raw word-list sources the build compresses, as (label, path) pairs.
const SOURCES: [(&str, &str); 2] = [
    ("answers", concat!(env!("CARGO_MANIFEST_DIR"), "/res/answer_words.txt")),
    ("valid", concat!(env!("CARGO_MANIFEST_DIR"), "/res/valid_words.txt")),
];

/// On-disk byte size of a source file (0 if it cannot be read).
fn file_len(path: &str) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn print_compression() {
    let valid = WORD_COUNT - ANSWER_COUNT;
    let sizes: Vec<u64> = SOURCES.iter().map(|(_, p)| file_len(p)).collect();
    let source: u64 = sizes.iter().sum();
    let packed = BLOB.len() as u64;

    println!("Wordle data");
    println!(
        "  {} words = {} answers + {} valid extensions (all {WORD_LEN} letters)",
        commas(WORD_COUNT as u64),
        commas(ANSWER_COUNT as u64),
        commas(valid as u64),
    );
    println!();
    println!("{}", row("stage", "bytes", "KiB", "% src"));
    for ((label, _), &size) in SOURCES.iter().zip(&sizes) {
        println!("{}", size_line(label, size, source));
    }
    println!("{}", size_line("source", source, source));
    println!("{}", size_line("packed", packed, source));
    println!("-> {:.2} B/word - encoder order {ORDER}, inc {INC}",
        packed as f64 / WORD_COUNT as f64,
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
    let shown = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(&path);
    println!("\n{}", shown.display());
    println!("  {} B on disk (file-aligned)\n", commas(bytes.len() as u64));

    #[cfg(windows)]
    let total = print_pe_sections(&bytes);
    #[cfg(target_os = "linux")]
    let total = print_elf_sections(&bytes, &path);
    #[cfg(not(any(windows, target_os = "linux")))]
    let total = {
        println!("  (section breakdown is not available on this platform)");
        bytes.len() as u64
    };

    // The packed word corpus is embedded verbatim in the binary (see `src/words.rs`), so its share
    // of the total is a direct read on how much of the game is Wordle data.
    println!();
    println!(
        "  word blob {} B = {:.1}% of the {} B total",
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

/// PE section table: each section's un-padded `VirtualSize` (the real byte count, vs. the
/// 512 B file-aligned `SizeOfRawData`), plus the total to compare between size changes. Parses the
/// header directly, so it targets the MSVC build. Mirrors the old `size.ps1`.
#[cfg(windows)]
fn print_pe_sections(bytes: &[u8]) -> u64 {
    let pe = u32le(bytes, 0x3C) as usize; // e_lfanew -> PE signature
    let num = u16le(bytes, pe + 6) as usize; // NumberOfSections
    let opt = u16le(bytes, pe + 20) as usize; // SizeOfOptionalHeader
    let tab = pe + 24 + opt; // section table follows the optional header

    // Collect first so each row can be shown as a percentage of the total computed below.
    let mut sections = Vec::with_capacity(num);
    let mut total = 0u64;
    for i in 0..num {
        let off = tab + i * 40; // 40 B per IMAGE_SECTION_HEADER
        let name = std::str::from_utf8(&bytes[off..off + 8])
            .unwrap_or("?")
            .trim_end_matches('\0');
        let vsize = u32le(bytes, off + 8) as u64; // VirtualSize (un-padded)
        total += vsize;
        sections.push((name, vsize));
    }

    println!("{}", row("section", "bytes", "KiB", "% total"));
    for (name, vsize) in sections {
        println!("{}", size_line(name, vsize, total));
    }
    println!("{}   <- compare", size_line("total", total, total));
    total
}

/// ELF section table: each section's `sh_size` (un-padded byte count) plus the total — the
/// self-contained equivalent of `size -A`, so no binutils dependency. Handles 64-bit
/// little-endian ELF (the only shape the project targets); anything else falls back to a hint.
#[cfg(target_os = "linux")]
fn print_elf_sections(bytes: &[u8], path: &Path) -> u64 {
    // ELF64 LE only: magic 0x7f'E''L''F', EI_CLASS == 2 (64-bit), EI_DATA == 1 (little-endian).
    if bytes.get(..4) != Some(&b"\x7fELF"[..]) || bytes.get(4) != Some(&2) || bytes.get(5) != Some(&1)
    {
        println!("  (not 64-bit little-endian ELF; use `size -A {}`)", path.display());
        return bytes.len() as u64;
    }
    let shoff = u64le(bytes, 0x28) as usize; // e_shoff: section header table offset
    let shentsize = u16le(bytes, 0x3A) as usize; // e_shentsize
    let shnum = u16le(bytes, 0x3C) as usize; // e_shnum
    let shstrndx = u16le(bytes, 0x3E) as usize; // e_shstrndx: section holding the section-name strings

    // Names are NUL-terminated strings at (.shstrtab file offset + sh_name).
    let strtab = u64le(bytes, shoff + shstrndx * shentsize + 24) as usize; // that section's sh_offset
    let name_at = |sh_name: u32| {
        let start = strtab + sh_name as usize;
        let end = bytes[start..].iter().position(|&b| b == 0).map_or(bytes.len(), |n| start + n);
        std::str::from_utf8(&bytes[start..end]).unwrap_or("?")
    };

    // Collect first so each row can be shown as a percentage of the total computed below.
    let mut sections = Vec::new();
    let mut total = 0u64;
    for i in 1..shnum {
        // skip the SHT_NULL entry at index 0, and the section-name string table (which binutils
        // `size` consumes rather than counts) so the total matches `size -A` exactly.
        if i == shstrndx {
            continue;
        }
        let e = shoff + i * shentsize;
        let size = u64le(bytes, e + 32); // sh_size
        total += size;
        if size == 0 {
            continue;
        }
        let name = name_at(u32le(bytes, e)); // sh_name
        sections.push((name, size));
    }

    println!("{}", row("section", "bytes", "KiB", "% total"));
    for (name, size) in sections {
        println!("{}", size_line(name, size, total));
    }
    println!("{}   <- compare", size_line("total", total, total));
    total
}

/// Label column width: wide enough for long ELF section names on Linux, tight elsewhere.
#[cfg(target_os = "linux")]
const LABEL_W: usize = 18;
#[cfg(not(target_os = "linux"))]
const LABEL_W: usize = 9;

/// One table line — a left-aligned label plus the right-aligned `bytes`, `KiB`, and percentage
/// columns. The single place the layout (and `LABEL_W`) lives; used for both the header and every
/// data row.
fn row(label: &str, bytes: &str, kib: &str, pct: &str) -> String {
    format!("  {label:<w$} {bytes:>11} {kib:>9} {pct:>8}", w = LABEL_W)
}

/// A data row: label, byte count with thousands separators, the same size in KiB, and its share of
/// `total` (blank when `total` is zero, to avoid a divide-by-zero).
fn size_line(label: &str, bytes: u64, total: u64) -> String {
    let pct = if total > 0 {
        format!("{:.1}%", bytes as f64 / total as f64 * 100.0)
    } else {
        String::new()
    };
    row(label, &commas(bytes), &format!("{:.2}", bytes as f64 / 1024.0), &pct)
}

/// Little-endian header readers, shared by the PE and ELF parsers.
#[cfg(any(windows, target_os = "linux"))]
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[cfg(any(windows, target_os = "linux"))]
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[cfg(target_os = "linux")]
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Group an integer with thousands separators for readable columns.
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, d) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*d as char);
    }
    out
}
