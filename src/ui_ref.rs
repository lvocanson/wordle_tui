//! Original ratatui-based renderer, kept ONLY as the reference oracle for the
//! crossterm rewrite. Compiled solely under the `ratatui-ref` feature (test use).
#![cfg(feature = "ratatui-ref")]

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

const KEYBOARD_ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const KEY_WIDTH: u16 = 3;
const KEY_HEIGHT: u16 = 1;
const KEYBOARD_WIDTH: u16 = 10 * KEY_WIDTH;
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
