#!/usr/bin/env python3
"""Generates the v3 "after" dialog mockups (inline golden format), per after-spec-v2.md v2.4:
panes get their own colours (nvim-inspired elevation scale, title chips, input row at the
bottom, split list | detail layouts, which-key grid, pmenu-style completion)."""
import os
import textwrap

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'mockups', 'dark', 'dialogs')
W = 100
# text and state colours (unchanged from v2)
TEXT, DIM = "#e5e5e7", "#989898"
TEAL, GREEN, RED, YELLOW = "#5cc8bb", "#b5bd68", "#d58181", "#f0c674"
# elevation scale (v2.4 palette, see NOTES.md)
BASE = "#14141c"      # terminal bg: transcript, status line (never written as bg)
SURFACE = "#2a2a36"   # user messages, the prompt row; also the pane input row
PANE = "#1e1e2a"      # every dialog, picker, tasks panel, completion menu
DETAIL = "#292b40"    # right column of split panes (preview sheet); v2.9: 1.19:1 vs pane, blue tint
SEL = "#233239"       # selected row inside a pane (teal-tinted)
INPUT = SURFACE       # pane input row = the prompt's colour, in the prompt's place
CHIP_FG = BASE        # dark text on accent / yellow chips


def S(text, fg=None, bold=False, italic=False):
    return (text, fg, bold, italic)


def vis(spans):
    return sum(len(t) for t, *_ in spans)


def fmt(spans, bg=None):
    out = []
    for text, fg, bold, italic in spans:
        if not text:
            continue
        attrs = [a for a in (f"fg={fg}" if fg else "", f"bg={bg}" if bg else "",
                             "bold" if bold else "", "italic" if italic else "") if a]
        out.append(f"«{';'.join(attrs)}»{text}" if attrs else f"«»{text}")
    return "".join(out)


def pad(spans, width):
    n = vis(spans)
    assert n <= width, (n, width, spans)
    return list(spans) + ([S(" " * (width - n))] if n < width else [])


def line(*segments):
    """segments: (spans, bg, width|None). A None width takes the rest of the row.
    Rows with any bg are padded to exactly W."""
    out, used = [], 0
    for i, (spans, bg, width) in enumerate(segments):
        if width is None:
            width = W - used if any(b for _, b, _ in segments) else vis(spans)
        out.append(fmt(pad(spans, width), bg))
        used += width
    assert used <= W, used
    return "".join(out) + "«»"


def plain(spans):
    return line((spans, None, None))


def pane(spans, bg=PANE):
    return line((spans, bg, None))


def sep(items, fg=DIM):
    out = []
    for i, s in enumerate(items):
        if i:
            out.append(S(" · ", DIM))
        out.append(s if isinstance(s, tuple) else S(s, fg))
    return out


def facts(*items, fg=DIM):
    return [S("  ")] + sep(list(items), fg) if items else []


# ---------------------------------------------------------------- pane parts

def title(name, cmd=None, fact_items=(), blocking=False, after=None):
    """Title chip (telescope-style): accent bg, dark bold fg; yellow when it blocks."""
    spans = [S(" "), S(f" {name} ", CHIP_FG, bold=True)]
    chip_bg = YELLOW if blocking else TEAL
    rest = []
    if cmd:
        rest.append(S(" " + cmd, DIM))
    if after:
        rest += [S("  ")] + after
    if fact_items:
        rest += facts(*fact_items)
    return (fmt([spans[0]], PANE) + fmt([spans[1]], chip_bg)
            + fmt(pad(rest, W - 1 - len(spans[1][0])), PANE) + "«»")


def input_row(placeholder="filter", typed=""):
    """Pane input row at the bottom (where the prompt was)."""
    body = [S(typed, TEXT)] if typed else [S(placeholder, DIM)]
    return pane([S(" › ", TEAL)] + body, INPUT)


def hint(*pairs, more=False):
    assert len(pairs) <= 4, pairs
    spans = [S(" ")]
    items = list(pairs) + ([("f1", "more")] if more else [])
    for i, (key, words) in enumerate(items):
        if i:
            spans.append(S(" · ", DIM))
        if key:
            spans.append(S(key, TEXT))
            if words:
                spans.append(S(" " + words, DIM))
        else:
            spans.append(S(words, DIM))
    return pane(spans)


