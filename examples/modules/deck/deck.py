#!/usr/bin/env python3
"""A self-contained Markdown slide presenter for luvus (docs/53, PRES-A).

Runs as a module pane entrypoint: luvus spawns it in a real pane, it takes over
the terminal, renders one slide at a time, and pages through them with the arrow
keys / space. Pure Python stdlib, no dependencies, so the module installs with
nothing to build.

Big titles: a terminal can't change its font size, so `#` headings render as
large block-font (figlet-style) text — the deck way of making a title "big".

Supports: block-font H1 titles, headings, bold/italic/code, bullet & ordered
lists, blockquotes, fenced code / ASCII art (verbatim), horizontal rules,
Markdown tables, links, and wide glyphs (CJK/box-drawing counted at 2 cells).
Images are out — luvus renders panes as cells, so pixel graphics don't pass
through (docs/42); use ASCII art instead.

Deck resolution: $LUVUS_SETTING_FILE (default "slides.md", resolved against the
node folder $LUVUS_WORKSPACE_CWD), else the bundled sample.md.

Keys: → / space / l / j / PageDn  next   ·   ← / h / k / PageUp  prev
      g / Home  first   ·   G / End  last   ·   r  reload   ·   q  quit
Non-interactive (piped, or `--print`): dumps every slide to stdout and exits.
"""

import os
import re
import shutil
import sys
import unicodedata

# ── ANSI ────────────────────────────────────────────────────────────────────
ESC = "\x1b"
RESET = f"{ESC}[0m"
HIDE_CURSOR = f"{ESC}[?25l"
SHOW_CURSOR = f"{ESC}[?25h"
CLEAR = f"{ESC}[2J{ESC}[H"

OPTS = {"big": True, "center": True}


def sgr(style):
    """Build an SGR sequence for a style set like {'bold','code'}."""
    codes = []
    if "bold" in style:
        codes.append("1")
    if "dim" in style:
        codes.append("2")
    if "italic" in style:
        codes.append("3")
    if "underline" in style:
        codes.append("4")
    if "code" in style:
        codes.append("36")  # cyan — readable on light and dark panes
    if "h1" in style:
        codes += ["1", "35"]  # bold magenta
    if "h2" in style:
        codes += ["1", "36"]  # bold cyan
    if "h3" in style:
        codes += ["1"]
    return f"{ESC}[{';'.join(codes)}m" if codes else ""


def cwidth(s):
    """Display width in terminal cells (CJK/full-width = 2, combining = 0)."""
    w = 0
    for ch in s:
        if unicodedata.combining(ch):
            continue
        w += 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
    return w


# ── inline markdown → styled segments ────────────────────────────────────────
_INLINE = re.compile(
    r"(`[^`]+`"           # `code`
    r"|\*\*[^*]+\*\*"     # **bold**
    r"|__[^_]+__"         # __bold__
    r"|\*[^*\s][^*]*\*"   # *italic*
    r"|_[^_\s][^_]*_"     # _italic_
    r"|\[[^\]]+\]\([^)]+\))"  # [text](url)
)


def parse_inline(text, base=frozenset()):
    """Return styled segments [(text, style)] with original spacing preserved."""
    segs, pos = [], 0
    for m in _INLINE.finditer(text):
        if m.start() > pos:
            segs.append((text[pos : m.start()], base))
        tok = m.group(0)
        if tok.startswith("`"):
            segs.append((tok[1:-1], base | {"code"}))
        elif tok.startswith("**") or tok.startswith("__"):
            segs.append((tok[2:-2], base | {"bold"}))
        elif tok.startswith("[") and "](" in tok:
            segs.append((tok[1 : tok.index("]")], base | {"underline"}))
        else:
            segs.append((tok[1:-1], base | {"italic"}))
        pos = m.end()
    if pos < len(text):
        segs.append((text[pos:], base))
    return segs


def wrap_segments(segments, width):
    """Wrap styled segments into lines of runs, preserving in-word adjacency."""
    width = max(1, width)
    atoms = []
    for text, style in segments:
        for tok in re.split(r"(\s+)", text):
            if tok:
                atoms.append((tok, style, tok.isspace()))
    lines, line, line_len, pending = [], [], 0, False
    for tok, style, is_space in atoms:
        if is_space:
            pending = True
            continue
        while cwidth(tok) > width:
            if line:
                lines.append(line)
                line, line_len = [], 0
            lines.append([(tok[:width], style)])
            tok, pending = tok[width:], False
        add = cwidth(tok) + (1 if (line and pending) else 0)
        if line and line_len + add > width:
            lines.append(line)
            line, line_len, pending = [], 0, False
            add = cwidth(tok)
        if line and pending:
            line.append((" ", frozenset()))
            line_len += 1
        line.append((tok, style))
        line_len += cwidth(tok)
        pending = False
    if line:
        lines.append(line)
    return lines or [[]]


