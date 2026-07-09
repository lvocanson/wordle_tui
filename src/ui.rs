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
const KB_COLS: usize = 10; // keys in the widest row
const KEY_H: usize = 1;
const KB_H: usize = KEYBOARD_ROWS.len() * (KEY_H + 1) - 1; // 5, 3 rows with 1-row gaps

const TITLE_H: usize = 1;
const FOOTER_H: usize = 2;
const REQ_VERT: usize = TITLE_H + BOARD_H + KB_H + FOOTER_H; // 31

// Clickable-region action codes stored in the hit map (0 = nothing). Letter keys
// register their own lowercase byte (b'a'..=b'z'), which never collides with these.
pub const ACT_BACK: u8 = 1;
pub const ACT_ENTER: u8 = 2;
const BTN_W: usize = 5; // full ENTER / BACK buttons flanking the bottom row
const BTN_W_MIN: usize = 3; // degraded (E) / (B) buttons

// Preferred startup terminal size (cols, rows): the vertical layout plus a
// 1-row gap between each of the 4 categories (SpaceBetween spreads +3 evenly).
pub const PREFERRED_SIZE: (u16, u16) = (BOARD_W as u16, (REQ_VERT + 3) as u16);
const MINI_W: usize = WORD_LEN; // 5
const MINI_H: usize = MAX_GUESSES; // 6

