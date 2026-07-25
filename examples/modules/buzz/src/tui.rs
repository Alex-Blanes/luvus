//! The interactive chat pane (docs/39, BUZZ-2/3/4). A ratatui full-screen app
//! with an IRC-like feel: a clickable channel list, a topic bar, timestamped
//! messages with per-nick colors and word-wrap, and a compose line. All network
//! work happens on `relay::run` in a background thread, so a slow relay can never
//! stall input.
//!
//! Mouse works because bohay forwards clicks/wheel into a pane once the app asks
//! for mouse tracking (`EnableMouseCapture`).

use std::io::Stdout;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use anyhow::Result;
use nostr::prelude::*;
use std::io::Write;

use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::crossterm::{execute, terminal};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

use crate::relay::{self, Channel, ChatMsg, FromRelay, ToRelay};

type Term = Terminal<ratatui::backend::CrosstermBackend<Stdout>>;

const CHAN_WIDTH: u16 = 16;

struct App {
    self_npub: String,
    created: bool,
    channels: Vec<Channel>,
    sel: usize,
    current: Option<String>,
    msgs: Vec<ChatMsg>,
    input: String,
    status: String,
    scroll: u16, // lines scrolled up from the bottom (0 = follow live)
    to_relay: Sender<ToRelay>,
    /// Screen row of each channel row, set during draw for click hit-testing.
    chan_rows: Vec<u16>,
}

pub fn run(startup: Result<(String, Keys, String, bool)>) -> Result<()> {
    let mut term = setup()?;
    let res = match startup {
        Ok((relay, keys, npub, created)) => run_connected(&mut term, relay, keys, npub, created),
        Err(e) => error_screen(&mut term, &format!("{e:#}")),
    };
    teardown(&mut term)?;
    res
}

fn run_connected(
    term: &mut Term,
    relay: String,
    keys: Keys,
    npub: String,
    created: bool,
) -> Result<()> {
    let (to_relay_tx, to_relay_rx) = channel::<ToRelay>();
    let (from_relay_tx, from_relay_rx) = channel::<FromRelay>();
    std::thread::spawn(move || relay::run(relay, keys, from_relay_tx, to_relay_rx));

    let mut app = App {
        self_npub: npub,
        created,
        channels: Vec::new(),
        sel: 0,
        current: None,
        msgs: Vec::new(),
        input: String::new(),
        status: "connecting…".into(),
        scroll: 0,
        to_relay: to_relay_tx,
        chan_rows: Vec::new(),
    };
    let res = event_loop(term, &mut app, &from_relay_rx);
    let _ = app.to_relay.send(ToRelay::Quit);
    res
}

fn event_loop(term: &mut Term, app: &mut App, rx: &Receiver<FromRelay>) -> Result<()> {
    loop {
        while let Ok(ev) = rx.try_recv() {
            app.apply(ev);
        }
        term.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if app.on_key(k) {
                        return Ok(());
                    }
                }
                Event::Mouse(m) => app.on_mouse(m),
                _ => {}
            }
        }
    }
}

