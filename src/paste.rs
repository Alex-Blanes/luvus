//! Recovering pastes on terminals that have no bracketed paste.
//!
//! A paste normally arrives as one [`Event::Paste`]: the terminal brackets it in
//! `ESC[200~ … ESC[201~` and crossterm hands it over whole. The Windows console
//! has no such thing — crossterm reads WinAPI input records there, so a paste
//! arrives as an ordinary burst of key presses and every `\r` in it looks like
//! the user hitting Enter. Pasting a five-line prompt into an agent then submits
//! five prompts, each cut at a line break.
//!
//! What separates the two is timing, and it is not close: a paste is already
//! sitting in the console queue when we read its first character, while a typist
//! leaves milliseconds between keys and the loop drains each one long before the
//! next arrives. So a key that has more input queued *behind it the instant we
//! look* is a paste, and the run is collected into one [`Event::Paste`] — the
//! same event the Unix path produces, so nothing downstream needs to know which
//! terminal it came from.

use std::io;

use ratatui::crossterm::event::{self, Event};
#[cfg(windows)]
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// The character a key press contributes to a pasted run, or `None` if it can't
/// be part of one and must end it. Deliberately narrow: pasted text is printable
/// characters and the line breaks and tabs between them, so a chord or a function
/// key in the middle of a burst is real input that has to survive as itself.
#[cfg(windows)]
pub(crate) fn paste_char(k: &KeyEvent) -> Option<char> {
    if k.kind == KeyEventKind::Release {
        return None;
    }
    // Shift is what produces the capitals in the pasted text; the rest change what
    // the key *means*, so they can't be part of one.
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return None;
    }
    match k.code {
        KeyCode::Char(c) => Some(c),
        // `\r`, not `\n`: a terminal sends carriage returns inside a bracketed
        // paste, so this is the byte the Unix path delivers for the same paste and
        // the byte a child that never enabled bracketing expects from Enter.
        KeyCode::Enter => Some('\r'),
        KeyCode::Tab => Some('\t'),
        _ => None,
    }
}

#[cfg(windows)]
thread_local! {
    /// The one event a burst may have to look at before deciding it is over.
    /// Reading it is what ends the run, so it is parked here and handed out by
    /// the next `read` instead of being lost.
    static PARKED: std::cell::RefCell<Option<Event>> = const { std::cell::RefCell::new(None) };
}

/// Read one terminal event, turning a Windows paste burst into `Event::Paste`.
/// Everywhere else this is `crossterm::event::read` unchanged.
pub fn read() -> io::Result<Event> {
    #[cfg(windows)]
    {
        if let Some(parked) = PARKED.with(|p| p.borrow_mut().take()) {
            return Ok(parked);
        }
        let first = event::read()?;
        let Event::Key(k) = &first else {
            return Ok(first);
        };
        let Some(c) = paste_char(k) else {
            return Ok(first);
        };
        // Nothing queued behind it → someone is typing. This is the check that
        // keeps ordinary input ordinary, so it comes before anything else.
        if !event::poll(std::time::Duration::ZERO)? {
            return Ok(first);
        }
        let mut text = String::from(c);
        while event::poll(std::time::Duration::ZERO)? {
            let ev = event::read()?;
            match &ev {
                // Key-ups are interleaved through the burst and mean nothing to
                // luvus (`handle_key` drops them), so they don't end the run.
                Event::Key(k) if k.kind == KeyEventKind::Release => continue,
                Event::Key(k) => match paste_char(k) {
                    Some(c) => text.push(c),
                    None => {
                        PARKED.with(|p| *p.borrow_mut() = Some(ev));
                        break;
                    }
                },
                _ => {
                    PARKED.with(|p| *p.borrow_mut() = Some(ev));
                    break;
                }
            }
        }
        // One character after all — whatever was queued wasn't text. It stays
        // parked for the next call and the key goes through as itself.
        if text.chars().nth(1).is_none() {
            return Ok(first);
        }
        return Ok(Event::Paste(text));
    }
    #[cfg(not(windows))]
    event::read()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_pasted_run_is_text_line_breaks_and_tabs() {
        assert_eq!(paste_char(&key(KeyCode::Char('a'))), Some('a'));
        assert_eq!(
            paste_char(&KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            Some('A'),
            "shift is how the capitals in a paste arrive"
        );
        // A line break in the pasted text reaches us as Enter — the whole reason
        // a paste turns into several prompts if it is left as a key press. It goes
        // back out as a carriage return, what a bracketed paste carries on Unix.
        assert_eq!(paste_char(&key(KeyCode::Enter)), Some('\r'));
        assert_eq!(paste_char(&key(KeyCode::Tab)), Some('\t'));
    }

    #[test]
    fn a_chord_or_a_key_up_can_never_be_part_of_one() {
        assert_eq!(
            paste_char(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            None,
            "Ctrl+C in a burst is still Ctrl+C"
        );
        assert_eq!(paste_char(&key(KeyCode::F(5))), None);
        assert_eq!(paste_char(&key(KeyCode::Esc)), None);
        let mut up = key(KeyCode::Char('a'));
        up.kind = KeyEventKind::Release;
        assert_eq!(paste_char(&up), None);
    }
}