def cells(cs, selected=False, tail=()):
    """Row cells with a 3-col gutter. On the selected row dim cells turn normal."""
    spans = [S(" › ", TEAL)] if selected else [S("   ")]
    for text, fg, width in cs:
        spans.append(S(text, TEXT if selected and fg == DIM else (fg or TEXT)))
        if width:
            assert width >= len(text), text
            spans.append(S(" " * (width - len(text))))
    if tail:
        fg = TEXT if selected else DIM
        spans += [S("  ")] + sep([t if isinstance(t, tuple) else S(t, fg) for t in tail])
    return spans


def row(cs, selected=False, tail=()):
    return pane(cells(cs, selected, tail), SEL if selected else PANE)


def split(left, right, lw):
    """List | detail. left: list of (spans, selected); right: list of spans."""
    n = max(len(left), len(right))
    left = left + [([], False)] * (n - len(left))
    right = right + [[]] * (n - len(right))
    out = []
    for (ls, sel), rs in zip(left, right):
        if isinstance(rs, tuple) and rs and rs[0] == "segs":   # nested bands in the detail
            out.append(line((ls, SEL if sel else PANE, lw), ([], PANE, 1), *rs[1]))
        else:   # v2.9: 1-col gutter in pane colour between list and detail
            out.append(line((ls, SEL if sel else PANE, lw), ([], PANE, 1), (rs, DETAIL, None)))
    return out


def kv(label, value, lw=11, vfg=TEXT):
    v = value if isinstance(value, list) else [S(value, vfg)]
    return [S(" " + label.ljust(lw - 1), DIM)] + v


def wrap(text, width, fg=DIM, indent=1):
    return [[S(" " * indent + t, fg)] for t in textwrap.wrap(text, width)]


# ---------------------------------------------------------------- frame

STATUS_RIGHT = "peek · amber-harbor · claude-haiku-5-5 · 42% context · $1.23"


def prompt_row(typed=""):
    return pane([S(" › ", TEAL)] + ([S(typed, TEXT)] if typed else []), SURFACE)


def status_line(state="idle", right=STATUS_RIGHT):
    left = {"working": [S(" ◉ ", YELLOW), S("working · 4s · esc stop", DIM)],
            "idle": [S(" ● ", GREEN), S("done · 11s · 23k tokens", DIM)],
            "blocked": [],
            # v2.9: outranks the last-turn result until a provider works; yields to working
            "signed_out": [S(" ⊗ ", RED), S("not signed in", DIM), S("  "), S("/login", TEXT)],
            }[state]
    gap = W - 1 - vis(left) - len(right)
    return plain(left + [S(" " * gap), S(right, DIM)])


def ctx_tool(tool, subject, *fact_items):
    return plain([S(" ● ", GREEN), S(tool, DIM, bold=True), S(" " + subject, DIM)]
                 + facts(*fact_items))


def ctx_prose(text):
    return plain([S(" " + text, DIM)])


DEFAULT_CTX = [ctx_tool("Edit", "src/wrap.rs", "+4 −1"),
               ctx_tool("Shell", "cargo test -p wrap", "exit 0 · 2.4s"),
               ctx_prose("Width is checked now; the two wrap tests pass.")]

REPLACING = {"permission", "question", "quit", "models", "settings", "sessions", "theme",
             "tree", "fork", "login", "scoped_models", "keys"}


def screen(name, rows, context=DEFAULT_CTX, typed="", state="idle"):
    lines = [plain([])] + list(context) + [plain([])] + rows
    if name[:-4] not in REPLACING:
        lines.append(prompt_row(typed))
    lines.append(status_line(state))
    with open(os.path.join(OUT, name), "w") as f:
        f.write("\n".join(lines) + "\n")


# ---------------------------------------------------------------- permission
screen("permission.txt", [
    title("Allow Shell?", blocking=True),
    pane([S("   "), S("$ ", DIM), S("printf 'alpha\\nbeta\\ngamma\\n' | sort -r", TEXT)]),
    pane([S("   access: pipes 2 programs", DIM)]),
    row([("1  ", DIM, None), ("yes", TEXT, None)], selected=True),
    row([("2  ", DIM, None), ("yes, always for printf this session", TEXT, None)]),
    row([("3  ", DIM, None), ("no, tell it why", TEXT, None)]),
    input_row("or type a reply"),
    hint(("1–3", "pick"), ("e", "edit first")),
], context=[ctx_prose("I'll check the order with a quick shell pipe first.")], state="blocked")