impl App {
    /// Returns true to quit.
    fn on_key(&mut self, k: event::KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => return true,
            KeyCode::Char('c') if ctrl => return true,
            KeyCode::Enter => self.send_current(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Tab => self.select(self.sel + 1),
            KeyCode::BackTab => self.select(self.sel + self.channels.len().saturating_sub(1)),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(5),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(5),
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
        false
    }

    fn on_mouse(&mut self, m: event::MouseEvent) {
        match m.kind {
            // Click a channel row in the sidebar to switch to it.
            MouseEventKind::Down(MouseButton::Left) if m.column < CHAN_WIDTH => {
                if let Some(i) = self.chan_rows.iter().position(|&y| y == m.row) {
                    self.select(i);
                }
            }
            MouseEventKind::ScrollUp => self.scroll = self.scroll.saturating_add(3),
            MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_sub(3),
            _ => {}
        }
    }

    fn apply(&mut self, ev: FromRelay) {
        match ev {
            FromRelay::Channels(cs) => {
                self.channels = cs;
                self.status = if self.channels.is_empty() {
                    "connected — no channels visible".into()
                } else {
                    format!("connected — {} channel(s)", self.channels.len())
                };
                if !self.channels.is_empty() {
                    self.select(0);
                }
            }
            FromRelay::Msg(m) => {
                if !self
                    .msgs
                    .iter()
                    .any(|x| x.ts == m.ts && x.content == m.content && x.author == m.author)
                {
                    self.msgs.push(m);
                    self.msgs.sort_by_key(|m| m.ts);
                    self.scroll = 0;
                }
            }
            FromRelay::Eose => self.status = "live".into(),
            FromRelay::Info(s) => self.status = s,
            FromRelay::Disconnected(s) => self.status = format!("disconnected: {s}"),
        }
    }

    /// Select channel `i` (wraps) and switch the subscription to it.
    fn select(&mut self, i: usize) {
        if self.channels.is_empty() {
            return;
        }
        self.sel = i % self.channels.len();
        let ch = self.channels[self.sel].clone();
        self.current = Some(ch.uuid.clone());
        self.msgs.clear();
        self.scroll = 0;
        self.status = format!("joining #{}…", ch.name);
        relay::write_current_channel(&ch.uuid); // BUZZ-5: for the share action
        let _ = self.to_relay.send(ToRelay::Switch(ch.uuid));
    }

    fn send_current(&mut self) {
        let text = self.input.trim();
        if text.is_empty() {
            return;
        }
        if let Some(ch) = self.current.clone() {
            let _ = self.to_relay.send(ToRelay::Send {
                channel: ch,
                content: text.to_string(),
            });
            self.input.clear();
        }
    }

    fn current_name(&self) -> &str {
        self.current
            .as_ref()
            .and_then(|u| self.channels.iter().find(|c| &c.uuid == u))
            .map(|c| c.name.as_str())
            .unwrap_or("—")
    }
}

// ── rendering ────────────────────────────────────────────────────────────────

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // topic bar
            Constraint::Min(1),    // channels | messages
            Constraint::Length(1), // input
            Constraint::Length(1), // hints
        ])
        .split(f.area());
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(CHAN_WIDTH), Constraint::Min(10)])
        .split(root[1]);

    draw_topbar(f, root[0], app);
    draw_channels(f, mid[0], app);
    draw_messages(f, mid[1], app);
    draw_input(f, root[2], app);
    draw_hints(f, root[3], app);
}

fn draw_topbar(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let title = format!(
        " #{}   {} msg   you: {} ",
        app.current_name(),
        app.msgs.len(),
        nick(&app.self_npub, true)
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            title,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

fn draw_channels(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    app.chan_rows.clear();
    let mut lines = vec![Line::from(Span::styled(
        "CHANNELS",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))];
    for (i, c) in app.channels.iter().enumerate() {
        app.chan_rows.push(area.y + 1 + i as u16); // header is row 0
        let sel = i == app.sel;
        let style = if sel {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(Span::styled(
            clip(&format!("#{}", c.name), (CHAN_WIDTH - 1) as usize),
            style,
        )));
    }
    if app.channels.is_empty() {
        lines.push(Line::from(Span::styled(
            "(none yet)",
            Style::default().fg(Color::DarkGray),
        )));
    }
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_messages(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let width = area.width.max(1) as usize;
    let height = area.height.max(1) as usize;

    // Flatten every message into wrapped display lines, then bottom-anchor with
    // PageUp/wheel scrollback.
    let mut all: Vec<Line> = Vec::new();
    for m in &app.msgs {
        let mine = m.author == app.self_npub;
        all.extend(message_lines(m, mine, width));
    }
    let total = all.len();
    let scroll = (app.scroll as usize).min(total.saturating_sub(1));
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(height);
    let view: Vec<Line> = all[start..end].to_vec();
    f.render_widget(Paragraph::new(view), area);
}

/// One message → styled `HH:MM  nick: text`, word-wrapped with continuation
/// lines indented to align under the text.
fn message_lines(m: &ChatMsg, mine: bool, width: usize) -> Vec<Line<'static>> {
    let time = hhmm(m.ts);
    let who = nick(&m.author, mine);
    let color = if mine {
        Color::Green
    } else {
        nick_color(&m.author)
    };
    let indent = time.chars().count() + 1 + who.chars().count() + 2; // "HH:MM " + nick + ": "
    let avail = width.saturating_sub(indent).max(8);

    let chunks = wrap(&m.content, avail);
    let mut out = Vec::new();
    for (i, chunk) in chunks.into_iter().enumerate() {
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled(format!("{time} "), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    who.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(": ", Style::default().fg(Color::DarkGray)),
                Span::raw(chunk),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::raw(chunk),
            ]));
        }
    }
    out
}

