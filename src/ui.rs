use std::io::Write;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::Color;

use crate::game::{Game, LetterState, Score, MAX_GUESSES};
use crate::words::WORD_LEN;

const CELL_W: usize = 7;
const CELL_H: usize = 3;
const BOARD_H: usize = MAX_GUESSES * (CELL_H + 1) - 1; // 23

const KEYBOARD_ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const KEY_W: usize = 3;
const KB_COLS: usize = 10; // keys in the widest row
const KEY_H: usize = 1;
const KB_H: usize = KEYBOARD_ROWS.len() * (KEY_H + 1) - 1; // 5, 3 rows with 1-row gaps

const TITLE_H: usize = 1;
const FOOTER_H: usize = 2;
const REQ_VERT: usize = TITLE_H + BOARD_H + KB_H + FOOTER_H; // 31, fully-expanded layout

// Number of `Section` variants; sizes the fixed layout arrays and the gap count.
const SECTION_COUNT: usize = 4;

// Vertical budget thresholds: as the terminal grows taller, features switch on in
// priority order (most essential first). Each threshold is the cumulative height needed
// for its feature plus everything above it, expressed as a sum of the base dimensions so
// the numbers stay correct if MAX_GUESSES / CELL_H / KEY_H / … ever change.
const MIN_H_BOARD: usize = MAX_GUESSES * KEY_H; // 6: one 1-tall row per guess
const MIN_H_FOOTER_1: usize = MIN_H_BOARD + 1; // 7: message line
const MIN_H_KEYBOARD: usize = MIN_H_FOOTER_1 + KEYBOARD_ROWS.len() * KEY_H; // 10
const MIN_H_FOOTER_2: usize = MIN_H_KEYBOARD + 1; // 11: controls line
const MIN_H_BOARD_GAPS: usize = MIN_H_FOOTER_2 + (MAX_GUESSES - 1); // 16: gaps between word rows
const MIN_H_CELL_PAD: usize = MIN_H_BOARD_GAPS + MAX_GUESSES * (CELL_H - KEY_H); // 28: cells to full height
const MIN_H_TITLE: usize = MIN_H_CELL_PAD + TITLE_H; // 29
const MIN_H_KB_GAPS: usize = MIN_H_TITLE + (KEYBOARD_ROWS.len() - 1); // 31: gaps between keyboard rows

// The incremental budget must reach exactly the fully-expanded layout height.
const _: () = assert!(MIN_H_KB_GAPS == REQ_VERT);

const BTN_W: usize = 5; // full ENTER / BACK buttons flanking the bottom row
const BTN_W_MIN: usize = 3; // degraded (E) / (B) buttons

const MINI_W: usize = WORD_LEN; // narrowest terminal that can show a board
const MINI_H: usize = MAX_GUESSES; // shortest terminal that can show a board

// ----------------------------------------------------------------------------
// Grid model
// ----------------------------------------------------------------------------

// One screen cell: its glyph, colors, and the keypress a click there replays (the whole
// area of every key/button is tagged so a click routes through the same path as a keypress).
#[derive(Clone, Copy)]
struct Cell {
    character: u8,
    foreground_color: Color,
    background_color: Color,
    hit_key: Option<KeyEvent>,
}

pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
}

impl Grid {
    fn new(w: usize, h: usize) -> Self {
        let blank = Cell {
            character: b' ',
            foreground_color: Color::Reset,
            background_color: Color::Reset,
            hit_key: None,
        };
        Grid {
            width: w,
            height: h,
            cells: vec![blank; w * h],
        }
    }

    // Tag a rectangle of cells with the keypress a click there replays, for later hit-testing.
    fn hit_rect(&mut self, x: usize, y: usize, w: usize, h: usize, key: KeyEvent) {
        for dy in 0..h {
            for dx in 0..w {
                let (px, py) = (x + dx, y + dy);
                if px < self.width && py < self.height {
                    self.cells[py * self.width + px].hit_key = Some(key);
                }
            }
        }
    }