def render_runs(line):
    out = []
    for text, st in line:
        seq = sgr(st)
        out.append(f"{seq}{text}{RESET}" if seq else text)
    return "".join(out)


def vis_len(line):
    return sum(cwidth(text) for text, _ in line)


# ── block font (figlet-style big titles) ─────────────────────────────────────
# Each glyph is 5 rows tall and 5 columns wide ('#' = ink). Missing chars fall
# back to '?'. Rendered with '█', colored, centered — the deck "big font" trick.
_F = {
    "A": ["  #  ", " # # ", "#####", "#   #", "#   #"],
    "B": ["#### ", "#   #", "#### ", "#   #", "#### "],
    "C": [" ####", "#    ", "#    ", "#    ", " ####"],
    "D": ["###  ", "#  # ", "#   #", "#  # ", "###  "],
    "E": ["#####", "#    ", "#### ", "#    ", "#####"],
    "F": ["#####", "#    ", "#### ", "#    ", "#    "],
    "G": [" ####", "#    ", "#  ##", "#   #", " ####"],
    "H": ["#   #", "#   #", "#####", "#   #", "#   #"],
    "I": ["#####", "  #  ", "  #  ", "  #  ", "#####"],
    "J": ["#####", "   # ", "   # ", "#  # ", " ##  "],
    "K": ["#   #", "#  # ", "###  ", "#  # ", "#   #"],
    "L": ["#    ", "#    ", "#    ", "#    ", "#####"],
    "M": ["#   #", "## ##", "# # #", "#   #", "#   #"],
    "N": ["#   #", "##  #", "# # #", "#  ##", "#   #"],
    "O": [" ### ", "#   #", "#   #", "#   #", " ### "],
    "P": ["#### ", "#   #", "#### ", "#    ", "#    "],
    "Q": [" ### ", "#   #", "# # #", "#  # ", " ## #"],
    "R": ["#### ", "#   #", "#### ", "#  # ", "#   #"],
    "S": [" ####", "#    ", " ### ", "    #", "#### "],
    "T": ["#####", "  #  ", "  #  ", "  #  ", "  #  "],
    "U": ["#   #", "#   #", "#   #", "#   #", " ### "],
    "V": ["#   #", "#   #", "#   #", " # # ", "  #  "],
    "W": ["#   #", "#   #", "# # #", "## ##", "#   #"],
    "X": ["#   #", " # # ", "  #  ", " # # ", "#   #"],
    "Y": ["#   #", " # # ", "  #  ", "  #  ", "  #  "],
    "Z": ["#####", "   # ", "  #  ", " #   ", "#####"],
    "0": [" ### ", "#  ##", "# # #", "##  #", " ### "],
    "1": ["  #  ", " ##  ", "  #  ", "  #  ", "#####"],
    "2": [" ### ", "#   #", "  ## ", " #   ", "#####"],
    "3": ["#### ", "    #", " ### ", "    #", "#### "],
    "4": ["#  # ", "#  # ", "#####", "   # ", "   # "],
    "5": ["#####", "#    ", "#### ", "    #", "#### "],
    "6": [" ### ", "#    ", "#### ", "#   #", " ### "],
    "7": ["#####", "   # ", "  #  ", " #   ", " #   "],
    "8": [" ### ", "#   #", " ### ", "#   #", " ### "],
    "9": [" ### ", "#   #", " ####", "    #", " ### "],
    " ": ["     ", "     ", "     ", "     ", "     "],
    ".": ["     ", "     ", "     ", "     ", "  #  "],
    ",": ["     ", "     ", "     ", "  #  ", " #   "],
    "!": ["  #  ", "  #  ", "  #  ", "     ", "  #  "],
    "?": [" ### ", "#   #", "  ## ", "     ", "  #  "],
    "-": ["     ", "     ", "#####", "     ", "     "],
    ":": ["     ", "  #  ", "     ", "  #  ", "     "],
    "'": ["  #  ", "  #  ", "     ", "     ", "     "],
    "/": ["    #", "   # ", "  #  ", " #   ", "#    "],
    "&": [" ##  ", "#  # ", " ##  ", "#  ##", " ## #"],
    "(": ["  ## ", " #   ", " #   ", " #   ", "  ## "],
    ")": [" ##  ", "   # ", "   # ", "   # ", " ##  "],
}
_BIG_GAP = 3  # blank columns between words


