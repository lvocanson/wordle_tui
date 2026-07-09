//! Direct crossterm rendering — no ratatui. An in-memory `Grid` is filled to
//! reproduce the exact same layout ratatui produced (verified cell-for-cell against
//! reference snapshots in `snapshot_tests`), then flushed to the terminal.

use std::io::Write;

use crossterm::style::Color;

use crate::app::{App, Phase};
use crate::game::{LetterState, MAX_GUESSES, WORD_LEN};

const CELL_W: usize = 7;
const CELL_H: usize = 3;
const BOARD_W: usize = WORD_LEN * (CELL_W + 1) - 1; // 39
const BOARD_H: usize = MAX_GUESSES * (CELL_H + 1) - 1; // 23

const KEYBOARD_ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const KEY_W: usize = 3;
const KB_W: usize = 10 * (KEY_W + 1) - 1; // 39, widest row (10 keys, 1-col gaps)
const KEY_H: usize = 1;
const KB_H: usize = KEYBOARD_ROWS.len() * (KEY_H + 1) - 1; // 5, 3 rows with 1-row gaps

const TITLE_H: usize = 1;
const FOOTER_H: usize = 2;
const REQ_VERT: usize = TITLE_H + BOARD_H + KB_H + FOOTER_H; // 31

// Preferred startup terminal size (cols, rows): the vertical layout plus a
// 1-row gap between each of the 4 categories (SpaceBetween spreads +3 evenly).
pub const PREFERRED_SIZE: (u16, u16) = (BOARD_W as u16, (REQ_VERT + 3) as u16);
const MINI_W: usize = WORD_LEN; // 5
const MINI_H: usize = MAX_GUESSES; // 6

// The too-small message hardcodes these dimensions; keep them in sync.
const _: () = assert!(MINI_W == 5 && MINI_H == 6);

// crossterm colors matching ratatui's crossterm backend conversion:
// Green->DarkGreen, Yellow->DarkYellow, Red->DarkRed, Gray->Grey, DarkGray->DarkGrey.
const BG: Color = Color::DarkGrey;

// ----------------------------------------------------------------------------
// Grid model
// ----------------------------------------------------------------------------

pub struct Grid {
    w: usize,
    h: usize,
    ch: Vec<u8>,
    fg: Vec<Color>,
    bg: Vec<Color>,
    bold: Vec<bool>,
}

impl Grid {
    fn new(w: usize, h: usize) -> Self {
        Grid {
            w,
            h,
            ch: vec![b' '; w * h],
            fg: vec![Color::Reset; w * h],
            bg: vec![Color::Reset; w * h],
            bold: vec![false; w * h],
        }
    }

    fn set(&mut self, x: usize, y: usize, ch: u8, fg: Color, bg: Color, bold: bool) {
        if x < self.w && y < self.h {
            let i = y * self.w + x;
            self.ch[i] = ch;
            self.fg[i] = fg;
            self.bg[i] = bg;
            self.bold[i] = bold;
        }
    }

    fn text(&mut self, x: usize, y: usize, s: &[u8], fg: Color, bg: Color, bold: bool) {
        for (i, &b) in s.iter().enumerate() {
            self.set(x + i, y, b, fg, bg, bold);
        }
    }
}

// ----------------------------------------------------------------------------
// Layout primitives — reproduce ratatui's cassowary integer rounding.
// ----------------------------------------------------------------------------

// round-half-up of a/b for non-negative integers.
fn rnd(a: usize, b: usize) -> usize {
    (2 * a + b) / (2 * b)
}

// Gap widths distributed between `n` items across `extra` leftover (Flex::SpaceBetween).
fn gaps(n: usize, extra: usize) -> [usize; MAX_GUESSES] {
    let mut g = [0usize; MAX_GUESSES];
    if n <= 1 {
        return g;
    }
    let d = n - 1;
    for i in 0..d {
        g[i] = rnd((i + 1) * extra, d) - rnd(i * extra, d);
    }
    g
}