# ---------------------------------------------------------------- question
screen("question.txt", [
    title("Question", blocking=True,
          after=[S("Where should retry logic live?", TEXT, bold=True), S("  1 of 2", DIM)]),
    pane([S("   the http client is shared by every provider adapter", DIM)]),
    row([("1  ", DIM, None), ("hoocode-ai", TEXT, 16), ("unified provider client", DIM, None)],
        selected=True),
    row([("2  ", DIM, None), ("agent-core", TEXT, 16), ("runtime wrap each tool call", DIM, None)]),
    input_row("or type your own answer"),
    hint(("1–2", "pick"), ("tab", "next question"), ("esc", "skip")),
], context=[ctx_prose("Two places could own the retry logic, and the choice shapes the"),
            ctx_prose("rest of the change.")], state="blocked")

# ---------------------------------------------------------------- tasks
TASK_CTX = [ctx_tool("Read", "docs/design/tui-activity.md", "212 lines"),
            ctx_prose("The renderer is next; I'll port it and then add the goldens.")]


def task(state, text, selected=False, tail=None):
    glyph, gfg, tfg, word = {
        "done": ("●", GREEN, DIM, None),
        "working": ("◉", YELLOW, TEXT, None),
        "todo": ("○", DIM, TEXT, None),
        "failed": ("⊗", RED, TEXT, None),
        "cancelled": ("⊗", DIM, DIM, "cancelled"),
    }[state]
    spans = [S(" › ", TEAL)] if selected else [S("   ")]
    spans += [S(glyph + " ", gfg), S(text, TEXT if selected and tfg == DIM else tfg)]
    if tail:
        spans += [S("  ")] + tail
    elif word:
        spans += [S("  " + word, TEXT if selected else DIM)]
    return pane(spans, SEL if selected else PANE)


def tabs(items, current):
    return [S(t, TEXT if i == current else DIM) for i, t in enumerate(items)]


PLAN3 = [("done", "Read the migration plan"), ("working", "Port the task panel renderer"),
         ("todo", "Add golden snapshots")]
PLAN5 = PLAN3 + [("failed", "Run the parity check"), ("cancelled", "Drop the old lens tabs")]
TABS = ["plan 1/3", "agents 1/1", "mcp 1/1"]

screen("tasks.txt",
       [pane([S("   ")] + sep(tabs(TABS, 0) + [S("ctrl+g", DIM)]))]
       + [task(s, t) for s, t in PLAN3],
       context=TASK_CTX, state="working")

screen("tasks_mixed.txt", [task(s, t) for s, t in PLAN5], context=TASK_CTX, state="working")

screen("tasks_focused.txt",
       [title("Tasks", "/tasks · ctrl+g", after=sep(tabs(TABS, 0)))]
       + [task(s, t, selected=(i == 0)) for i, (s, t) in enumerate(PLAN3)]
       + [hint(("↑↓", "move"), ("←→", "tab"), ("esc", "back"))],
       context=TASK_CTX, state="working")

screen("tasks_focused_agents.txt",
       [title("Tasks", "/tasks · ctrl+g", after=sep(tabs(TABS, 1))),
        task("done", "explore  Trace the auth flow", selected=True),
        hint(("↑↓", "move"), ("←→", "tab"), ("enter", "open its log"), ("esc", "back"))],
       context=TASK_CTX, state="working")

screen("tasks_focused_shell.txt",
       [title("Tasks", "/tasks · ctrl+g", after=sep(tabs(["shell 1/3"], 0))),
        task("done", "cargo build --release", selected=True),
        task("failed", "cargo test -p nope", tail=[S("exit 101", RED)]),
        task("cancelled", "sleep 600"),
        hint(("↑↓", "move"), ("←→", "tab"), ("enter", "open output"), ("esc", "back"))],
       context=TASK_CTX, state="working")

