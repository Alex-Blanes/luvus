# Markdown Deck — a slide presenter module

Present a Markdown file as a slide deck in a luvus pane (docs/53). Pure Python
stdlib, so there is nothing to build and no external presenter to install.

Slides split on a line that is exactly `---`. Common Markdown renders inline:
headings, **bold**, *italic*, `code`, bullet/numbered lists, blockquotes, fenced
code blocks, rules, and links. An optional front-matter block sets the title:

```markdown
---
title: My talk
---

# First slide
...

---

# Second slide
...
```

## Install

```sh
luvus module link examples/modules/deck
```

(or `luvus module install <owner>/<repo>/examples/modules/deck` from GitHub).

## Present

- **Right-click a node** in the WORKSPACES sidebar → **Present as slides**, or
- `luvus module pane open example.deck present`

It opens a **tab** with the deck. Navigate:

| keys | action |
|------|--------|
| `→` `space` `l` `j` `PageDn` | next slide |
| `←` `h` `k` `PageUp` | previous slide |
| `g` / `Home`, `G` / `End` | first / last |
| `r` | reload the file from disk |
| `q` | quit |

## Choosing the deck

By default it looks for **`slides.md`** in the node's folder. Change it in
**Settings → Modules → Markdown Deck → Deck file** (a relative path is resolved
against the node folder; an absolute path is used as-is). If nothing is found it
falls back to the bundled `sample.md`, so the module works the moment you enable
it.

The presenter re-reads the file on `r`, so an agent that writes a `report.md`
while it works becomes a running report-out you page through.

## What it renders

- **Big titles.** A terminal can't change its font size, so `#` (H1) headings
  render as large **block-font** text (figlet-style, self-contained — no `figlet`
  needed), centered. Turn it off with the **Big titles** setting.
- Headings, **bold**, *italic*, `code`, bullet & ordered lists, blockquotes,
  fenced code / **ASCII art** (kept verbatim), horizontal rules, and **tables**
  (aligned columns, `:---:` alignment, bold header).
- **Any Unicode glyph** — arrows, box-drawing, symbols — passes straight through,
  and CJK / wide glyphs are measured at two cells so wrapping stays aligned.

## Notes / limits

- Text only. Images (PDF/PowerPoint, kitty-graphics) are out — luvus renders
  panes as cells, so pixel graphics do not pass through (docs/42, docs/47). Use
  ASCII art instead.
- macOS and Linux (raw-terminal input via `termios`).
- Piping or `python3 deck.py --print` dumps every slide to stdout for a quick
  look without the interactive UI.