// Offset of an `inner`-sized block centered within `outer` (Flex::Center, ceil).
fn center(outer: usize, inner: usize) -> usize {
    if outer <= inner {
        0
    } else {
        (outer - inner + 1) / 2
    }
}

// SpaceBetween placement of `n` equal items of size `s` within `len`.
// Returns (start, size) pairs. Degrades gracefully when the items don't fit.
fn stack(n: usize, s: usize, len: usize) -> [(usize, usize); MAX_GUESSES] {
    let mut out = [(0usize, 0usize); MAX_GUESSES];
    if len >= n * s {
        let g = gaps(n, len - n * s);
        let mut pos = 0;
        for i in 0..n {
            out[i] = (pos, s);
            pos += s + g[i];
        }
    } else {
        // Squeezed: split available length as evenly as possible, no gaps.
        let mut pos = 0;
        for i in 0..n {
            let sz = rnd((i + 1) * len, n) - rnd(i * len, n);
            out[i] = (pos, sz);
            pos += sz;
        }
    }
    out
}

// SpaceBetween placement of items with individual `sizes` within `len` (title/kb/footer).
fn stack_sizes(sizes: &[usize], len: usize, out: &mut [usize]) {
    let n = sizes.len();
    let sum: usize = sizes.iter().sum();
    let mut pos = 0;
    if len >= sum {
        let g = gaps(n, len - sum);
        for i in 0..n {
            out[i] = pos;
            pos += sizes[i] + g[i];
        }
    } else {
        for i in 0..n {
            out[i] = pos.min(len.saturating_sub(1));
            pos += sizes[i];
        }
    }
}

// ----------------------------------------------------------------------------
// Cell colors
// ----------------------------------------------------------------------------

enum CellColor {
    Empty,
    Draft,
    Submitted(LetterState),
}

fn cell_colors(c: &CellColor) -> (Color, Color) {
    match c {
        CellColor::Empty => (BG, BG),
        CellColor::Draft => (BG, Color::White),
        CellColor::Submitted(s) => match s {
            LetterState::CorrectSpot => (Color::DarkGreen, Color::White),
            LetterState::WrongSpot => (Color::DarkYellow, Color::White),
            LetterState::NotInAnySpot => (Color::Grey, Color::White),
        },
    }
}

fn draw_cell(grid: &mut Grid, x: usize, y: usize, w: usize, h: usize, letter: u8, c: CellColor) {
    let (bg, fg) = cell_colors(&c);
    for dy in 0..h {
        for dx in 0..w {
            grid.set(x + dx, y + dy, b' ', fg, bg, false);
        }
    }
    if letter != 0 && w > 0 && h > 0 {
        grid.set(
            x + (w - 1) / 2,
            y + (h - 1) / 2,
            letter.to_ascii_uppercase(),
            fg,
            bg,
            false,
        );
    }
}

// ----------------------------------------------------------------------------
// Widgets
// ----------------------------------------------------------------------------

fn draw_title(grid: &mut Grid, x: usize, y: usize, w: usize) {
    const T: &[u8] = b"WORDLE TUI";
    let off = w.saturating_sub(T.len()) / 2; // Line::centered = (w-len)/2
    grid.text(x + off, y, T, Color::Reset, Color::Reset, true);
}

fn draw_footer(grid: &mut Grid, app: &App, x: usize, y: usize, w: usize, h: usize) {
    let msg = app.message.as_deref().unwrap_or("");
    // Paragraph centering: (w/2) - (len/2), clipped to the area width.
    let put = |grid: &mut Grid, row: usize, s: &str, fg: Color| {
        let off = (w / 2).saturating_sub(s.len() / 2);
        let visible = s.len().min(w.saturating_sub(off));
        grid.text(x + off, row, &s.as_bytes()[..visible], fg, Color::Reset, false);
    };
    if h >= FOOTER_H {
        put(grid, y, msg, Color::DarkRed);
        put(grid, y + 1, app.controls, BG);
    } else if h >= 1 {
        if app.message.is_some() {
            put(grid, y, msg, Color::DarkRed);
        } else {
            put(grid, y, app.controls, BG);
        }
    }
}

