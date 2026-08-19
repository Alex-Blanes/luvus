---
title: Markdown Deck
---

# Deck

Big block-font titles, right in a luvus pane.

**→** or **space** forward · **←** back · **r** reload · **q** quit

---

# Glyphs

Any Unicode glyph renders as-is:

- ✓ done   ✗ failed   ● working   ◆ blocked   ▸ next
- Arrows → ← ↑ ↓ ↩ ⇄   ·   Stars ★ ☆   ·   Marks ⚑ ⚙ ⧉ ⌘
- Box drawing: ┌─┬─┐ │ ├─┼─┤ │ └─┴─┘

Wide glyphs (CJK, 世界) are measured at two cells, so wrapping stays aligned.

---

# Tables

| Agent   | Task           | State    |
|---------|----------------|:--------:|
| claude  | OAuth module   | done     |
| codex   | API layer      | running  |
| kimi    | Integration    | queued   |

Columns align, and the header is bold. Alignment follows the `:---:` row.

---

# ASCII art

Images do not pass through a cell terminal, so use ASCII instead:

```
        ┌───────────┐        ┌───────────┐
        │  agent A  │ ─────▶ │  worktree │
        └───────────┘        └─────┬─────┘
                                   │ merge gate
                             ┌─────▼─────┐
                             │   main    │
                             └───────────┘
```

Everything inside a fenced block is kept verbatim.

---

# Code and emphasis

```
fn main() {
    println!("rendered verbatim, not wrapped");
}
```

Inline: **bold**, *italic*, `code`, and [links](https://luvus.dev) keep their
label. Blockquotes get a rule:

> Present what your agents built, without leaving the terminal.

---

# Point it at your own deck

1. Drop a `slides.md` in the node's folder, or
2. set **Deck file** in Settings → Modules → Markdown Deck

Then press **r** to reload. Turn big titles off there too, if you prefer.