def render_big(text, width):
    """Render a heading as block-font rows, centered. None if it can't fit."""
    words = text.upper().split()
    if not words:
        return []

    def block(word):
        rows = ["", "", "", "", ""]
        for k, ch in enumerate(word):
            g = _F.get(ch, _F["?"])
            for r in range(5):
                rows[r] += g[r] + (" " if k < len(word) - 1 else "")
        return rows

    blocks = [block(w) for w in words]
    bw = [len(b[0]) for b in blocks]
    lines, cur, curw = [], [], 0
    for i, w in enumerate(bw):
        if w > width:
            return None  # a single word won't fit → caller falls back
        add = w + (_BIG_GAP if cur else 0)
        if cur and curw + add > width:
            lines.append(cur)
            cur, curw, add = [], 0, w
        cur.append(i)
        curw += add
    if cur:
        lines.append(cur)
    out = []
    accent = f"{ESC}[1;35m"
    for lb in lines:
        total = sum(bw[i] for i in lb) + _BIG_GAP * (len(lb) - 1)
        lead = max(0, (width - total) // 2)
        for r in range(5):
            row = (" " * _BIG_GAP).join(blocks[i][r] for i in lb)
            out.append((f"{accent}{' ' * lead}{row.replace('#', '█')}{RESET}",
                        lead + len(row)))
        out.append(("", 0))
    if out and out[-1][1] == 0:
        out.pop()
    return out


# ── markdown tables ──────────────────────────────────────────────────────────
def _is_table_sep(s):
    return bool(re.match(r"^\s*\|?[\s:+-]*-[\s:|+-]*\|?\s*$", s)) and "-" in s


def render_table(rows, sep, width):
    def cells(r):
        r = r.strip()
        r = r[1:] if r.startswith("|") else r
        r = r[:-1] if r.endswith("|") else r
        return [c.strip() for c in r.split("|")]

    header = cells(rows[0])
    aligns = []
    for c in cells(sep):
        c = c.strip()
        a = "center" if c.startswith(":") and c.endswith(":") else (
            "right" if c.endswith(":") else "left")
        aligns.append(a)
    body = [cells(r) for r in rows[1:]]
    ncol = max(len(header), *(len(r) for r in body)) if body else len(header)

    def norm(r):
        return (r + [""] * ncol)[:ncol]

    header, body = norm(header), [norm(r) for r in body]
    aligns = (aligns + ["left"] * ncol)[:ncol]
    w = [cwidth(header[c]) for c in range(ncol)]
    for r in body:
        for c in range(ncol):
            w[c] = max(w[c], cwidth(r[c]))

    def pad(text, width_, align):
        gap = width_ - cwidth(text)
        if gap <= 0:
            return text
        if align == "right":
            return " " * gap + text
        if align == "center":
            left = gap // 2
            return " " * left + text + " " * (gap - left)
        return text + " " * gap

    def row(cells_, bold=False):
        parts = []
        for c in range(ncol):
            cell = pad(cells_[c], w[c], aligns[c])
            parts.append(f" {ESC}[1m{cell}{RESET} " if bold else f" {cell} ")
        return (f"{ESC}[2m│{RESET}").join(parts)

    vis = sum(w) + ncol * 2 + (ncol - 1)
    rule = f"{ESC}[2m" + "┼".join("─" * (w[c] + 2) for c in range(ncol)) + RESET
    out = [(row(header, bold=True), vis), (rule, vis)]
    out += [(row(r), vis) for r in body]
    return out


# ── block markdown → rendered ANSI lines ────────────────────────────────────
def render_slide(md, width):
    out = []

    def emit(ansi, vis):
        out.append((ansi, vis))

    lines = md.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i].rstrip()
        # fenced code / ASCII art — verbatim, not wrapped
        if re.match(r"^\s*```", line):
            i += 1
            emit("", 0)
            while i < len(lines) and not re.match(r"^\s*```", lines[i]):
                code = lines[i].rstrip("\n")
                emit(f"{ESC}[36m  {code[:width]}{RESET}", cwidth(code[:width]) + 2)
                i += 1
            i += 1
            emit("", 0)
            continue
        if not line.strip():
            emit("", 0)
            i += 1
            continue
        # table
        if "|" in line and i + 1 < len(lines) and _is_table_sep(lines[i + 1]):
            tbl = [line]
            i += 2
            while i < len(lines) and "|" in lines[i] and lines[i].strip():
                tbl.append(lines[i].rstrip())
                i += 1
            emit("", 0)
            for a, v in render_table(tbl, lines[i - len(tbl)], width):
                emit(a, v)
            emit("", 0)
            continue
        # heading
        h = re.match(r"^(#{1,6})\s+(.*)$", line)
        if h:
            level, htext = len(h.group(1)), h.group(2).strip()
            if level == 1 and OPTS["big"]:
                big = render_big(htext, width)
                if big:
                    emit("", 0)
                    for a, v in big:
                        emit(a, v)
                    emit("", 0)
                    i += 1
                    continue
            tag = "h1" if level == 1 else "h2" if level == 2 else "h3"
            text = htext.upper() if level == 1 else htext
            wrapped = wrap_segments(parse_inline(text, frozenset({tag})), width)
            emit("", 0)
            for wl in wrapped:
                emit(render_runs(wl), vis_len(wl))
            if level == 1:
                w0 = min(width, max(4, vis_len(wrapped[0])))
                emit(f"{ESC}[2m{'─' * w0}{RESET}", w0)
            i += 1
            continue
        # horizontal rule (not `---`, that's the slide split)
        if re.match(r"^\s*(\*\s*){3,}$", line) or re.match(r"^\s*(_\s*){3,}$", line):
            emit(f"{ESC}[2m{'─' * width}{RESET}", width)
            i += 1
            continue
        # blockquote
        q = re.match(r"^\s*>\s?(.*)$", line)
        if q:
            for wl in wrap_segments(parse_inline(q.group(1), frozenset({"dim"})), width - 2):
                emit(f"{ESC}[2m▏ {RESET}{render_runs(wl)}", vis_len(wl) + 2)
            i += 1
            continue
        # list item
        li = re.match(r"^(\s*)([-*+]|\d+\.)\s+(.*)$", line)
        if li:
            indent = len(li.group(1))
            marker = "•" if li.group(2) in "-*+" else li.group(2)
            head = f"{' ' * indent}{ESC}[36m{marker}{RESET} "
            hang = " " * (indent + len(marker) + 1)
            avail = max(1, width - indent - len(marker) - 1)
            for j, wl in enumerate(wrap_segments(parse_inline(li.group(3)), avail)):
                emit((head if j == 0 else hang) + render_runs(wl),
                     indent + len(marker) + 1 + vis_len(wl))
            i += 1
            continue
        # paragraph
        para = [line]
        i += 1
        while i < len(lines) and lines[i].strip() and not re.match(
            r"^\s*(#{1,6}\s|```|>\s?|[-*+]\s|\d+\.\s)", lines[i]
        ) and not ("|" in lines[i] and _is_table_sep(lines[i])):
            para.append(lines[i].rstrip())
            i += 1
        for wl in wrap_segments(parse_inline(" ".join(para)), width):
            emit(render_runs(wl), vis_len(wl))
    while out and out[0][1] == 0:
        out.pop(0)
    while out and out[-1][1] == 0:
        out.pop()
    return out


