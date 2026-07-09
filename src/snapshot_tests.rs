//! Fidelity harness for the crossterm rewrite.
//!
//! `cargo test --release write_reference_snapshots -- --ignored --nocapture` renders
//! the current UI through ratatui's TestBackend and writes glyph+color dumps to
//! `snapshots/`. After the crossterm rewrite, `crossterm_matches_reference` re-renders
//! the same scenarios through the new grid renderer and asserts byte-for-byte equality.

use crate::app::{App, Phase};
use crate::game::{LetterState, MAX_GUESSES, WORD_LEN};

use LetterState::*;

/// Map a color to a single stable character for dumping.
#[cfg(feature = "ratatui-ref")]
pub fn color_code(c: ratatui::style::Color) -> char {
    use ratatui::style::Color::*;
    match c {
        Reset => '.',
        White => 'w',
        Green => 'G',
        Yellow => 'Y',
        Gray => 'g',
        DarkGray => 'd',
        Red => 'r',
        _ => '?',
    }
}

fn make_app(
    target: &[u8; WORD_LEN],
    guesses: &[(&str, [LetterState; WORD_LEN])],
    input: &str,
    phase: Phase,
    message: Option<&str>,
) -> App {
    let mut history = [([0u8; WORD_LEN], [WrongSpot; WORD_LEN]); MAX_GUESSES];
    for (i, (w, res)) in guesses.iter().enumerate() {
        history[i].0.copy_from_slice(w.as_bytes());
        history[i].1 = *res;
    }
    let idx = guesses.len();
    if !input.is_empty() {
        history[idx].0[..input.len()].copy_from_slice(input.as_bytes());
    }
    App {
        target: *target,
        history,
        input_idx: idx,
        input_len: input.len(),
        phase,
        message: message.map(|s| s.to_string()),
        controls: match phase {
            Phase::Playing => "<Enter> Submit  <Esc> Quit",
            _ => "<R/Enter> Restart  <Q/Esc> Quit",
        },
    }
}

/// (name, width, height, app)
fn scenarios() -> Vec<(String, u16, u16, App)> {
    let g = [CorrectSpot; WORD_LEN];
    let mixed = [
        CorrectSpot,
        NotInAnySpot,
        WrongSpot,
        NotInAnySpot,
        WrongSpot,
    ];
    let all6 = [
        ("slate", mixed),
        ("slate", mixed),
        ("slate", mixed),
        ("slate", mixed),
        ("slate", mixed),
        ("slate", mixed),
    ];
    vec![
        (
            "vertical_fresh".into(),
            60,
            30,
            make_app(b"crane", &[], "", Phase::Playing, None),
        ),
        (
            "vertical_draft".into(),
            60,
            30,
            make_app(b"crane", &[("slate", mixed)], "cr", Phase::Playing, None),
        ),
        (
            "vertical_msg".into(),
            60,
            30,
            make_app(b"crane", &[], "abc", Phase::Playing, Some("Invalid word.")),
        ),
        (
            "vertical_won".into(),
            60,
            32,
            make_app(
                b"crane",
                &[("slate", mixed), ("crane", g)],
                "",
                Phase::Won,
                Some("Found in 2 guesses. Magnificent!"),
            ),
        ),
        (
            "vertical_lost".into(),
            55,
            45,
            make_app(
                b"crane",
                &all6,
                "",
                Phase::Lost,
                Some("The word was CRANE."),
            ),
        ),
        (
            "vertical_tall".into(),
            41,
            50,
            make_app(b"crane", &[("slate", mixed)], "abc", Phase::Playing, None),
        ),
        (
            "vertical_threshold".into(),
            40,
            29,
            make_app(b"crane", &[], "", Phase::Playing, None),
        ),
        (
            "horizontal".into(),
            90,
            18,
            make_app(b"crane", &[("slate", mixed)], "cr", Phase::Playing, None),
        ),
        (
            "horizontal_narrow".into(),
            75,
            15,
            make_app(b"crane", &[("slate", mixed)], "", Phase::Playing, None),
        ),
        (
            "horizontal_won".into(),
            100,
            20,
            make_app(
                b"crane",
                &[("slate", mixed), ("crane", g)],
                "",
                Phase::Won,
                Some("Found in 2 guesses. Magnificent!"),
            ),
        ),
        (
            "horizontal_wide".into(),
            120,
            25,
            make_app(b"crane", &[], "", Phase::Playing, None),
        ),
        (
            "horizontal_oddwidth".into(),
            101,
            22,
            make_app(b"crane", &[], "ab", Phase::Playing, None),
        ),
        (
            "too_small_both".into(),
            3,
            2,
            make_app(b"crane", &[], "", Phase::Playing, None),
        ),
        (
            "too_small_w".into(),
            4,
            10,
            make_app(b"crane", &[], "", Phase::Playing, None),
        ),
        (
            "too_small_h".into(),
            20,
            5,
            make_app(b"crane", &[], "", Phase::Playing, None),
        ),
    ]
}

/// Render through ratatui TestBackend and dump glyphs + bg + fg grids.
#[cfg(feature = "ratatui-ref")]
fn dump_ratatui(width: u16, height: u16, app: &App) -> String {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::ui_ref::draw(f, app)).unwrap();
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for layer in 0..3 {
        for y in 0..height {
            for x in 0..width {
                let cell = &buf[(x, y)];
                let ch = match layer {
                    0 => cell.symbol().chars().next().unwrap_or(' '),
                    1 => color_code(cell.bg),
                    _ => color_code(cell.fg),
                };
                let ch = if ch == '\0' { ' ' } else { ch };
                out.push(ch);
            }
            out.push('\n');
        }
        out.push_str("----\n");
    }
    out
}