// The too-small message hardcodes these dimensions; keep them in sync.
const _: () = assert!(MINI_W == 5 && MINI_H == 6);

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
    hit: Vec<u8>,
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
            hit: vec![0u8; w * h],
        }
    }

    // Tag a rectangle of cells with a click action for later hit-testing.
    fn hit_rect(&mut self, x: usize, y: usize, w: usize, h: usize, action: u8) {
        for dy in 0..h {
            for dx in 0..w {
                let (px, py) = (x + dx, y + dy);
                if px < self.w && py < self.h {
                    self.hit[py * self.w + px] = action;
                }
            }
        }
    }

    // Action registered at a screen cell, or 0 if none (out-of-bounds included).
    pub fn hit_test(&self, x: usize, y: usize) -> u8 {
        if x < self.w && y < self.h {
            self.hit[y * self.w + x]
        } else {
            0
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

// Horizontal budget shared by board and keyboard, priority interior > gaps > center:
// `n` cells of full width `full` (odd) in available `w`. The cell width degrades from
// `full` down to 1 in steps of 2 (staying odd so the glyph keeps centered) as space
// shrinks; the inter-cell gap (lower priority) only appears once cells are full width.
// Returns (per-cell width, gap 0/1, offset centering the whole block).
fn col_budget(w: usize, n: usize, full: usize) -> (usize, usize, usize) {
    let mut cell = full;
    while cell > 1 && n * cell > w {
        cell -= 2;
    }
    let gap = (w >= n * full + (n - 1)) as usize;
    let content = n * cell + (n - 1) * gap;
    (cell, gap, center(w, content))
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
    Keyboard,
    Submitted(LetterState),
}

fn cell_bg_color(c: &CellColor) -> Color {
    match c {
        CellColor::Empty => Color::Rgb {
            r: 18,
            g: 18,
            b: 19,
        },
        CellColor::Draft => Color::Rgb {
            r: 18,
            g: 18,
            b: 19,
        },
        CellColor::Keyboard => Color::Rgb {
            r: 129,
            g: 131,
            b: 132,
        },
        CellColor::Submitted(s) => match s {
            LetterState::CorrectSpot => Color::Rgb {
                r: 83,
                g: 141,
                b: 78,
            },
            LetterState::WrongSpot => Color::Rgb {
                r: 181,
                g: 159,
                b: 59,
            },
            LetterState::NotInAnySpot => Color::Rgb {
                r: 58,
                g: 58,
                b: 60,
            },
        },
    }
}

fn draw_cell(grid: &mut Grid, x: usize, y: usize, w: usize, h: usize, letter: u8, c: CellColor) {
    let bg = cell_bg_color(&c);
    for dy in 0..h {
        for dx in 0..w {
            grid.set(x + dx, y + dy, b' ', bg, bg, false);
        }
    }
    if letter != 0 && w > 0 && h > 0 {
        grid.set(
            x + (w - 1) / 2,
            y + (h - 1) / 2,
            letter.to_ascii_uppercase(),
            Color::White,
            bg,
            false,
        );
    }
}

// ----------------------------------------------------------------------------
// Widgets
// ----------------------------------------------------------------------------

fn draw_title(grid: &mut Grid, x: usize, y: usize, w: usize) {
    const T: &[u8] = b"- Wordle -";
    let off = w.saturating_sub(T.len()) / 2; // Line::centered = (w-len)/2
    grid.text(x + off, y, T, Color::Reset, Color::Reset, true);
}

fn draw_footer(grid: &mut Grid, app: &App, x: usize, y: usize, w: usize, h: usize) {
    let msg = app.message.as_deref().unwrap_or("");
    // Paragraph centering: (w/2) - (len/2), clipped to the area width.
    let put = |grid: &mut Grid, row: usize, s: &str, fg: Color| {
        let off = (w / 2).saturating_sub(s.len() / 2);
        let visible = s.len().min(w.saturating_sub(off));
        grid.text(
            x + off,
            row,
            &s.as_bytes()[..visible],
            fg,
            Color::Reset,
            false,
        );
    };
    if h >= FOOTER_H {
        put(grid, y, msg, Color::DarkRed);
        put(grid, y + 1, app.controls, Color::Reset);
    } else if h >= 1 {
        if app.message.is_some() {
            put(grid, y, msg, Color::DarkRed);
        } else {
            put(grid, y, app.controls, Color::Reset);
        }
    }
}

// Vertical geometry (cell height `cell_h`, row gap `vgap`, y position) comes from the
// caller's budget; columns follow the shared `col_budget` (interior > gaps > center).
fn draw_board(grid: &mut Grid, app: &App, by: usize, w: usize, cell_h: usize, vgap: usize) {
    let (cell_w, hgap, bx) = col_budget(w, WORD_LEN, CELL_W);

    for word_idx in 0..MAX_GUESSES {
        let ry = by + word_idx * (cell_h + vgap);
        for cell_idx in 0..WORD_LEN {
            let cx = bx + cell_idx * (cell_w + hgap);
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
            draw_cell(grid, cx, ry, cell_w, cell_h, letter, status);
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

// Draw one keyboard row of letters starting at (x, y), registering each key as clickable.
fn draw_keys(
    grid: &mut Grid,
    states: &[Option<LetterState>; 26],
    row: &str,
    x: usize,
    y: usize,
    key_w: usize,
    hgap: usize,
) {
    for (j, letter) in row.bytes().enumerate() {
        let color = match states[(letter - b'a') as usize] {
            Some(state) => CellColor::Submitted(state),
            None => CellColor::Keyboard,
        };
        let key_x = x + j * (key_w + hgap);
        draw_cell(grid, key_x, y, key_w, 1, letter, color);
        grid.hit_rect(key_x, y, key_w, 1, letter);
    }
}

fn draw_keyboard(grid: &mut Grid, app: &App, ky: usize, w: usize, vgap: usize) {
    let states = keyboard_letter_states(app);
    let (key_w, hgap, _) = col_budget(w, KB_COLS, KEY_W);

    let last = KEYBOARD_ROWS.len() - 1;
    let bottom = KEYBOARD_ROWS[last];
    let letters_w = bottom.len() * key_w + (bottom.len() - 1) * hgap;

    // ENTER / BACK flank the bottom row. Keep the full 5-wide labels while the cells are
    // at full width, otherwise fall back to the 3-wide (E)/(B) — always shown, even if
    // that overflows a very narrow terminal. `sep` matches the inter-key gap.
    let sep = hgap;
    let bw = if key_w == KEY_W && 2 * BTN_W + 2 * sep + letters_w <= w {
        BTN_W
    } else {
        BTN_W_MIN
    };
    let bottom_w = 2 * bw + 2 * sep + letters_w;

    // Width the keyboard on the widest of the top row and the button-inclusive bottom row.
    let top_w = KB_COLS * key_w + (KB_COLS - 1) * hgap;
    let kb_w = top_w.max(bottom_w);
    let kx = center(w, kb_w);

    for (row_idx, row) in KEYBOARD_ROWS.iter().enumerate() {
        let row_y = ky + row_idx * (KEY_H + vgap);
        if row_idx == last {
            let start = kx + center(kb_w, bottom_w);
            let (enter, back): (&[u8], &[u8]) = if bw == BTN_W {
                (b"ENTER", b"BACK")
            } else {
                (b"(E)", b"(B)")
            };
            draw_button(grid, start, row_y, bw, enter, ACT_ENTER);
            let letters_x = start + bw + sep;
            draw_keys(grid, &states, row, letters_x, row_y, key_w, hgap);
            draw_button(grid, letters_x + letters_w + sep, row_y, bw, back, ACT_BACK);
        } else {
            let row_w = row.len() * key_w + (row.len() - 1) * hgap;
            draw_keys(
                grid,
                &states,
                row,
                kx + center(kb_w, row_w),
                row_y,
                key_w,
                hgap,
            );
        }
    }
}

// A grey labelled button that also registers its whole area as clickable.
fn draw_button(grid: &mut Grid, x: usize, y: usize, w: usize, label: &[u8], action: u8) {
    let bg = cell_bg_color(&CellColor::Keyboard);
    for dx in 0..w {
        grid.set(x + dx, y, b' ', bg, bg, false);
    }
    let off = w.saturating_sub(label.len()) / 2;
    grid.text(x + off, y, label, Color::White, bg, false);
    grid.hit_rect(x, y, w, 1, action);
}

// ----------------------------------------------------------------------------
// Top-level layout
// ----------------------------------------------------------------------------

// Enum-free section tags for the vertical stack (title/board/keyboard/footer).
const S_TITLE: u8 = 0;
const S_BOARD: u8 = 1;
const S_KEYBOARD: u8 = 2;
const S_FOOTER: u8 = 3;

pub fn build_grid(w: usize, h: usize, app: &App) -> Grid {
    let mut grid = Grid::new(w, h);

    // Priority-driven vertical budget. Each feature is enabled (high priority
    // first) only while the running total of rows it needs still fits in `h`;
    // the leftover is spread as gaps between sections (lowest priority). Column
    // widths are handled separately and are not part of this budget.
    //
    //   prio  feature                              rows   cumulative
    //   10    "too small" notice (w<5 or h<6)                fallback
    //    9    board (6 word rows, 1 tall each)        6         6
    //    8    footer, 1 line                          1         7
    //    7    keyboard (3 key rows)                   3        10
    //    6    footer, 2nd line (message + controls)   1        11
    //    5    gaps between board rows                 5        16
    //    4    cell inner padding (rows to CELL_H)    12        28
    //    3    title                                   1        29
    //    2    gaps between keyboard rows              2        31
    //    1    gaps between the four sections         rest
    if w < MINI_W || h < MINI_H {
        // MINI_W=5, MINI_H=6 — hardcoded to avoid pulling in formatting machinery.
        const MSG: &[u8] = b"Terminal too small. Must be at least 5x6.";
        grid.text(0, 0, MSG, Color::Reset, Color::Reset, false);
        return grid;
    }

    let footer1 = h >= 7;
    let keyboard = h >= 10;
    let footer2 = h >= 11;
    let board_gaps = h >= 16;
    let cell_pad = h >= 28;
    let title = h >= 29;
    let kb_gaps = h >= 31;

    let cell_h = if cell_pad { CELL_H } else { KEY_H };
    let bgap = board_gaps as usize;
    let board_h = MAX_GUESSES * cell_h + (MAX_GUESSES - 1) * bgap;

    let kgap = kb_gaps as usize;
    let kb_h = if keyboard {
        KEYBOARD_ROWS.len() * KEY_H + (KEYBOARD_ROWS.len() - 1) * kgap
    } else {
        0
    };

    let title_h = title as usize;
    let footer_h = footer1 as usize + footer2 as usize;

    // Collect the present sections in top-to-bottom order, then let stack_sizes
    // spread the remaining rows as gaps between them (Flex::SpaceBetween).
    let mut tags = [0u8; 4];
    let mut sizes = [0usize; 4];
    let mut n = 0;
    let mut push = |tag: u8, size: usize| {
        if size > 0 {
            tags[n] = tag;
            sizes[n] = size;
            n += 1;
        }
    };
    push(S_TITLE, title_h);
    push(S_BOARD, board_h);
    push(S_KEYBOARD, kb_h);
    push(S_FOOTER, footer_h);

    let mut starts = [0usize; 4];
    stack_sizes(&sizes[..n], h, &mut starts[..n]);

    for i in 0..n {
        let y = starts[i];
        match tags[i] {
            S_TITLE => draw_title(&mut grid, 0, y, w),
            S_BOARD => draw_board(&mut grid, app, y, w, cell_h, bgap),
            S_KEYBOARD => draw_keyboard(&mut grid, app, y, w, kgap),
            _ => draw_footer(&mut grid, app, 0, y, w, footer_h),
        }
    }

    grid
}

// ----------------------------------------------------------------------------
// Terminal output
// ----------------------------------------------------------------------------

pub fn render<W: Write>(out: &mut W, grid: &Grid) -> std::io::Result<()> {
    use crossterm::cursor::MoveTo;
    use crossterm::queue;
    use crossterm::style::{Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor};

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
                SetAttribute(if bold {
                    Attribute::Bold
                } else {
                    Attribute::Reset
                }),
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