# ── deck parsing ─────────────────────────────────────────────────────────────
def parse_deck(text):
    title = ""
    lines = text.split("\n")
    if lines and lines[0].strip() == "---":
        for j in range(1, len(lines)):
            if lines[j].strip() == "---":
                for meta in lines[1:j]:
                    mt = re.match(r"^\s*title\s*:\s*(.+?)\s*$", meta)
                    if mt:
                        title = mt.group(1).strip().strip("'\"")
                lines = lines[j + 1 :]
                break
    slides, cur, fence = [], [], False
    for line in lines:
        if re.match(r"^\s*```", line):
            fence = not fence
            cur.append(line)
            continue
        if not fence and line.strip() == "---":
            slides.append("\n".join(cur))
            cur = []
            continue
        cur.append(line)
    slides.append("\n".join(cur))
    slides = [s for s in slides if s.strip()]
    if not slides:
        slides = ["# Empty deck\n\nThis file has no slides yet."]
    return title, slides


def resolve_deck_path():
    setting = os.environ.get("LUVUS_SETTING_FILE", "slides.md").strip()
    if setting:
        p = os.path.expanduser(setting)
        if os.path.isabs(p):
            if os.path.isfile(p):
                return p
        else:
            for base in (os.environ.get("LUVUS_WORKSPACE_CWD"),
                         os.environ.get("LUVUS_PANE_CWD"), os.getcwd()):
                if base and os.path.isfile(os.path.join(base, p)):
                    return os.path.join(base, p)
    here = os.environ.get("LUVUS_MODULE_ROOT") or os.path.dirname(os.path.abspath(__file__))
    sample = os.path.join(here, "sample.md")
    return sample if os.path.isfile(sample) else None


