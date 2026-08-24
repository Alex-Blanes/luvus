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

/// How long a run already known to be a paste waits for its next character
/// before deciding the paste is over.
///
/// The console does not hand a large paste over in one go: it refills its input
/// buffer in blocks, and between two blocks there is a moment with nothing
/// queued. Ending the run there chops one paste into several, each of which goes
/// to the pane wrapped in its own `ESC[200~ … ESC[201~` — and an agent CLI that
/// coalesces paste input drops most of them, which looks like a paste that lost
/// its text rather than one that arrived in pieces.
///
/// Only ever applied *after* a burst has been identified, so ordinary typing is
/// unaffected: the first character still has to have input queued behind it the
/// instant it is read. The cost of being wrong is a keystroke typed within a few
/// milliseconds of a paste joining it, against a paste that arrives intact.
#[cfg(windows)]
const BURST_GRACE: std::time::Duration = std::time::Duration::from_millis(15);

/// The character a key press contributes to a pasted run, or `None` if it can't
/// be part of one and must end it. Deliberately narrow: pasted text is printable
/// characters and the line breaks and tabs between them, so a chord or a function
/// key in the middle of a burst is real input that has to survive as itself.
#[cfg(windows)]
pub(crate) fn paste_char(k: &KeyEvent) -> Option<char> {
    if k.kind == KeyEventKind::Release {
        return None;
    }
    // Shift is what produces the capitals in the pasted text, and AltGr — which
    // Windows reports as Ctrl+Alt — is what produces `\ @ # [ ] { } | ~` on a
    // Spanish or German layout, so a pasted Windows path is full of them
    // (`super::app::keys::is_ctrl_chord`). Any *other* modifier changes what the
    // key means, so it cannot be part of a paste.
    let altgr =
        k.modifiers.contains(KeyModifiers::CONTROL) && k.modifiers.contains(KeyModifiers::ALT);
    if !altgr
        && k.modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    if k.modifiers.contains(KeyModifiers::SUPER) {
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

/// Read one terminal event. On every platform but Windows this is
/// `crossterm::event::read` unchanged — the terminal already brackets its pastes.
#[cfg(not(windows))]
pub fn read() -> io::Result<Event> {
    event::read()
}

/// Where [`read_burst`] gets its events. Exists so the burst logic can be tested
/// against a scripted console: the real one needs a person holding Ctrl+V.
#[cfg(windows)]
pub(crate) trait EventSource {
    fn poll(&mut self, timeout: std::time::Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
}

#[cfg(windows)]
struct Console;

#[cfg(windows)]
impl EventSource for Console {
    fn poll(&mut self, timeout: std::time::Duration) -> io::Result<bool> {
        event::poll(timeout)
    }
    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}

/// Read one terminal event, rebuilding a paste from the key burst the Windows
/// console delivers instead of one.
#[cfg(windows)]
pub fn read() -> io::Result<Event> {
    read_burst(&mut Console)
}

#[cfg(windows)]
pub(crate) fn read_burst<S: EventSource>(source: &mut S) -> io::Result<Event> {
    if let Some(parked) = PARKED.with(|p| p.borrow_mut().take()) {
        return Ok(parked);
    }
    let first = source.read()?;
    let Event::Key(k) = &first else {
        return Ok(first);
    };
    let Some(c) = paste_char(k) else {
        return Ok(first);
    };
    // Nothing queued behind it → someone is typing. This is the check that keeps
    // ordinary input ordinary, so it comes before anything else, and it is the
    // only one that uses a zero timeout.
    if !source.poll(std::time::Duration::ZERO)? {
        return Ok(first);
    }
    let mut text = String::from(c);
    // Past here the run is a paste, so a momentary gap in the console's input
    // buffer is a refill, not the end of it. See [`BURST_GRACE`].
    while source.poll(BURST_GRACE)? {
        let ev = source.read()?;
        match &ev {
            // Key-ups are interleaved through the burst and mean nothing to luvus
            // (`handle_key` drops them), so they don't end the run.
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
    // One character after all — whatever was queued wasn't text. It stays parked
    // for the next call and the key goes through as itself.
    match text.chars().nth(1) {
        Some(_) => Ok(Event::Paste(text)),
        None => Ok(first),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Duration;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A scripted console. `Step::Gap` is a moment with nothing queued: a zero
    /// poll sees an empty buffer, a poll that waits sees the refill.
    enum Step {
        Event(Event),
        Gap,
    }

    struct Script {
        steps: VecDeque<Step>,
        zero_polls_during_gap: usize,
    }

    impl Script {
        fn new(steps: Vec<Step>) -> Self {
            Self {
                steps: steps.into(),
                zero_polls_during_gap: 0,
            }
        }
        fn chars(text: &str) -> Vec<Step> {
            text.chars()
                .map(|c| Step::Event(Event::Key(key(KeyCode::Char(c)))))
                .collect()
        }
    }

    impl EventSource for Script {
        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            match self.steps.front() {
                None => Ok(false),
                Some(Step::Event(_)) => Ok(true),
                Some(Step::Gap) => {
                    if timeout.is_zero() {
                        self.zero_polls_during_gap += 1;
                        return Ok(false);
                    }
                    // Waiting outlasts the gap, exactly as it does against the
                    // console refilling its buffer.
                    self.steps.pop_front();
                    Ok(matches!(self.steps.front(), Some(Step::Event(_))))
                }
            }
        }
        fn read(&mut self) -> io::Result<Event> {
            match self.steps.pop_front() {
                Some(Step::Event(ev)) => Ok(ev),
                _ => Err(io::Error::other("script exhausted")),
            }
        }
    }

    fn clear_parked() {
        PARKED.with(|p| *p.borrow_mut() = None);
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

    /// AltGr arrives as Ctrl+Alt on Windows, and it is how a Spanish or German
    /// layout types `\ @ # [ ] { } | ~`. Treating it as a chord ended the run at
    /// every backslash, so a pasted Windows path lost its separators — and the
    /// text after them went out as a separate paste.
    #[test]
    fn altgr_characters_stay_inside_a_paste() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        for c in ['\\', '@', '#', '[', ']', '{', '}', '|', '~'] {
            assert_eq!(
                paste_char(&KeyEvent::new(KeyCode::Char(c), altgr)),
                Some(c),
                "AltGr+{c} is pasted text, not a chord"
            );
        }
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

    /// The console refills its input buffer in blocks, so a large paste has gaps
    /// in it. Ending the run at the first gap split one paste into several, each
    /// separately bracketed — which is how a paste arrives at an agent with most
    /// of its text missing.
    #[test]
    fn a_gap_mid_burst_does_not_end_the_paste() {
        clear_parked();
        let mut steps = Script::chars("Traza WMI");
        steps.push(Step::Gap);
        steps.extend(Script::chars(" de procesos"));
        steps.push(Step::Gap);
        steps.extend(Script::chars(", 30 s"));
        let mut script = Script::new(steps);

        match read_burst(&mut script).unwrap() {
            Event::Paste(text) => assert_eq!(text, "Traza WMI de procesos, 30 s"),
            other => panic!("expected one whole paste, got {other:?}"),
        }
        assert_eq!(
            script.zero_polls_during_gap, 0,
            "only the very first check may use a zero timeout"
        );
    }

    /// The grace period must not swallow typing. A single key with nothing behind
    /// it is a keystroke, and stays one however long we would have been willing
    /// to wait afterwards.
    #[test]
    fn a_lone_keystroke_is_never_a_paste() {
        clear_parked();
        let mut script = Script::new(vec![Step::Event(Event::Key(key(KeyCode::Char('a'))))]);
        match read_burst(&mut script).unwrap() {
            Event::Key(k) => assert_eq!(k.code, KeyCode::Char('a')),
            other => panic!("expected a keystroke, got {other:?}"),
        }
    }

    /// A chord in the middle of a burst ends the paste and survives as itself on
    /// the next read, rather than being folded into the text or dropped.
    #[test]
    fn a_chord_mid_burst_is_parked_not_swallowed() {
        clear_parked();
        let mut steps = Script::chars("hola");
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        steps.push(Step::Event(Event::Key(ctrl_c)));
        steps.extend(Script::chars("mundo"));
        let mut script = Script::new(steps);

        match read_burst(&mut script).unwrap() {
            Event::Paste(text) => assert_eq!(text, "hola"),
            other => panic!("expected the run before the chord, got {other:?}"),
        }
        match read_burst(&mut script).unwrap() {
            Event::Key(k) => assert_eq!(k, ctrl_c, "the chord itself comes next"),
            other => panic!("expected the parked chord, got {other:?}"),
        }
        match read_burst(&mut script).unwrap() {
            Event::Paste(text) => assert_eq!(text, "mundo", "and the rest still arrives"),
            other => panic!("expected the remainder, got {other:?}"),
        }
        clear_parked();
    }
}