    // The keypress registered at a screen cell, if any (out-of-bounds reads as None).
    pub fn hit_test(&self, x: usize, y: usize) -> Option<KeyEvent> {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x].hit_key
        } else {
            None
        }
    }

    fn set(&mut self, x: usize, y: usize, ch: u8, fg: Color, bg: Color) {
        if x < self.width && y < self.height {
            let cell = &mut self.cells[y * self.width + x];
            cell.character = ch;
            cell.foreground_color = fg;
            cell.background_color = bg;
        }
    }

    fn text(&mut self, x: usize, y: usize, s: &[u8], fg: Color, bg: Color) {
        for (i, &b) in s.iter().enumerate() {
            self.set(x + i, y, b, fg, bg);
        }
    }
}

// Round a/b to the nearest integer (ties up). Used to split space into whole cells
// without ever losing or inventing a row/column to rounding.
fn div_round(a: usize, b: usize) -> usize {
    (2 * a + b) / (2 * b)
}

// Spread `extra` leftover rows/columns into the `n - 1` gaps between `n` items so the
// items sit flush at both ends and the gaps differ by at most one — the discrete
// equivalent of placing each item at its evenly-spaced ideal position.
fn gaps(n: usize, extra: usize) -> [usize; SECTION_COUNT] {
    let mut g = [0usize; SECTION_COUNT];
    if n <= 1 {
        return g;
    }
    let d = n - 1;
    
    // Indexed rather than `iter_mut().take(d)`: the plain loop optimizes smaller here.
    #[allow(clippy::needless_range_loop)]
    for i in 0..d {
        g[i] = div_round((i + 1) * extra, d) - div_round(i * extra, d);
    }
    g
}

// Left/top offset that centers an `inner`-wide block in `outer`, biasing the extra
// odd cell to the trailing side so a lone centered glyph never drifts left.
fn center(outer: usize, inner: usize) -> usize {
    outer.saturating_sub(inner).div_ceil(2)
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

// Place differently-sized bands top-to-bottom within `len`, writing each band's start
// row to `out`. Leftover height becomes even gaps between them (first band flush to the
// top, last flush to the bottom); if they can't all fit, they just stack from the top.
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
    Keyboard,
    Board(LetterState),
}

// Background palette (24-bit). Empty cells are near-black; a draft cell (holds a typed letter
// not yet submitted) is a lighter grey; the three submitted states are Wordle's green / yellow
// / grey.
const EMPTY_BG: Color = Color::Rgb { r: 18, g: 18, b: 19 };
const DRAFT_BG: Color = Color::Rgb { r: 18, g: 18, b: 19 };
const KEY_BG: Color = Color::Rgb { r: 129, g: 131, b: 132 };
const CORRECT_BG: Color = Color::Rgb { r: 83, g: 141, b: 78 };
const MISPLACED_BG: Color = Color::Rgb { r: 181, g: 159, b: 59 };
const ABSENT_BG: Color = Color::Rgb { r: 58, g: 58, b: 60 };

fn cell_bg_color(c: &CellColor) -> Color {
    match c {
        CellColor::Keyboard => KEY_BG,
        CellColor::Board(LetterState::Empty) => EMPTY_BG,
        CellColor::Board(LetterState::Draft) => DRAFT_BG,
        CellColor::Board(LetterState::Submitted(Score::Correct)) => CORRECT_BG,
        CellColor::Board(LetterState::Submitted(Score::Misplaced)) => MISPLACED_BG,
        CellColor::Board(LetterState::Submitted(Score::Absent)) => ABSENT_BG,
    }
}

// `letter` is a space for empty cells, which paints nothing over the background — so the
// glyph is written unconditionally. Cells are always at least 1x1 (see `col_budget`), so
// the centered position can't underflow.
fn draw_cell(grid: &mut Grid, x: usize, y: usize, w: usize, h: usize, letter: u8, c: CellColor) {
    let bg = cell_bg_color(&c);
    for dy in 0..h {
        for dx in 0..w {
            grid.set(x + dx, y + dy, b' ', bg, bg);
        }
    }
    grid.set(
        x + (w - 1) / 2,
        y + (h - 1) / 2,
        letter.to_ascii_uppercase(),
        Color::White,
        bg,
    );
}

