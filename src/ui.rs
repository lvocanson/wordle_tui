use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Clear, Padding, Paragraph},
    Frame,
};

use crate::app::{App, Phase};
use crate::game::{self, LetterState};

const GAME_BG_COLOR: Color = Color::DarkGray;
const CELL_WIDTH: u16 = 7;
const CELL_HEIGHT: u16 = 3;
const BOARD_WIDTH: u16 = game::WORD_LEN as u16 * (CELL_WIDTH + 1) - 1;
const BOARD_HEIGHT: u16 = game::MAX_GUESSES as u16 * (CELL_HEIGHT + 1) - 1;
const BOARD_MINI_WIDTH: u16 = game::WORD_LEN as u16;
const BOARD_MINI_HEIGHT: u16 = game::MAX_GUESSES as u16;
const TITLE_MAX_HEIGHT: u16 = 1;
const FOOTER_MAX_HEIGHT: u16 = 2;

// QWERTY, hardcoded (no cross-platform way to read the OS layout).
const KEYBOARD_ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const KEY_WIDTH: u16 = 3;
const KEY_HEIGHT: u16 = 1;
const KEYBOARD_WIDTH: u16 = 10 * KEY_WIDTH; // widest row (top row, 10 keys)
const KEYBOARD_HEIGHT: u16 = KEYBOARD_ROWS.len() as u16 * KEY_HEIGHT;

const REQUIRED_HEIGHT_FOR_VERTICAL_KEYBOARD: u16 =
    TITLE_MAX_HEIGHT + BOARD_HEIGHT + KEYBOARD_HEIGHT + FOOTER_MAX_HEIGHT;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    if area.width < BOARD_MINI_WIDTH || area.height < BOARD_MINI_HEIGHT {
        let too_small = Paragraph::new(format!(
            "Terminal too small. Must be at least {}x{}.",
            BOARD_MINI_WIDTH, BOARD_MINI_HEIGHT
        ));
        frame.render_widget(too_small, area);
        return;
    }

    if area.height >= REQUIRED_HEIGHT_FOR_VERTICAL_KEYBOARD {
        draw_vertical(frame, app, area);
    } else {
        draw_horizontal(frame, app, area);
    }
}

/// Title, board, keyboard and footer stacked at their ideal sizes; leftover
/// height becomes gaps between them.
fn draw_vertical(frame: &mut Frame, app: &App, area: Rect) {
    let outer = vertical_sections(area);

    draw_title(frame, outer[0]);
    draw_board(frame, app, outer[1]);
    draw_keyboard(frame, app, outer[2]);
    draw_footer(frame, app, outer[3]);
}

fn vertical_sections(area: Rect) -> [Rect; 4] {
    let rects = Layout::vertical([
        Constraint::Length(TITLE_MAX_HEIGHT),
        Constraint::Length(BOARD_HEIGHT),
        Constraint::Length(KEYBOARD_HEIGHT),
        Constraint::Length(FOOTER_MAX_HEIGHT),
    ])
    .flex(Flex::SpaceBetween)
    .split(area);
    [rects[0], rects[1], rects[2], rects[3]]
}

/// Board and keyboard side by side (board left, keyboard right); title and
/// footer wrap only around the keyboard column.
fn draw_horizontal(frame: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(BOARD_WIDTH),
        Constraint::Max((BOARD_WIDTH + KEYBOARD_WIDTH) / 3),
        Constraint::Length(KEYBOARD_WIDTH),
        Constraint::Fill(1),
    ])
    .flex(Flex::SpaceEvenly)
    .split(area);

    draw_board(frame, app, columns[1]);

    let right = keyboard_column_sections(columns[3]);

    draw_title(frame, right[0]);
    draw_keyboard(frame, app, right[1]);
    draw_footer(frame, app, right[2]);
}

fn keyboard_column_sections(area: Rect) -> [Rect; 3] {
    let area = area.centered_vertically(Constraint::Max(BOARD_HEIGHT));
    let rects = Layout::vertical([
        Constraint::Length(TITLE_MAX_HEIGHT),
        Constraint::Length(KEYBOARD_HEIGHT),
        Constraint::Length(FOOTER_MAX_HEIGHT),
    ])
    .flex(Flex::SpaceBetween)
    .split(area);
    [rects[0], rects[1], rects[2]]
}

