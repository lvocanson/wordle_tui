//! Symbol-level size attribution from a linker map — this project's replacement for `cargo bloat`.
//!
//! Run with `cargo run --example bloat` after an immediate-abort build (BUILD.md's commands emit
//! the map: `/MAP:target/wordle_tui.map` on MSVC, `-Wl,-Map=target/wordle_tui-linux.map` on lld).
//! With no argument it reads the freshest of those two; pass a path to pin one, `-n N` for the
//! list length.
//!
//! Why not `cargo bloat`: it re-runs its own build instead of reading the link that produced the
//! shipping binary, and attributes ICF-folded bodies to an arbitrary symbol. The map is
//! emitted by the very link that produced the shipping binary — emitting it is byte-neutral
//! (verified: only the 6 link-timestamp bytes differ) — so every size here is ground truth.
//!
//! MSVC maps carry no sizes: a symbol's size is the gap to the next symbol in its section, so
//! the last symbol of a section absorbs any tail padding, and ICF-folded symbols (same address)
//! are listed together instead of misattributed. lld maps carry exact per-input-section sizes.
//!
//! Crates are read from the demangled symbol names (under fat LTO every Rust symbol lands in the
//! one wordle_tui object, so object-based attribution — what `cargo bloat` shows — is useless).

use std::path::Path;

/// One attributed chunk: its size, the demangled name(s) at that address (several = ICF fold),
/// and the crate (or library) it came from.
struct Sym {
    size: u64,
    names: Vec<String>,
    krate: String,
}

struct Section {
    name: String,
    syms: Vec<Sym>,
}

fn main() {
    let mut top = 25usize;
    let mut path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "-n" {
            top = args.next().and_then(|v| v.parse().ok()).unwrap_or(top);
        } else {
            path = Some(a);
        }
    }
    let Some(path) = path.or_else(default_map) else {
        eprintln!(
            "no linker map found — build one first (see BUILD.md), e.g. on MSVC add\n  \
             -Clink-arg=/MAP:target/wordle_tui.map\nthen run `cargo run --example bloat`"
        );
        std::process::exit(1);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };

    // lld maps start with a "VMA LMA Size Align Out In Symbol" header; MSVC maps with the module
    // name. Sniff the format the same way stats.rs sniffs PE vs ELF.
    let first = text.lines().next().unwrap_or("");
    let (flavor, sections) = if first.contains("VMA") && first.contains("Symbol") {
        ("lld link map", parse_lld(&text, first))
    } else {
        ("MSVC link map", parse_msvc(&text))
    };

    println!("── symbol bloat — {path} ({flavor}) ──");
    print_crate_table(&sections);
    for sec in &sections {
        let total: u64 = sec.syms.iter().map(|s| s.size).sum();
        if total < 1024 {
            continue; // tiny sections (.data, .reloc glue) are noise at symbol level
        }
        println!(
            "\ntop {top} of {} ({} B in {} slots)",
            sec.name,
            commas(total),
            sec.syms.len()
        );
        let mut syms: Vec<&Sym> = sec.syms.iter().collect();
        syms.sort_by_key(|s| std::cmp::Reverse(s.size));
        for s in syms.iter().take(top) {
            let fold = if s.names.len() > 1 {
                format!("  (+{} folded)", s.names.len() - 1)
            } else {
                String::new()
            };
            println!(
                "  {:>9} {:>5.1}%  [{}] {}{}",
                commas(s.size),
                s.size as f64 / total as f64 * 100.0,
                s.krate,
                s.names[0],
                fold
            );
        }
    }
}

/// Freshest existing default map, mirroring how stats.rs picks the freshest binary.
fn default_map() -> Option<String> {
    ["target/wordle_tui.map", "target/wordle_tui-linux.map"]
        .into_iter()
        .filter(|p| Path::new(p).exists())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .map(str::to_string)
}

/// Aggregate every section's bytes by crate and print the totals — the `cargo bloat --crates`
/// view, but per real post-LTO symbol.
fn print_crate_table(sections: &[Section]) {
    let mut crates: Vec<(String, u64)> = Vec::new();
    let mut total = 0u64;
    for sec in sections {
        for s in &sec.syms {
            total += s.size;
            match crates.iter_mut().find(|(k, _)| *k == s.krate) {
                Some((_, b)) => *b += s.size,
                None => crates.push((s.krate.clone(), s.size)),
            }
        }
    }
    crates.sort_by_key(|&(_, b)| std::cmp::Reverse(b));
    println!("\nby crate (all sections, {} B attributed)", commas(total));
    for (k, b) in crates {
        println!(
            "  {:>9} {:>5.1}%  {k}",
            commas(b),
            b as f64 / total as f64 * 100.0
        );
    }
}

// --- MSVC map ----------------------------------------------------------------------------------