#[cfg(feature = "ratatui-ref")]
#[test]
#[ignore]
fn probe_layout() {
    use ratatui::layout::{Constraint, Flex, Layout, Rect};
    // Reproduce vertical_sections for a sweep of heights.
    eprintln!("== vertical_sections (title1, board23, kb3, footer2, SpaceBetween) ==");
    for h in [29u16, 30, 31, 32, 33, 34, 35, 36, 40, 45, 50] {
        let area = Rect::new(0, 0, 39, h);
        let r = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(23),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .flex(Flex::SpaceBetween)
        .split(area);
        eprintln!(
            "h={h}: title.y={} board.y={} kb.y={} footer.y={}",
            r[0].y, r[1].y, r[2].y, r[3].y
        );
    }
    eprintln!("== keyboard_column_sections (title1, kb3, footer2, SpaceBetween) after centered_vert(Max23) ==");
    for h in [6u16, 7, 8, 10, 15, 20, 23, 30] {
        let area = Rect::new(0, 0, 30, h).centered_vertically(Constraint::Max(23));
        let r = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .flex(Flex::SpaceBetween)
        .split(area);
        eprintln!(
            "h={h}: area.y={} area.h={} title.y={} kb.y={} footer.y={}",
            area.y, area.height, r[0].y, r[1].y, r[2].y
        );
    }
    eprintln!("== horizontal columns (Fill1, Min39, Max23, Length30, Fill1, SpaceEvenly) ==");
    for w in [69u16, 70, 72, 75, 80, 90, 100, 120] {
        let area = Rect::new(0, 0, w, 18);
        let r = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Min(39),
            Constraint::Max(23),
            Constraint::Length(30),
            Constraint::Fill(1),
        ])
        .flex(Flex::SpaceEvenly)
        .split(area);
        eprintln!(
            "w={w}: c0(x={},w={}) c1(x={},w={}) c2(x={},w={}) c3(x={},w={}) c4(x={},w={})",
            r[0].x,
            r[0].width,
            r[1].x,
            r[1].width,
            r[2].x,
            r[2].width,
            r[3].x,
            r[3].width,
            r[4].x,
            r[4].width
        );
    }
    eprintln!("== horizontal Fill split (odd leftover) ==");
    for w in [101u16, 103, 105, 107, 111] {
        let area = Rect::new(0, 0, w, 18);
        let r = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Min(39),
            Constraint::Max(23),
            Constraint::Length(30),
            Constraint::Fill(1),
        ])
        .flex(Flex::SpaceEvenly)
        .split(area);
        eprintln!(
            "w={w}: c0(x={},w={}) c4(x={},w={})",
            r[0].x, r[0].width, r[4].x, r[4].width
        );
    }
    eprintln!("== board rows: 6x Max3 SpaceBetween in height H ==");
    for h in [18u16, 19, 20, 21, 22, 23] {
        let r = Layout::vertical([Constraint::Max(3); 6])
            .flex(Flex::SpaceBetween)
            .split(Rect::new(0, 0, 39, h));
        let ys: Vec<u16> = r.iter().map(|x| x.y).collect();
        let hs: Vec<u16> = r.iter().map(|x| x.height).collect();
        eprintln!("h={h}: ys={ys:?} hs={hs:?}");
    }
    eprintln!("== board internal: centered(Max39,Max23), then rows Max3 x6 SpaceBetween ==");
    for (w, h) in [(39u16, 23u16), (50, 30), (45, 25)] {
        let area = Rect::new(0, 0, w, h).centered(Constraint::Max(39), Constraint::Max(23));
        eprintln!(
            "area {w}x{h} -> centered x={} y={} w={} h={}",
            area.x, area.y, area.width, area.height
        );
    }
    eprintln!("== keyboard row Center flex: Max3 x N in width 30 ==");
    for n in [7usize, 9, 10] {
        let area = Rect::new(0, 0, 30, 1);
        let r = Layout::horizontal(vec![Constraint::Max(3); n])
            .flex(Flex::Center)
            .split(area);
        eprintln!(
            "n={n}: first.x={} last.x={} w={}",
            r[0].x,
            r[n - 1].x,
            r[0].width
        );
    }
}

#[cfg(feature = "ratatui-ref")]
#[test]
#[ignore]
fn write_reference_snapshots() {
    std::fs::create_dir_all("snapshots").unwrap();
    for (name, w, h, app) in scenarios() {
        let dump = dump_ratatui(w, h, &app);
        std::fs::write(format!("snapshots/{name}.txt"), dump).unwrap();
    }
    eprintln!("wrote reference snapshots");
}

/// After the rewrite: render via the new grid renderer and compare to references.
#[cfg(not(feature = "ratatui-ref"))]
#[test]
fn crossterm_matches_reference() {
    // Reference snapshots are generated locally and not committed. Skip when absent
    // (regenerate with `--features ratatui-ref write_reference_snapshots -- --ignored`).
    if !std::path::Path::new("snapshots").exists() {
        eprintln!("snapshots/ absent — skipping crossterm fidelity check");
        return;
    }
    let mut failures = Vec::new();
    for (name, w, h, app) in scenarios() {
        let expected = std::fs::read_to_string(format!("snapshots/{name}.txt"))
            .unwrap_or_else(|e| panic!("missing snapshot {name}: {e}"));
        // Tolerate CRLF from git line-ending normalization.
        let expected = expected.replace('\r', "");
        let got = crate::ui::dump_grid(w, h, &app);
        if got != expected {
            let _ = std::fs::create_dir_all("snapshots_got");
            std::fs::write(format!("snapshots_got/{name}.txt"), &got).unwrap();
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "mismatched scenarios: {failures:?}");
}