# ---------------------------------------------------------------- quit
screen("quit.txt", [
    title("Quit?", fact_items=("2 background jobs running",)),
    row([("◉ ", YELLOW, None), ("job-1  ", DIM, None), ("sleep 60", TEXT, 11),
         ("4s of ~60s", DIM, None)]),
    row([("◉ ", YELLOW, None), ("job-2  ", DIM, None), ("sleep 120", TEXT, None)]),
    hint(("k", "stop jobs and quit"), ("l", "leave them running"), ("esc", "stay")),
])

# ---------------------------------------------------------------- models (split)
screen("models.txt", [
    title("Models", "/model", ("showing all", "2 signed in")),
] + split(
    [(cells([("mock-large", TEXT, 13), ("free", DIM, None)], True), True),
     (cells([("mock-fast", TEXT, 13), ("free", DIM, None)]), False)],
    [[S(" mock-large", TEXT, bold=True), S("  in use", DIM)],
     kv("provider", "mock"), kv("context", "128k"), kv("price", "free"),
     kv("thinking", "supported")],
    lw=36) + [
    input_row(),
    hint(("↑↓", "move"), ("enter", "pick"), ("tab", "all / scoped"), ("esc", "close")),
])

# ---------------------------------------------------------------- settings (split)
SETTINGS = [("Tools", "1 on"), ("Tool output", "peek"), ("Models", "all models"),
            ("Context", "3 settings"), ("Behavior", "6 settings"), ("Interface", "8 settings"),
            ("Plugins", "3 settings"), ("Images", "2 settings"), ("Learning", "5 settings"),
            ("Advanced", "8 settings")]
SET_DESC = ("Turn tools and tool groups (web, semantic search) on or off. Each is priced by "
            "what its schema costs per turn. Changes persist across sessions.")
screen("settings.txt", [
    title("Settings", "/settings", ("4,140 tokens per turn", "prompt 1,430", "tools 2,710")),
] + split(
    [(cells([(n, TEXT, 14), (v, DIM, None)], i == 0), i == 0)
     for i, (n, v) in enumerate(SETTINGS)],
    [[S(" Tools", TEXT, bold=True)],
     kv("current", "1 on"),
     kv("per turn", "2,710 tokens"),
     kv("", sep([S("Read 162", TEXT), S("Shell 128", TEXT)])),
     []] + wrap(SET_DESC, 60),
    lw=36) + [
    input_row(),
    hint(("↑↓", "move"), ("space", "toggle"), ("enter", "open"), ("esc", "close")),
])

# ---------------------------------------------------------------- sessions (split)
screen("sessions.txt", [
    title("Sessions", "/resume", ("2 sessions", "this folder")),
] + split(
    [(cells([("wrap width", TEXT, 47), ("56y ago", DIM, None)], True), True),
     (cells([("add a width check to wrap_text in src/wrap.rs", TEXT, 47),
             ("56y ago", DIM, None)]), False)],
    [[S(" wrap width", TEXT, bold=True)] + facts("amber-harbor", "5 messages"),
     [S(" › ", TEAL), S("what does the wrap crate do?", TEXT)],
     [S("   It wraps text greedily.", DIM)],
     [S(" › ", TEAL), S("add a width check to wrap_text", TEXT)]],
    lw=60) + [
    input_row(),
    hint(("↑↓", "move"), ("enter", "open"), ("tab", "scope"), ("esc", "close"), more=True),
])

# ---------------------------------------------------------------- theme (split, live preview)
THEMES = [("dark", "Default dark theme"), ("light", "Default light theme"),
          ("deep-space", "Deep navy dark theme"), ("daylight", "Paper-white light theme"),
          ("colorsafe-dark", "Low vision + color blindness: Okabe-Ito hues on dark, no red/green pairs"),
          ("colorsafe-light", "Low vision + color blindness: blue/violet/magenta on white, no red/green pairs")]
PW = W - 36 - 1 - 2  # preview band width inside the detail column (gutter, 1-col pad each side)


def prev(spans, bg=BASE):
    """A live-preview row: the theme's own base (or surface) inside the detail column."""
    return ("segs", [([], DETAIL, 1), (spans, bg, PW), ([], DETAIL, None)])