fn draw_title(frame: &mut Frame, area: Rect) {
    let title = Line::from("WORDLE TUI").centered().bold();
    frame.render_widget(title, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let footer_lines: Vec<Line> = if area.height >= FOOTER_MAX_HEIGHT {
        vec![
            app.message.clone().unwrap_or_default().red().into(),
            app.controls.dark_gray().into(),
        ]
    } else if let Some(msg) = &app.message {
        vec![msg.clone().red().into()]
    } else {
        vec![app.controls.dark_gray().into()]
    };
    frame.render_widget(Paragraph::new(footer_lines).centered(), area);
}

enum CellColor {
    Empty,
    Draft,
    Submitted(LetterState),
}

fn draw_board(frame: &mut Frame, app: &App, area: Rect) {
    let area = area.centered(Constraint::Max(BOARD_WIDTH), Constraint::Max(BOARD_HEIGHT));
    let words_rects = Layout::vertical([Constraint::Max(CELL_HEIGHT); game::MAX_GUESSES])
        .flex(Flex::SpaceBetween)
        .split(area);

    for (word_idx, word_area) in words_rects.iter().enumerate() {
        let cols = Layout::horizontal([Constraint::Max(CELL_WIDTH); game::WORD_LEN])
            .flex(Flex::SpaceBetween)
            .split(*word_area);

        for (cell_idx, cell_area) in cols.iter().enumerate() {
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
            draw_cell(frame, *cell_area, letter, status);
        }
    }
}

/// Best state seen so far per letter a-z across all submitted guesses.
fn keyboard_letter_states(app: &App) -> [Option<LetterState>; 26] {
    let mut states: [Option<LetterState>; 26] = [None; 26];

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

/// Keys grow up to `KEY_WIDTH`x`KEY_HEIGHT`, shrink down to 1x1, no border or gap.
fn draw_keyboard(frame: &mut Frame, app: &App, area: Rect) {
    let states = keyboard_letter_states(app);
    let area = area.centered(
        Constraint::Max(KEYBOARD_WIDTH),
        Constraint::Max(KEYBOARD_HEIGHT),
    );
    let rows = Layout::vertical([Constraint::Max(KEY_HEIGHT); KEYBOARD_ROWS.len()]).split(area);

    for (row, row_area) in KEYBOARD_ROWS.iter().zip(rows.iter()) {
        let keys = Layout::horizontal(vec![Constraint::Max(KEY_WIDTH); row.len()])
            .flex(Flex::Center)
            .split(*row_area);

        for (letter, key_area) in row.bytes().zip(keys.iter()) {
            let idx = (letter - b'a') as usize;
            let color = match states[idx] {
                Some(state) => CellColor::Submitted(state),
                None => CellColor::Draft,
            };
            draw_cell(frame, *key_area, letter, color);
        }
    }
}

fn draw_cell(frame: &mut Frame, area: Rect, letter: u8, color: CellColor) {
    let (bg, fg) = match color {
        CellColor::Empty => (GAME_BG_COLOR, GAME_BG_COLOR),
        CellColor::Draft => (GAME_BG_COLOR, Color::White),
        CellColor::Submitted(letter_state) => match letter_state {
            LetterState::CorrectSpot => (Color::Green, Color::White),
            LetterState::WrongSpot => (Color::Yellow, Color::White),
            LetterState::NotInAnySpot => (Color::Gray, Color::White),
        },
    };

    let text = (letter as char).to_ascii_uppercase().to_string();
    let cell = Paragraph::new(text)
        .style(Style::new().bg(bg).fg(fg))
        .block(Block::new().padding(Padding::symmetric(
            (area.width.saturating_sub(1)) / 2,
            (area.height.saturating_sub(1)) / 2,
        )));
    frame.render_widget(cell, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_sections_are_flush_to_top_and_bottom_with_no_extra_height() {
        let area = Rect::new(0, 0, BOARD_WIDTH, REQUIRED_HEIGHT_FOR_VERTICAL_KEYBOARD);
        let s = vertical_sections(area);

        assert_eq!(s[0].y, 0);
        assert_eq!(s[0].height, TITLE_MAX_HEIGHT);
        assert_eq!(s[1].y, s[0].y + s[0].height);
        assert_eq!(s[1].height, BOARD_HEIGHT);
        assert_eq!(s[2].y, s[1].y + s[1].height);
        assert_eq!(s[2].height, KEYBOARD_HEIGHT);
        assert_eq!(s[3].y, s[2].y + s[2].height);
        assert_eq!(s[3].height, FOOTER_MAX_HEIGHT);
        assert_eq!(s[3].y + s[3].height, area.height);
    }

    #[test]
    fn vertical_sections_spread_extra_height_as_gaps_not_into_one_element() {
        let extra = 6;
        let area = Rect::new(
            0,
            0,
            BOARD_WIDTH,
            REQUIRED_HEIGHT_FOR_VERTICAL_KEYBOARD + extra,
        );
        let s = vertical_sections(area);

        // Every section keeps its ideal size — none of them absorbed the extra.
        assert_eq!(s[0].height, TITLE_MAX_HEIGHT);
        assert_eq!(s[1].height, BOARD_HEIGHT);
        assert_eq!(s[2].height, KEYBOARD_HEIGHT);
        assert_eq!(s[3].height, FOOTER_MAX_HEIGHT);

        // The extra shows up as gaps between sections instead.
        assert!(s[1].y > s[0].y + s[0].height, "gap between title and board");
        assert!(
            s[2].y > s[1].y + s[1].height,
            "gap between board and keyboard"
        );
        assert!(
            s[3].y > s[2].y + s[2].height,
            "gap between keyboard and footer"
        );
        assert_eq!(s[0].y, 0, "title stays flush to the top");
        assert_eq!(
            s[3].y + s[3].height,
            area.height,
            "footer stays flush to the bottom"
        );
    }

    #[test]
    fn keyboard_column_sections_are_flush_with_no_extra_height() {
        let min_height = TITLE_MAX_HEIGHT + KEYBOARD_HEIGHT + FOOTER_MAX_HEIGHT;
        let area = Rect::new(0, 0, KEYBOARD_WIDTH, min_height);
        let s = keyboard_column_sections(area);

        assert_eq!(s[0].y, 0);
        assert_eq!(s[1].y, s[0].y + s[0].height);
        assert_eq!(s[2].y, s[1].y + s[1].height);
        assert_eq!(s[2].y + s[2].height, area.height);
    }

    #[test]
    fn keyboard_rows_cover_the_alphabet_exactly_once() {
        let mut seen = [0u8; 26];
        for row in KEYBOARD_ROWS {
            for letter in row.bytes() {
                seen[(letter - b'a') as usize] += 1;
            }
        }
        assert!(seen.iter().all(|&count| count == 1), "{seen:?}");
    }

    fn app_with_history(entries: &[(&str, [LetterState; game::WORD_LEN])]) -> App {
        let mut app = App {
            target: b"xxxxx",
            history: [(
                [0; game::WORD_LEN],
                [LetterState::NotInAnySpot; game::WORD_LEN],
            ); game::MAX_GUESSES],
            input_idx: entries.len(),
            input_len: 0,
            phase: Phase::Playing,
            message: None,
            controls: "",
        };
        for (i, (word, result)) in entries.iter().enumerate() {
            let mut w = [0u8; game::WORD_LEN];
            w.copy_from_slice(word.as_bytes());
            app.history[i] = (w, *result);
        }
        app
    }

    #[test]
    fn untried_letters_have_no_state() {
        let app = app_with_history(&[]);
        assert_eq!(keyboard_letter_states(&app), [None; 26]);
    }

    #[test]
    fn keyboard_tracks_the_best_state_seen_per_letter() {
        use LetterState::*;
        let app = app_with_history(&[
            (
                "crane",
                [
                    CorrectSpot,
                    NotInAnySpot,
                    NotInAnySpot,
                    NotInAnySpot,
                    NotInAnySpot,
                ],
            ),
            (
                "slate",
                [
                    NotInAnySpot,
                    NotInAnySpot,
                    WrongSpot,
                    NotInAnySpot,
                    CorrectSpot,
                ],
            ),
        ]);
        let states = keyboard_letter_states(&app);

        assert_eq!(states[(b'c' - b'a') as usize], Some(CorrectSpot));
        // 'a' is NotInAnySpot in "crane" but WrongSpot in "slate" -> best wins.
        assert_eq!(states[(b'a' - b'a') as usize], Some(WrongSpot));
        assert_eq!(states[(b'e' - b'a') as usize], Some(CorrectSpot));
        assert_eq!(states[(b'z' - b'a') as usize], None);
    }
}