def load_deck():
    path = resolve_deck_path()
    if path and os.path.isfile(path):
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                title, slides = parse_deck(f.read())
            return (title or os.path.basename(path)), slides, path
        except OSError as e:
            return "deck", [f"# Cannot read deck\n\n`{path}`\n\n{e}"], path
    want = os.environ.get("LUVUS_SETTING_FILE", "slides.md")
    return "deck", [
        "# No deck found\n\n"
        f"Looked for `{want}` in this node's folder.\n\n"
        "Set **Deck file** in Settings then Modules then Markdown Deck."
    ], None


# ── screen ────────────────────────────────────────────────────────────────────
def term_size():
    try:
        c = shutil.get_terminal_size()
        return max(c.columns, 1), max(c.lines, 1)
    except OSError:
        return 80, 24


def draw(title, slides, idx):
    cols, rows = term_size()
    margin = 2 if cols < 60 else max(4, cols // 12)
    inner = max(10, cols - margin * 2)
    body = render_slide(slides[idx], inner)[: rows - 2]
    top = max(0, (rows - 1 - len(body)) // 2) if OPTS["center"] else 1
    buf = [CLEAR] + ["\r\n"] * top
    pad = " " * margin
    for ansi, _ in body:
        buf.append(f"\r{pad}{ansi}\r\n")
    buf.append(f"{ESC}[{rows};1H")
    counter = f"{idx + 1} / {len(slides)}"
    barw = max(0, cols - cwidth(title) - len(counter) - margin * 2 - 2)
    filled = 0 if len(slides) < 2 else round(barw * idx / (len(slides) - 1))
    bar = "─" * filled + "·" * (barw - filled)
    buf.append(f"{ESC}[2K{pad}{ESC}[2m{title}  {bar}  {counter}{RESET}")
    sys.stdout.write("".join(buf))
    sys.stdout.flush()


def print_all(title, slides):
    cols, _ = term_size()
    inner = max(20, min(cols, 80) - 4)
    print(f"# {title}  ({len(slides)} slides)\n")
    for n, s in enumerate(slides, 1):
        print(f"{ESC}[2m── slide {n} ──{RESET}")
        for ansi, _ in render_slide(s, inner):
            print("  " + ansi)
        print()


def run_interactive(title, slides):
    import signal
    import termios
    import tty

    idx, fd = 0, sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    resized = {"f": True}

    def on_winch(*_):
        resized["f"] = True

    try:
        signal.signal(signal.SIGWINCH, on_winch)
    except (ValueError, OSError):
        pass
    sys.stdout.write(HIDE_CURSOR)
    try:
        tty.setcbreak(fd)
        while True:
            if resized["f"]:
                resized["f"] = False
                draw(title, slides, idx)
            ch = sys.stdin.read(1)
            if ch == "":
                break
            key = ch
            if ch == ESC:
                seq = sys.stdin.read(1)
                if seq == "[":
                    code = sys.stdin.read(1)
                    key = {"C": "next", "D": "prev", "A": "prev", "B": "next",
                           "H": "first", "F": "last"}.get(code, "")
                    if code in "56":
                        sys.stdin.read(1)
                        key = "prev" if code == "5" else "next"
                else:
                    key = "quit"
            elif ch in (" ", "l", "j", "n"):
                key = "next"
            elif ch in ("h", "k", "p"):
                key = "prev"
            elif ch == "g":
                key = "first"
            elif ch == "G":
                key = "last"
            elif ch == "r":
                key = "reload"
            elif ch in ("q", "\x03", "\x04"):
                key = "quit"
            new = idx
            if key == "next":
                new = min(idx + 1, len(slides) - 1)
            elif key == "prev":
                new = max(idx - 1, 0)
            elif key == "first":
                new = 0
            elif key == "last":
                new = len(slides) - 1
            elif key == "reload":
                title, slides, _ = load_deck()
                idx = min(idx, len(slides) - 1)
                draw(title, slides, idx)
                continue
            elif key == "quit":
                break
            if new != idx:
                idx = new
                draw(title, slides, idx)
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old)
        sys.stdout.write(SHOW_CURSOR + RESET + CLEAR)
        sys.stdout.flush()


def _flag(name, default):
    return os.environ.get(name, str(default)).lower() not in ("false", "0", "no", "off")


def main():
    OPTS["big"] = _flag("LUVUS_SETTING_BIG_TITLES", True)
    OPTS["center"] = _flag("LUVUS_SETTING_CENTER", True)
    title, slides, _ = load_deck()
    if "--print" in sys.argv or not sys.stdout.isatty():
        print_all(title, slides)
        return
    try:
        run_interactive(title, slides)
    except KeyboardInterrupt:
        sys.stdout.write(SHOW_CURSOR + RESET)


if __name__ == "__main__":
    main()