screen("theme.txt", [
    title("Theme", "/theme", ("6 themes",)),
] + split(
    [(cells([(n, TEXT, 17)] + ([("in use", DIM, None)] if i == 0 else []), i == 0), i == 0)
     for i, (n, _) in enumerate(THEMES)],
    [[S(" dark", TEXT, bold=True), S("  Default dark theme", DIM)],
     prev([]),   # v2.9: base row above the surface band (detail is now as light as surface)
     prev([S(" › ", TEAL), S("add a width check to wrap_text", TEXT)], SURFACE),
     prev([S(" ● ", GREEN), S("Edit", TEXT, bold=True), S(" src/wrap.rs", TEXT),
           S("  +4 ", GREEN), S("−1", RED)]),
     prev([S(" ⊗ ", RED), S("Shell", TEXT, bold=True), S(" cargo test -p wrap", TEXT),
           S("  exit 1", RED), S(" · 3.1s", DIM)]),
     prev([S(" Width is checked now; the wrap tests pass.", TEXT)])],
    lw=36) + [
    hint(("↑↓", "preview"), ("enter", "keep"), ("esc", "back")),
])

# ---------------------------------------------------------------- tree (split)
screen("tree.txt", [
    title("Tree", "/tree", ("3 entries",)),
] + split(
    [(cells([("user       ", DIM, None), ("what does the wrap crate do?", TEXT, None)]), False),
     (cells([("assistant  ", DIM, None), ("It wraps text greedily.", TEXT, None)]), False),
     (cells([("user       ", DIM, None), ("add a width check to wrap_text", TEXT, None)],
            True), True)],
    [[S(" user", TEXT, bold=True)] + facts("message 3 of 3", "you are here"),
     [S(" add a width check to wrap_text in src/wrap.rs", TEXT)]],
    lw=50) + [
    input_row(),
    hint(("↑↓", "move"), ("enter", "go here"), ("f", "fork"), ("esc", "close"), more=True),
])

# ---------------------------------------------------------------- fork
screen("fork.txt", [
    title("Fork", "/fork", ("message 2 of 3",)),
    pane([S("   The new session gets the selected message and everything above it; "
            "this one stays as it is.", DIM)]),
    row([("what does the wrap crate do?", TEXT, None)]),
    row([("add a width check to wrap_text in src/wrap.rs", TEXT, None)], selected=True,
        tail=("carries 2 messages",)),
    row([("lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor",
          DIM, None)]),
    hint(("↑↓", "move"), ("enter", "fork"), ("esc", "close")),
])

# ---------------------------------------------------------------- login
screen("login.txt", [
    title("Log in", "/login", ("3 providers",)),
    row([("Anthropic", TEXT, 11), ("configured", GREEN, None)], selected=True),
    row([("OpenAI", TEXT, 11), ("configured", GREEN, None)], tail=("env OPENAI_API_KEY",)),
    row([("Ollama", TEXT, 11), ("not configured", DIM, None)]),
    input_row(),
    hint(("↑↓", "move"), ("enter", "log in"), ("esc", "cancel")),
])

# ---------------------------------------------------------------- scoped models
screen("scoped_models.txt", [
    title("Scoped models", "/scoped-models", ("2 of 3 on", "this session only")),
    row([("claude-haiku-5-5", TEXT, 18), ("on ", GREEN, None)], selected=True,
        tail=("Claude Haiku 5.5", "anthropic", "effort low", "cheap")),
    row([("gpt-5-mini", TEXT, 18), ("on ", GREEN, None)]),
    row([("gpt-5", DIM, 18), ("off", DIM, None)]),
    input_row(),
    hint(("enter", "on/off"), ("tab", "effort"), ("alt+s", "save to settings"),
         ("esc", "close"), more=True),
])


# ---------------------------------------------------------------- slash / files (pmenu)
def pmenu(typed, token, items, name_w):
    """Completion block: pane colour, width = longest row, text aligned under the
    typed token (prompt text starts at col 3), rest of the row base colour.
    Selected row: pane selection bg + bold (catppuccin PmenuSel)."""
    col = 3 + typed.rindex(token) - 1   # block starts 1 col left of the token (inner pad)
    body = [[S(" " + n.ljust(name_w), TEXT, bold=sel), S(d + " ", TEXT if sel else DIM)]
            for n, d, sel in items]
    bw = max(vis(b) for b in body)
    return [line(([], None, col), (b, SEL if sel else PANE, bw), ([], None, W - col - bw))
            for b, (_, _, sel) in zip(body, items)]


