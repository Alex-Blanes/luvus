//! The bottom status line: prefix hint, key cheatsheet, and right-aligned
//! mode / pane / tab / version readout.

use super::*;

// ── status ──────────────────────────────────────────────────────────────────

pub(super) fn draw_status(f: &mut RenderTarget, area: Rect, app: &mut App, t: &Theme) {
    // Compact/touch mode collapses this row to nothing (docs/18) to reclaim it
    // for content — the status readout is keyboard-oriented and redundant on a
    // phone (the tab bar shows tabs, the switcher shows panes/nodes).
    if area.height == 0 {
        return;
    }
    f.render_widget(Block::new().style(Style::new().bg(t.crust)), area);
    let cat = app.catalog;

    // Keyboard scroll mode owns the whole status line with its own hints.
    if app.scroll_pane.is_some() {
        let mut left: Vec<Span> = vec![Span::raw(" ")];
        left.push(Span::styled(
            format!(" {} ", cat.mode_scroll),
            Style::new().fg(t.crust).bg(t.accent).bold(),
        ));
        left.push(Span::raw("  "));
        left.extend(hint("1-9", cat.scroll_jump, t));
        left.extend(hint("j/k f/b ↑↓", cat.act_scroll, t));
        left.extend(hint("g/G", cat.scroll_ends, t));
        left.extend(hint("q", cat.scroll_live, t));
        f.render_widget(Paragraph::new(Line::from(left)), area);
        return;
    }

    // Keyboard copy mode owns navigation too. Keep its hints compact because
    // the selected cells and inverse cursor are the primary affordance.
    if app.copy_mode.is_some() {
        let mut left: Vec<Span> = vec![Span::raw(" ")];
        left.push(Span::styled(
            " COPY ",
            Style::new().fg(t.crust).bg(t.accent).bold(),
        ));
        left.push(Span::raw("  "));
        left.extend(hint("hjkl arrows", "move", t));
        left.extend(hint("v", "anchor", t));
        left.extend(hint("y", "copy", t));
        left.extend(hint("q", "cancel", t));
        f.render_widget(Paragraph::new(Line::from(left)), area);
        return;
    }

    // Keyboard resize mode owns the status line with its own hint (docs/27).
    if app.mode == Mode::Resize {
        let mut left: Vec<Span> = vec![Span::raw(" ")];
        left.push(Span::styled(
            format!(" {} ", cat.mode_resize),
            Style::new().fg(t.crust).bg(t.accent).bold(),
        ));
        left.push(Span::styled(
            format!("  {}", cat.mode_resize_hint),
            Style::new().fg(t.subtext0),
        ));
        f.render_widget(Paragraph::new(Line::from(left)), area);
        return;
    }

    let prefix = app.mode == Mode::Prefix;

    let mut left: Vec<Span> = vec![Span::raw(" ")];
    // The hint keys reflect the *actual* bindings (docs/64), so they stay correct
    // after a rebind or the tmux preset (e.g. splits show `%`/`"`, not `v`/`s`).
    let k = |c: crate::app::Cmd| app.key_for(c);
    let prefix_label = app.prefix.label();
    if prefix {
        // The user just pressed the prefix — give the hints the full width (the
        // right-side readout is suppressed below) and lead with `?` so the
        // pointer to the full cheat-sheet never clips on a narrow terminal.
        left.push(Span::styled(
            format!(" {} ", cat.mode_prefix),
            Style::new().fg(t.crust).bg(t.accent).bold(),
        ));
        left.push(Span::raw("  "));
        left.extend(hint("?", cat.all_keys, t));
        left.extend(hint("←↓↑→", cat.pane, t));
        left.extend(hint(
            &format!(
                "{}/{}",
                k(crate::app::Cmd::SplitRight),
                k(crate::app::Cmd::SplitDown)
            ),
            cat.act_split,
            t,
        ));
        left.extend(hint(&k(crate::app::Cmd::ClosePane), cat.act_close, t));
        left.extend(hint(&k(crate::app::Cmd::NewTab), cat.act_new_tab, t));
        left.extend(hint(
            &format!(
                "{}/{}",
                k(crate::app::Cmd::NextTab),
                k(crate::app::Cmd::PrevTab)
            ),
            cat.act_tab,
            t,
        ));
        left.extend(hint(&k(crate::app::Cmd::NewWorkspace), cat.workspace, t));
        left.extend(hint(&k(crate::app::Cmd::OpenGit), "git", t));
        left.extend(hint(&k(crate::app::Cmd::OpenBoard), "orch", t));
        left.extend(hint(&k(crate::app::Cmd::GlobalSearch), cat.act_search, t));
    } else {
        left.push(Span::styled(
            format!(" {prefix_label} "),
            Style::new().fg(t.crust).bg(t.accent).bold(),
        ));
        left.push(Span::styled(
            format!("  {}", cat.prefix),
            Style::new().fg(t.subtext0),
        ));
        left.push(Span::styled("  ·  ", Style::new().fg(t.overlay0)));
        left.extend(hint(&format!("{prefix_label} ?"), cat.all_shortcuts, t));
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);

    // The right-side readout only shows in Normal mode; in Prefix mode the hint
    // bar owns the full width so nothing collides.
    if !prefix {
        let panes = app.layout().len();
        let (active_tab, tab_count) = {
            let ws = app.ws();
            (ws.active_tab, ws.tabs.len())
        };
        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        let version_w = crate::ui::display_width(version) as u16;
        let dot = " ●";
        let dot_w = if app.update_available.is_some() {
            crate::ui::display_width(dot) as u16
        } else {
            0
        };
        // The version is the rightmost meaningful item, followed by one padding
        // cell. It therefore remains fully visible even when a narrow status row
        // clips older metadata on the left.
        let click_w = version_w + dot_w;
        let version_rect = Rect::new(
            area.right().saturating_sub(click_w + 1),
            area.y,
            click_w.min(area.width.saturating_sub(1)),
            1,
        );
        app.version_rect = (click_w < area.width).then_some(version_rect);
        let version_fg = if app.hover.is_some_and(|(x, y)| {
            x >= version_rect.x
                && x < version_rect.right()
                && y >= version_rect.y
                && y < version_rect.bottom()
        }) {
            t.accent
        } else {
            t.subtext1
        };
        let right = Line::from(vec![
            Span::styled(cat.mode_normal, Style::new().fg(t.overlay1).bold()),
            Span::styled("  ·  ", Style::new().fg(t.overlay0)),
            Span::styled(
                format!("{panes} {}", if panes == 1 { cat.pane } else { cat.panes }),
                Style::new().fg(t.subtext0),
            ),
            Span::styled("  ·  ", Style::new().fg(t.overlay0)),
            Span::styled(
                format!("{} {}/{}", cat.act_tab, active_tab + 1, tab_count),
                Style::new().fg(t.subtext0),
            ),
            Span::styled("  ·  ", Style::new().fg(t.overlay0)),
            Span::styled(version, Style::new().fg(version_fg)),
            Span::styled(
                if app.update_available.is_some() {
                    dot
                } else {
                    ""
                },
                Style::new().fg(t.accent).bold(),
            ),
            Span::raw(" "),
        ]);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
    }
}

fn hint(key: &str, word: &str, t: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled(key.to_string(), Style::new().fg(t.accent).bold()),
        Span::styled(format!(" {word}   "), Style::new().fg(t.subtext0)),
    ]
}