fn draw_board(grid: &mut Grid, app: &App, rx: usize, ry: usize, rw: usize, rh: usize) {
    let inner_w = rw.min(BOARD_W);
    let inner_h = rh.min(BOARD_H);
    let bx = rx + center(rw, BOARD_W);
    let by = ry + center(rh, BOARD_H);

    let rows = stack(MAX_GUESSES, CELL_H, inner_h);
    let cols = stack(WORD_LEN, CELL_W, inner_w);

    for word_idx in 0..MAX_GUESSES {
        let (rs, rhh) = rows[word_idx];
        for cell_idx in 0..WORD_LEN {
            let (cs, cw) = cols[cell_idx];
            let (letter, status) = if word_idx < app.input_idx {
                let (word, result) = &app.history[word_idx];
                (word[cell_idx], CellColor::Submitted(result[cell_idx]))
            } else if word_idx == app.input_idx
                && app.phase == Phase::Playing
                && cell_idx < app.input_len
            {
                (app.history[app.input_idx].0[cell_idx], CellColor::Draft)
            } else {
                (0u8, CellColor::Empty)
            };
            draw_cell(grid, bx + cs, by + rs, cw, rhh, letter, status);
        }
    }
}

fn keyboard_letter_states(app: &App) -> [Option<LetterState>; 26] {
    let mut states = [None; 26];
    for (word, result) in &app.history[..app.input_idx] {
        for (&letter, &state) in word.iter().zip(result.iter()) {
            let idx = (letter - b'a') as usize;
            states[idx] = Some(match states[idx] {
                Some(existing) => better_state(existing, state),
                None => state,
            });
        }
    }
    states
}

fn better_state(a: LetterState, b: LetterState) -> LetterState {
    use LetterState::*;
    match (a, b) {
        (CorrectSpot, _) | (_, CorrectSpot) => CorrectSpot,
        (WrongSpot, _) | (_, WrongSpot) => WrongSpot,
        _ => NotInAnySpot,
    }
}

fn draw_keyboard(grid: &mut Grid, app: &App, rx: usize, ry: usize, rw: usize, rh: usize) {
    let states = keyboard_letter_states(app);
    let kw = rw.min(KB_W);
    let kh = rh.min(KB_H);
    let kx = rx + center(rw, KB_W);
    let ky = ry + center(rh, KB_H);

    for (row_idx, row) in KEYBOARD_ROWS.iter().enumerate() {
        if row_idx * (KEY_H + 1) >= kh {
            break;
        }
        let n = row.len();
        let before = center(kw, n * (KEY_W + 1) - 1);
        for (j, letter) in row.bytes().enumerate() {
            let idx = (letter - b'a') as usize;
            let color = match states[idx] {
                Some(state) => CellColor::Submitted(state),
                None => CellColor::Draft,
            };
            draw_cell(grid, kx + before + j * (KEY_W + 1), ky + row_idx * (KEY_H + 1), KEY_W, 1, letter, color);
        }
    }
}

// Split a keyboard column (horizontal layout) into title / keyboard / footer rows.
fn keyboard_column(grid: &mut Grid, app: &App, rx: usize, ry: usize, rw: usize, rh: usize) {
    let a = rh.min(BOARD_H);
    let area_y = ry + center(rh, BOARD_H);
    let mut starts = [0usize; 3];
    stack_sizes(&[TITLE_H, KB_H, FOOTER_H], a, &mut starts);
    draw_title(grid, rx, area_y + starts[0], rw);
    draw_keyboard(grid, app, rx, area_y + starts[1], rw, KB_H);
    draw_footer(grid, app, rx, area_y + starts[2], rw, FOOTER_H);
}

// ----------------------------------------------------------------------------
// Top-level layout
// ----------------------------------------------------------------------------