/// Parse an MSVC `link /MAP` file. Section-table lines give each section index its extent and
/// display name; symbol lines (`NNNN:OFFSET name ADDRESS [f] [i] lib:object`) give addresses,
/// from which sizes are derived by consecutive-address deltas.
fn parse_msvc(text: &str) -> Vec<Section> {
    // (section index) -> extent, display name (the '$'-stripped name of its largest group entry),
    // and raw (offset, mangled name, lib) symbol rows.
    let mut extents: Vec<(u32, u64)> = Vec::new();
    let mut names: Vec<(u32, u64, String)> = Vec::new();
    let mut rows: Vec<(u32, u64, &str, &str)> = Vec::new();

    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let Some(&sec_off) = t.first() else { continue };
        let Some((sec, off)) = parse_sec_off(sec_off) else {
            continue;
        };
        if sec == 0 {
            continue; // <absolute> pseudo-symbols
        }
        // Section-table entry: "NNNN:OFFSET LENGTHH name CODE|DATA".
        let table_len = if t.len() == 4 && (t[3] == "CODE" || t[3] == "DATA") {
            t[1].strip_suffix('H')
                .and_then(|h| u64::from_str_radix(h, 16).ok())
        } else {
            None
        };
        if let Some(len) = table_len {
            let end = off + len;
            match extents.iter_mut().find(|(s, _)| *s == sec) {
                Some((_, e)) => *e = (*e).max(end),
                None => extents.push((sec, end)),
            }
            let base = t[2].split('$').next().unwrap_or(t[2]).to_string();
            match names.iter_mut().find(|(s, _, _)| *s == sec) {
                Some((_, best, name)) if len > *best => (*best, *name) = (len, base),
                Some(_) => {}
                None => names.push((sec, len, base)),
            }
            continue;
        }
        // Symbol entry: "NNNN:OFFSET name 16-hex-address [f] [i] lib:object".
        if t.len() >= 3 && t[2].len() == 16 && u64::from_str_radix(t[2], 16).is_ok() {
            rows.push((sec, off, t[1], t.last().unwrap_or(&"")));
        }
    }

    let mut sections = Vec::new();
    let mut secs: Vec<u32> = extents.iter().map(|&(s, _)| s).collect();
    secs.sort_unstable();
    for sec in secs {
        let extent = extents.iter().find(|(s, _)| *s == sec).unwrap().1;
        let mut in_sec: Vec<&(u32, u64, &str, &str)> =
            rows.iter().filter(|(s, _, _, _)| *s == sec).collect();
        if in_sec.is_empty() {
            continue;
        }
        in_sec.sort_by_key(|(_, off, _, _)| *off);
        // Group same-address rows (ICF folds), then size each group by the gap to the next.
        let mut syms: Vec<Sym> = Vec::new();
        let mut i = 0;
        while i < in_sec.len() {
            let (_, off, _, _) = *in_sec[i];
            let mut names_here = Vec::new();
            let mut krate = String::new();
            while i < in_sec.len() && in_sec[i].1 == off {
                let (_, _, name, lib) = *in_sec[i];
                names_here.push(display_name(name));
                if krate.is_empty() {
                    krate = crate_of(name, lib);
                }
                i += 1;
            }
            let next = in_sec.get(i).map_or(extent, |(_, o, _, _)| *o);
            // Shortest demangled name first: for a fold it is usually the least noisy alias.
            names_here.sort_by_key(String::len);
            syms.push(Sym {
                size: next - off,
                names: names_here,
                krate,
            });
        }
        let name = names
            .iter()
            .find(|(s, _, _)| *s == sec)
            .map_or_else(|| format!("section {sec}"), |(_, _, n)| n.clone());
        sections.push(Section { name, syms });
    }
    sections
}

/// Split "NNNN:HHHHHHHH" into (section index, offset).
fn parse_sec_off(s: &str) -> Option<(u32, u64)> {
    let (sec, off) = s.split_once(':')?;
    if sec.len() != 4 || off.len() != 8 {
        return None;
    }
    Some((
        u32::from_str_radix(sec, 16).ok()?,
        u64::from_str_radix(off, 16).ok()?,
    ))
}

// --- lld map -----------------------------------------------------------------------------------