screen("slash.txt", pmenu("/sessio", "/sessio",
                          [("/session", "session info and stats", True)], 10),
       typed="/sessio")
screen("files.txt", pmenu("look at @wr", "@wr",
                          [("wrap.rs", "src/wrap.rs", True),
                           ("wrap_tests.rs", "tests/wrap_tests.rs", False)], 15),
       typed="look at @wr")

# ---------------------------------------------------------------- keys (which-key grid)
PROMPT_KEYS = [("ctrl+a", "Move to line start"), ("ctrl+e", "Move to line end"),
               ("ctrl+b", "Move cursor left"), ("ctrl+f", "Move cursor right"),
               ("ctrl+w", "Delete word backward"), ("ctrl+u", "Delete to line start"),
               ("ctrl+k", "Delete to line end"), ("ctrl+y", "Yank"),
               ("ctrl+c", "Clear editor"), ("ctrl+v", "Paste image from clipboard")]
TRANSCRIPT_KEYS = [("ctrl+o", "Step tool output: outline → peek → full → outline"),
                   ("ctrl+t", "Toggle thinking blocks"), ("ctrl+r", "Search the transcript"),
                   ("ctrl+n", "Attach to the last Agent's subagent run (read-only)")]
APP_KEYS = [("ctrl+g", "Show or hide the task panel"),
            ("ctrl+d", "Exit when editor is empty"), ("ctrl+z", "Suspend to background")]
# three which-key columns: 30 + 32 + 38


def kcell(k, d):
    return [S(" " + k.ljust(8), TEXT), S(d, DIM)]


def grid_line(*cols):
    widths = [30, 32, 38]
    segs, i = [], 0
    for c, span in cols:            # span: number of columns this cell covers
        w = sum(widths[i:i + span])
        segs.append((c, PANE, w))
        i += span
    return line(*segs)


krows = [grid_line(([S(" prompt", DIM, bold=True)], 3))]
for r in range(4):
    krows.append(grid_line(*[(kcell(*PROMPT_KEYS[c * 4 + r]) if c * 4 + r < 10 else [], 1)
                             for c in range(3)]))
krows.append(grid_line(([S(" transcript", DIM, bold=True)], 2), ([S(" app", DIM, bold=True)], 1)))
for r in range(4):
    krows.append(grid_line((kcell(*TRANSCRIPT_KEYS[r]), 2),
                           (kcell(*APP_KEYS[r]) if r < 3 else [], 1)))
screen("keys.txt", [title("Keys", "?", ("118 bindings", "17 shown"))] + krows
       + [input_row(), hint(("esc", "close"))])


# ---------------------------------------------------------------- first run (v2.9)
# Start screen behind; with no working provider the Log in pane opens on its own (yellow
# chip: it needs an action). Esc closes it; the status line then says how to get back.
START_HEAD = [plain([S(" hoocode v0.2.31", TEXT, bold=True)]),
              plain([S(" /work/hoocode · 1 skill · 5 agents", DIM)])]


def write(name, lines):
    with open(os.path.join(OUT, name), "w") as f:
        f.write("\n".join(lines) + "\n")


FIRST_RIGHT = "peek · amber-harbor"   # nothing signed in: no model, context or cost yet
LOGIN_ROWS = [
    title("Log in", "/login", ("3 providers",), blocking=True),
    pane([S("   no model is signed in yet", DIM)]),
    row([("Anthropic", TEXT, 11), ("subscription or API key", DIM, None)], selected=True),
    row([("OpenAI", TEXT, 11), ("subscription or API key", DIM, None)]),
    row([("Ollama", TEXT, 11), ("local, no key", DIM, None)]),
    input_row(),
    hint(("enter", "log in"), ("esc", "later")),
]
# start.txt is 12 rows: head 2 + 8 blank + prompt + status; the pane takes the prompt's place
write("first_run_login.txt",
      START_HEAD + [plain([])] * (12 - 2 - len(LOGIN_ROWS) - 1) + LOGIN_ROWS
      + [status_line("blocked", FIRST_RIGHT)])
write("first_run_dismissed.txt",
      START_HEAD + [plain([])] * 8 + [prompt_row(), status_line("signed_out", FIRST_RIGHT)])