fn draw_input(f: &mut ratatui::Frame, area: Rect, app: &App) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " > ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(&app.input),
            Span::styled("▏", Style::default().fg(Color::Cyan)),
        ])),
        area,
    );
}

fn draw_hints(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let status = if app.created {
        format!(
            "new identity {} — add it to your relay if gated · {}",
            nick(&app.self_npub, true),
            app.status
        )
    } else {
        app.status.clone()
    };
    let hint = " Enter send · click/Tab channel · wheel/PgUp scroll · Esc quit";
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {status}"),
            Style::default().fg(Color::Yellow),
        ))),
        cols[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )))
        .alignment(ratatui::layout::Alignment::Right),
        cols[1],
    );
}

/// A full-screen error, instead of the pane closing instantly.
fn error_screen(term: &mut Term, msg: &str) -> Result<()> {
    loop {
        term.draw(|f| {
            let lines = vec![
                Line::from(Span::styled(
                    " Buzz — cannot start ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Red)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::raw(""),
                Line::from(Span::styled(
                    msg.to_string(),
                    Style::default().fg(Color::Yellow),
                )),
                Line::raw(""),
                Line::from(Span::styled(
                    "Fix it in Settings → Modules → Buzz, then reopen.  Esc to close.",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            f.render_widget(
                Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
                f.area(),
            );
        })?;
        if event::poll(Duration::from_millis(120))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press
                    && (k.code == KeyCode::Esc
                        || (k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL)))
                {
                    return Ok(());
                }
            }
        }
    }
}

// ── terminal setup ───────────────────────────────────────────────────────────

fn setup() -> Result<Term> {
    terminal::enable_raw_mode()?;
    let mut out = std::io::stdout();
    execute!(out, terminal::EnterAlternateScreen)?;
    // Enable ONLY button + SGR mouse tracking (1000 + 1006): clicks and the
    // wheel, never motion. crossterm's EnableMouseCapture also turns on
    // any-motion (1003), which would make bohay forward every mouse *move* into
    // the pane and redraw on each — a needless perf drain. Writing the modes
    // directly keeps events firing only on click/scroll.
    write!(out, "\x1b[?1000h\x1b[?1006h")?;
    out.flush()?;
    Ok(Terminal::new(ratatui::backend::CrosstermBackend::new(out))?)
}

fn teardown(term: &mut Term) -> Result<()> {
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b[?1000l\x1b[?1006l"); // disable the mouse modes
    let _ = out.flush();
    terminal::disable_raw_mode()?;
    execute!(term.backend_mut(), terminal::LeaveAlternateScreen)?;
    term.show_cursor()?;
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

const NICK_COLORS: &[Color] = &[
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::LightRed,
    Color::LightGreen,
    Color::LightMagenta,
];

fn nick_color(npub: &str) -> Color {
    let h = npub
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    NICK_COLORS[(h as usize) % NICK_COLORS.len()]
}

/// A short, stable handle: `you` for self, else the first 8 chars of the npub
/// body (display names from kind:0 profiles are a later addition).
fn nick(npub: &str, mine: bool) -> String {
    if mine {
        return "you".to_string();
    }
    npub.strip_prefix("npub1")
        .map(|s| s.chars().take(8).collect::<String>())
        .unwrap_or_else(|| clip(npub, 8))
}

/// `HH:MM` in UTC from a unix timestamp (no tz dependency; local time is a later
/// nicety).
fn hhmm(ts: u64) -> String {
    let s = ts % 86_400;
    format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// Word-wrap to `width`, honoring existing newlines and hard-splitting any word
/// longer than the width.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let mut line = String::new();
        let mut len = 0usize;
        for word in raw.split(' ') {
            let wlen = word.chars().count();
            if wlen > width {
                // flush, then hard-split the long word
                if len > 0 {
                    out.push(std::mem::take(&mut line));
                    len = 0;
                }
                let mut chunk = String::new();
                for ch in word.chars() {
                    chunk.push(ch);
                    if chunk.chars().count() == width {
                        out.push(std::mem::take(&mut chunk));
                    }
                }
                if !chunk.is_empty() {
                    line = chunk;
                    len = line.chars().count();
                }
            } else if len == 0 {
                line = word.to_string();
                len = wlen;
            } else if len + 1 + wlen <= width {
                line.push(' ');
                line.push_str(word);
                len += 1 + wlen;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
                len = wlen;
            }
        }
        out.push(line); // may be empty (blank line) — preserved
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn app_with(msgs: Vec<ChatMsg>, me: &str) -> (App, std::sync::mpsc::Receiver<ToRelay>) {
        let (tx, rx) = channel();
        (
            App {
                self_npub: me.to_string(),
                created: false,
                channels: vec![
                    Channel {
                        uuid: "u1".into(),
                        name: "general".into(),
                    },
                    Channel {
                        uuid: "u2".into(),
                        name: "welcome".into(),
                    },
                ],
                sel: 0,
                current: Some("u1".into()),
                msgs,
                input: "draft".into(),
                status: "live".into(),
                scroll: 0,
                to_relay: tx,
                chan_rows: Vec::new(),
            },
            rx,
        )
    }

    #[test]
    fn renders_irc_layout() {
        let me = "npub1selfselfselfselfselfselfself";
        let (mut app, _rx) = app_with(
            vec![
                ChatMsg {
                    author: "npub1otherotherother".into(),
                    content: "hi team".into(),
                    ts: 45_000,
                },
                ChatMsg {
                    author: me.into(),
                    content: "hello world".into(),
                    ts: 45_060,
                },
            ],
            me,
        );
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();

        assert!(text.contains("#general"), "channel with # prefix");
        assert!(text.contains("#welcome"), "second channel");
        assert!(text.contains("hi team"), "another user's message");
        assert!(text.contains("hello world"), "our message");
        assert!(text.contains("you"), "own messages labeled 'you'");
        assert!(text.contains("12:30"), "a HH:MM timestamp (UTC of 45000s)");
        assert!(text.contains("draft"), "the compose buffer");
        // Channel rows recorded for click hit-testing.
        assert_eq!(app.chan_rows.len(), 2, "one hit-rect per channel");
    }

    #[test]
    fn clicking_a_channel_row_switches() {
        let me = "npub1x";
        let (mut app, rx) = app_with(vec![], me);
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        // Click the *second* channel's recorded row, inside the sidebar column.
        let y = app.chan_rows[1];
        app.on_mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            app.current.as_deref(),
            Some("u2"),
            "clicked channel is selected"
        );
        assert!(matches!(rx.try_recv(), Ok(ToRelay::Switch(u)) if u == "u2"));
    }

    #[test]
    fn wrap_honors_width_and_newlines() {
        let w = wrap("hello world foo", 5);
        assert!(
            w.iter().all(|l| l.chars().count() <= 5),
            "each line within width"
        );
        assert_eq!(
            wrap("a\nb", 10),
            vec!["a".to_string(), "b".to_string()],
            "newlines split"
        );
    }
}