// ----------------------------------------------------------------------------
// Widgets
// ----------------------------------------------------------------------------

fn draw_title(grid: &mut Grid, x: usize, y: usize, w: usize) {
    const TITLE: &[u8] = b"- Wordle -";
    let off = w.saturating_sub(TITLE.len()) / 2; // center the title over the whole width
    grid.text(x + off, y, TITLE, Color::Reset, Color::Reset);
}

fn draw_footer(grid: &mut Grid, message: Option<&str>, controls: &str, x: usize, y: usize, w: usize, h: usize) {
    let msg = message.unwrap_or("");
    // Center each line, and clip it to the band so an over-long message can't spill
    // past the right edge into neighbouring cells.
    let put = |grid: &mut Grid, row: usize, s: &str, fg: Color| {
        let off = (w / 2).saturating_sub(s.len() / 2);
        let visible = s.len().min(w.saturating_sub(off));
        grid.text(
            x + off,
            row,
            &s.as_bytes()[..visible],
            fg,
            Color::Reset,
        );
    };
    if h >= FOOTER_H {
        put(grid, y, msg, Color::DarkRed);
        put(grid, y + 1, controls, Color::Reset);
    } else if h >= 1 {
        if message.is_some() {
            put(grid, y, msg, Color::DarkRed);
        } else {
            put(grid, y, controls, Color::Reset);
        }
    }
}

// Vertical geometry (cell height `cell_h`, row gap `vgap`, y position) comes from the
// caller's budget; columns follow the shared `col_budget` (interior > gaps > center).
fn draw_board(grid: &mut Grid, game: &Game, y: usize, w: usize, cell_h: usize, vgap: usize) {
    let (cell_w, hgap, bx) = col_budget(w, WORD_LEN, CELL_W);

    for word_idx in 0..MAX_GUESSES {
        let guess = game.board()[word_idx];
        let ry = y + word_idx * (cell_h + vgap);
        for cell_idx in 0..WORD_LEN {
            let cx = bx + cell_idx * (cell_w + hgap);
            draw_cell(grid, cx, ry, cell_w, cell_h, guess.word[cell_idx], CellColor::Board(guess.result[cell_idx]));
        }
    }
}

// Draw one keyboard row of letters starting at (x, y), registering each key as clickable.
fn draw_keys(
    grid: &mut Grid,
    game: &Game,
    row: &str,
    x: usize,
    y: usize,
    key_w: usize,
    hgap: usize,
) {
    for (j, letter) in row.bytes().enumerate() {
        let color = match game.get_letter_state(letter) {
            None => CellColor::Keyboard,
            Some(s) => CellColor::Board(LetterState::Submitted(s)),
        };
        let key_x = x + j * (key_w + hgap);
        draw_cell(grid, key_x, y, key_w, 1, letter, color);
        let event = KeyEvent::new(KeyCode::Char(letter as char), KeyModifiers::NONE);
        grid.hit_rect(key_x, y, key_w, 1, event);
    }
}

