// The controller: it owns a `Game` (the pure game state) and adds everything around playing it
// that is not game data — the letters being typed but not yet submitted (`draft`), and the
// presentation chrome (`message`, `controls`). Input events land here; drawing reads from here.

use crate::game::{Game, Phase, SubmitError, MAX_GUESSES};

const PLAYING_CONTROLS: &str = "<Enter> Submit  <Esc> Quit";
const END_CONTROLS: &str = "<Enter> Restart <Esc> Quit";

// Fixed capacity for the footer message: enough for "Found in N guesses. <adjective>" with a
// multi-digit guess count and for "The word was <WORD>." at any supported WORD_LEN. Writers clamp,
// so an overflow truncates the message rather than corrupting anything.
const MSG_CAP: usize = 48;

// The one dynamic string in the game, kept in a fixed buffer instead of a `String`: these
// messages are the only dynamic text the binary produces, and the heapless buffer keeps out
// the `String::push`/`push_str` monomorphizations and the `RawVec` grow path a `String` anchors
// (the only other allocation, `ui::Grid`'s cell `Vec`, is sized exactly and never grows).
// `len == 0` doubles as "no message": no message the game produces is empty.
pub struct App {
    pub game: Game,
    msg: [u8; MSG_CAP],
    msg_len: usize,
    pub controls: &'static str,
}

// Append `s` to the message buffer, truncating at capacity.
fn push_bytes(msg: &mut [u8; MSG_CAP], len: &mut usize, s: &[u8]) {
    let n = s.len().min(MSG_CAP - *len);
    msg[*len..*len + n].copy_from_slice(&s[..n]);
    *len += n;
}

// Append `n` in decimal. A hand-rolled alternative to `write!`, which drags in the
// `fmt::Arguments`/`fmt::write` machinery. The caller only ever passes a guess count, so `n` is
// bounded by `MAX_GUESSES`: when that fits in one digit (the usual case) the whole general
// branch is dead code the compiler drops, leaving a single byte write. The `% 10` loop stays as
// a correct fallback should `MAX_GUESSES` ever exceed 9. `b'0' + d` is always in range since `d`
// is a single digit.
fn push_decimal(msg: &mut [u8; MSG_CAP], len: &mut usize, mut n: usize) {
    if MAX_GUESSES <= 9 {
        push_bytes(msg, len, &[b'0' + n as u8]);
        return;
    }
    const USIZE_MAX_DIGITS: usize = usize::MAX.ilog10() as usize + 1;
    let mut buf = [0u8; USIZE_MAX_DIGITS];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    push_bytes(msg, len, &buf[i..]);
}

impl App {
    pub fn new() -> Self {
        App {
            game: Game::new(),
            msg: [0; MSG_CAP],
            msg_len: 0,
            controls: PLAYING_CONTROLS,
        }
    }

    // The footer message currently displayed, if any.
    pub fn message(&self) -> Option<&[u8]> {
        (self.msg_len > 0).then(|| &self.msg[..self.msg_len])
    }

    pub fn phase(&self) -> Phase {
        self.game.phase()
    }

    pub fn type_letter(&mut self, letter: u8) {
        if self.game.type_letter(letter) {
            self.msg_len = 0;
        }
    }

    pub fn backspace(&mut self) {
        if self.game.backspace() {
            self.msg_len = 0;
        }
    }

    pub fn submit(&mut self) {
        if let Err(error) = self.game.submit() {
            self.msg_len = 0;
            let text: &[u8] = match error {
                SubmitError::TooShort => b"Incorrect size.",
                SubmitError::InvalidWord => b"Invalid word.",
            };
            push_bytes(&mut self.msg, &mut self.msg_len, text);
            return;
        }
        self.msg_len = 0;

        match self.game.phase() {
            Phase::Won => {
                self.controls = END_CONTROLS;
                let n = self.game.nb_guesses();
                let adjective: &[u8] = match n {
                    1 => b"Genius!",
                    2 => b"Magnificent!",
                    3 => b"Impressive!",
                    4 => b"Splendid!",
                    5 => b"Great!",
                    MAX_GUESSES => b"Phew!",
                    _ => b"Good!",
                };
                push_bytes(&mut self.msg, &mut self.msg_len, b"Found in ");
                push_decimal(&mut self.msg, &mut self.msg_len, n);
                push_bytes(&mut self.msg, &mut self.msg_len, b" guesses. ");
                push_bytes(&mut self.msg, &mut self.msg_len, adjective);
            }
            Phase::Lost => {
                self.controls = END_CONTROLS;
                push_bytes(&mut self.msg, &mut self.msg_len, b"The word was ");
                for &c in self.game.target() {
                    push_bytes(&mut self.msg, &mut self.msg_len, &[c.to_ascii_uppercase()]);
                }
                push_bytes(&mut self.msg, &mut self.msg_len, b".");
            }
            Phase::Playing => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_word(app: &mut App, word: &str) {
        for c in word.chars() {
            app.type_letter(c as u8);
        }
    }

    #[test]
    fn wrong_size_sets_message() {
        let mut app = App::new();
        type_word(&mut app, "cra");
        app.submit();
        assert!(app.message().is_some());
    }

    #[test]
    fn invalid_word_sets_message() {
        let mut app = App::new();
        type_word(&mut app, "zzzzz");
        app.submit();
        assert!(app.message().is_some());
    }
}