/// Parse an lld `-Map` file at input-section granularity: with one function/global per input
/// section (the rustc default), each depth-8 line `file:(.text.<mangled>)` carries an exact size
/// and the mangled name, which is all the attribution needs. ICF-folded copies simply do not
/// appear, so there is nothing to misattribute.
fn parse_lld(text: &str, header: &str) -> Vec<Section> {
    let col_out = header.find("Out").unwrap_or(0);
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines().skip(1) {
        // Four leading numeric columns (hex), then a name whose indent encodes the depth:
        // output section at the header's "Out" column, input section at +8, symbol at +16.
        let mut pos = 0;
        let mut fields = [0u64; 3];
        let mut it = line.char_indices().peekable();
        let mut field = 0;
        while field < 4 {
            while let Some(&(_, c)) = it.peek() {
                if c == ' ' {
                    it.next();
                } else {
                    break;
                }
            }
            let start = match it.peek() {
                Some(&(i, _)) => i,
                None => break,
            };
            while let Some(&(i, c)) = it.peek() {
                if c != ' ' {
                    it.next();
                    pos = i + 1;
                } else {
                    break;
                }
            }
            if field < 3 {
                let Ok(v) = u64::from_str_radix(&line[start..pos], 16) else {
                    break;
                };
                fields[field] = v;
            }
            field += 1;
        }
        if field < 4 {
            continue;
        }
        let Some(rest_at) = line[pos..].find(|c| c != ' ').map(|i| pos + i) else {
            continue;
        };
        let depth = rest_at.saturating_sub(col_out);
        let name = &line[rest_at..];
        let size = fields[2];
        match depth {
            0 => sections.push(Section {
                name: name.to_string(),
                syms: Vec::new(),
            }),
            8 => {
                let Some(sec) = sections.last_mut() else {
                    continue;
                };
                if size == 0 {
                    continue;
                }
                // "path/lib.rlib(obj.o):(.text._RNv...)" -> the inner input-section name.
                let inner = name
                    .rsplit_once(":(")
                    .and_then(|(_, i)| i.strip_suffix(')'))
                    .unwrap_or(name);
                // Strip the output section's own prefix (and rustc's hot/cold qualifiers) to
                // reach the per-symbol suffix.
                let sym = inner
                    .strip_prefix(&format!("{}.", sec.name))
                    .unwrap_or(inner);
                let sym = sym
                    .strip_prefix("unlikely.")
                    .or_else(|| sym.strip_prefix("hot."))
                    .unwrap_or(sym);
                sections.last_mut().unwrap().syms.push(Sym {
                    size,
                    names: vec![display_name(sym)],
                    krate: crate_of(sym, name),
                });
            }
            _ => {} // per-symbol lines add nothing over the input-section rows
        }
    }
    sections.retain(|s| !s.syms.is_empty());
    sections
}

// --- Names and crates --------------------------------------------------------------------------

/// Human-readable form of a symbol name: demangled without the trailing hash for Rust symbols
/// (jump tables keep a marker), unchanged otherwise.
fn display_name(name: &str) -> String {
    if let Some(inner) = name.strip_prefix("switch.table.") {
        return format!("switch table of {}", display_name(inner));
    }
    format!("{:#}", rustc_demangle::demangle(name))
}

/// First path segment of a demangled name ("a::B::c" -> "a").
fn first_seg(d: &str) -> &str {
    let d = d.trim_start_matches(['<', '&', '*']);
    let d = d.strip_prefix("dyn ").unwrap_or(d);
    let end = d.find("::").unwrap_or(d.len());
    &d[..end
        .min(d.find(' ').unwrap_or(d.len()))
        .min(d.find('>').unwrap_or(d.len()))]
}

/// The crate a symbol belongs to. Rust symbols carry it in their mangled name (the only reliable
/// source under fat LTO); everything else is attributed to its library/object.
fn crate_of(name: &str, origin: &str) -> String {
    let name = name.strip_prefix("switch.table.").unwrap_or(name);
    if name.starts_with("_R") || name.starts_with("_ZN") {
        let d = format!("{:#}", rustc_demangle::demangle(name));
        let seg = first_seg(&d);
        // Impl-for-primitive symbols ("<i32 as core::fmt::Display>::fmt") belong to the crate on
        // the trait side of the `as`.
        let primitive = seg.starts_with(['[', '(', 'i', 'u', 'f'])
            && !seg.contains(char::is_uppercase)
            && seg.len() <= 5
            || matches!(seg, "bool" | "char" | "str");
        if primitive {
            if let Some((_, trait_side)) = d.split_once(" as ") {
                return first_seg(trait_side).to_string();
            }
        }
        return seg.to_string();
    }
    if name.starts_with("anon.") || name.starts_with(".L") {
        return "const data".to_string();
    }
    // MSVC "lib:object" / lld "path/libfoo.rlib(obj)" origins.
    if let Some((lib, _)) = origin.split_once(':') {
        let base = lib.rsplit(['/', '\\']).next().unwrap_or(lib);
        let base = base.split('(').next().unwrap_or(base);
        let base = base.trim_matches(['<', '>']);
        let base = base.strip_prefix("lib").unwrap_or(base);
        return base.split(['-', '.']).next().unwrap_or(base).to_string();
    }
    if origin == name || origin.is_empty() {
        return "other".to_string();
    }
    let base = origin.trim_matches(['<', '>']);
    base.split(['-', '.']).next().unwrap_or(base).to_string()
}

/// Group an integer with thousands separators (same helper as stats.rs).
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