fn draw_keyboard(grid: &mut Grid, game: &Game, y: usize, w: usize, vgap: usize) {
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
        let row_y = y + row_idx * (KEY_H + vgap);
        if row_idx == last {
            let start = kx + center(kb_w, bottom_w);
            let (enter, back): (&[u8], &[u8]) = if bw == BTN_W {
                (b"ENTER", b"BACK")
            } else {
                (b"(E)", b"(B)")
            };
            let enter_key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            let back_key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
            let letters_x = start + bw + sep;
            draw_button(grid, start, row_y, bw, enter, enter_key);
            draw_keys(grid, game, row, letters_x, row_y, key_w, hgap);
            draw_button(grid, letters_x + letters_w + sep, row_y, bw, back, back_key);
        } else {
            let row_w = row.len() * key_w + (row.len() - 1) * hgap;
            draw_keys(
                grid,
                game,
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
fn draw_button(grid: &mut Grid, x: usize, y: usize, w: usize, label: &[u8], key: KeyEvent) {
    let bg = cell_bg_color(&CellColor::Keyboard);
    for dx in 0..w {
        grid.set(x + dx, y, b' ', bg, bg);
    }
    let off = w.saturating_sub(label.len()) / 2;
    grid.text(x + off, y, label, Color::White, bg);
    grid.hit_rect(x, y, w, 1, key);
}

// ----------------------------------------------------------------------------
// Top-level layout
// ----------------------------------------------------------------------------

// The four stacked bands, top to bottom. A band is skipped entirely when the terminal
// is too short to afford it (see the priority budget below).
#[derive(Clone, Copy)]
enum Section {
    Title,
    Board,
    Keyboard,
    Footer,
}

pub fn build_grid(w: usize, h: usize, game: &Game, message: Option<&str>, controls: &str) -> Grid {
    let mut grid = Grid::new(w, h);

    // Below a usable minimum, point at the axis that's too small instead of drawing a
    // broken layout. Dimensionless messages need no formatting and stay correct whatever
    // WORD_LEN / MAX_GUESSES are.
    if w < MINI_W || h < MINI_H {
        let msg: &[u8] = if w < MINI_W {
            b"Terminal too narrow."
        } else {
            b"Terminal too short."
        };
        grid.text(0, 0, msg, Color::Reset, Color::Reset);
        return grid;
    }

    // Turn features on as height allows (see the MIN_H_* budget). Any rows left once the
    // highest affordable feature is on become the gaps spread between the sections.
    let footer_message = h >= MIN_H_FOOTER_1;
    let keyboard = h >= MIN_H_KEYBOARD;
    let footer_controls = h >= MIN_H_FOOTER_2;
    let board_gaps = h >= MIN_H_BOARD_GAPS;
    let cell_pad = h >= MIN_H_CELL_PAD;
    let title = h >= MIN_H_TITLE;
    let kb_gaps = h >= MIN_H_KB_GAPS;

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
    let footer_h = footer_message as usize + footer_controls as usize;

    // Keep only the bands that earned a slot, preserving top-to-bottom order.
    let mut sections = [Section::Title; SECTION_COUNT];
    let mut sizes = [0usize; SECTION_COUNT];
    let mut n = 0;
    for (section, size) in [
        (Section::Title, title_h),
        (Section::Board, board_h),
        (Section::Keyboard, kb_h),
        (Section::Footer, footer_h),
    ] {
        if size > 0 {
            sections[n] = section;
            sizes[n] = size;
            n += 1;
        }
    }

    // Whatever rows are left over become even gaps between the bands.
    let mut starts = [0usize; SECTION_COUNT];
    stack_sizes(&sizes[..n], h, &mut starts[..n]);

    for (&section, &y) in sections[..n].iter().zip(&starts[..n]) {
        match section {
            Section::Title => draw_title(&mut grid, 0, y, w),
            Section::Board => draw_board(&mut grid, game, y, w, cell_h, bgap),
            Section::Keyboard => draw_keyboard(&mut grid, game, y, w, kgap),
            Section::Footer => {
                draw_footer(&mut grid, message, controls, 0, y, w, footer_h)
            }
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

    for y in 0..grid.height {
        queue!(out, MoveTo(0, y as u16))?;
        let mut x = 0;
        while x < grid.width {
            let start = y * grid.width + x;
            let (fg, bg) = (grid.cells[start].foreground_color, grid.cells[start].background_color);
            while x < grid.width {
                let cell = &grid.cells[y * grid.width + x];
                if cell.foreground_color != fg || cell.background_color != bg {
                    break;
                }
                x += 1;
            }
            let end = y * grid.width + x;
            queue!(
                out,
                SetForegroundColor(fg),
                SetBackgroundColor(bg)
            )?;
            // AoS storage interleaves glyphs, so the run is emitted cell by cell rather than
            // as one contiguous slice. Bytes go through the buffered writer, not per syscall.
            // All glyphs are ASCII, so the raw bytes are valid UTF-8 for the terminal.
            for cell in &grid.cells[start..end] {
                out.write_all(&[cell.character])?;
            }
        }
    }
    queue!(out, SetAttribute(Attribute::Reset))?;
    out.flush()
}