pub fn build_grid(w: usize, h: usize, app: &App) -> Grid {
    let mut grid = Grid::new(w, h);

    if w < MINI_W || h < MINI_H {
        // MINI_W=5, MINI_H=6 — hardcoded to avoid pulling in formatting machinery.
        const MSG: &[u8] = b"Terminal too small. Must be at least 5x6.";
        grid.text(0, 0, MSG, Color::Reset, Color::Reset, false);
        return grid;
    }

    if h >= REQ_VERT {
        // Vertical: title / board / keyboard / footer stacked, extra as gaps.
        let mut starts = [0usize; 4];
        stack_sizes(&[TITLE_H, BOARD_H, KB_H, FOOTER_H], h, &mut starts);
        draw_title(&mut grid, 0, starts[0], w);
        draw_board(&mut grid, app, 0, starts[1], w, BOARD_H);
        draw_keyboard(&mut grid, app, 0, starts[2], w, KB_H);
        draw_footer(&mut grid, app, 0, starts[3], w, FOOTER_H);
    } else {
        // Horizontal: board left, keyboard column right (SpaceEvenly with Fill margins).
        // Columns: Fill | Min(39) board | Max(23) gap | Length(39) keyboard | Fill.
        let rem = w.saturating_sub(BOARD_W + KB_W); // leftover beyond the 69 fixed cols
        let gap = rem.min(23); // Max(23) gap fills before the Fill margins
        let left = rem - gap; // remainder split evenly into the two Fill margins
        let c0 = (left + 1) / 2; // left margin (ceil)
        let board_x = c0;
        let kb_x = c0 + BOARD_W + gap;
        draw_board(&mut grid, app, board_x, 0, BOARD_W, h);
        keyboard_column(&mut grid, app, kb_x, 0, KB_W, h);
    }

    grid
}

// ----------------------------------------------------------------------------
// Terminal output
// ----------------------------------------------------------------------------

pub fn render<W: Write>(out: &mut W, grid: &Grid) -> std::io::Result<()> {
    use crossterm::cursor::MoveTo;
    use crossterm::style::{Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor};
    use crossterm::queue;

    for y in 0..grid.h {
        queue!(out, MoveTo(0, y as u16))?;
        let mut x = 0;
        while x < grid.w {
            let i = y * grid.w + x;
            let (fg, bg, bold) = (grid.fg[i], grid.bg[i], grid.bold[i]);
            let start = i;
            while x < grid.w {
                let j = y * grid.w + x;
                if grid.fg[j] != fg || grid.bg[j] != bg || grid.bold[j] != bold {
                    break;
                }
                x += 1;
            }
            queue!(
                out,
                SetAttribute(if bold { Attribute::Bold } else { Attribute::Reset }),
                SetForegroundColor(fg),
                SetBackgroundColor(bg)
            )?;
            // All glyphs are ASCII, so the raw bytes are valid UTF-8 for the terminal.
            out.write_all(&grid.ch[start..y * grid.w + x])?;
        }
    }
    queue!(out, SetAttribute(Attribute::Reset))?;
    out.flush()
}

// ----------------------------------------------------------------------------
// Test dump (glyph / bg / fg layers) matching the reference snapshot format.
// ----------------------------------------------------------------------------

#[cfg(test)]
pub fn dump_grid(w: u16, h: u16, app: &App) -> String {
    fn code(c: Color) -> char {
        match c {
            Color::Reset => '.',
            Color::White => 'w',
            Color::DarkGreen => 'G',
            Color::DarkYellow => 'Y',
            Color::Grey => 'g',
            Color::DarkGrey => 'd',
            Color::DarkRed => 'r',
            _ => '?',
        }
    }
    let grid = build_grid(w as usize, h as usize, app);
    let mut out = String::new();
    for layer in 0..3 {
        for y in 0..grid.h {
            for x in 0..grid.w {
                let i = y * grid.w + x;
                let ch = match layer {
                    0 => {
                        let b = grid.ch[i];
                        if b == 0 {
                            ' '
                        } else {
                            b as char
                        }
                    }
                    1 => code(grid.bg[i]),
                    _ => code(grid.fg[i]),
                };
                out.push(ch);
            }
            out.push('\n');
        }
        out.push_str("----\n");
    }
    out
}
