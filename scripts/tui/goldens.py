#!/usr/bin/env python3
"""Level-2 screen goldens for the hoocode TUI (hoocode only, no reference app).

Forked from ``migration/tui-parity/harness.py`` with the hoocode-ts half removed.
Each scenario runs the real ``hoocode`` binary inside a fixed-size tmux terminal
against the scripted mock LLM, sends the scripted keys, and captures the screen at
each named ``snapshot`` step. The capture is normalized (``normalize.json``) and
stored as styled grid text, so a colour or attribute change shows up as a diff too.

Usage::

    goldens.py run <scenario|all>      # run, write target/tui-goldens/<scenario>/
    goldens.py check <scenario|all>    # run, diff against tests/golden/tui/<scenario>/*.txt
    goldens.py update <scenario|all>   # run, overwrite the golden files

Output per snapshot ``<snap>`` (both trees mirror each other):

* ``<snap>.txt``       the normalized styled grid (the golden content: one line per row,
                       style runs as ``«fg=...;bold»text``)
* ``<snap>.plain.txt`` the normalized plain text, for reading

``check`` exits non-zero on any difference, a missing or stale golden file, or a
scenario that fails a step. It prints a unified diff of the styled text for each
difference. ``update`` leaves a scenario's goldens alone when that scenario fails.

Tools: the scenarios and the normalizer use hoocode's tool names (Read, Shell, ...), so
the tool-name rules of ``normalize.json`` are not applied here. They exist only to map
hoocode-ts names to hoocode's for the parity harness.

Scenario files, the mock LLM and the normalizer rules are read from ``PARITY_DIR``
(``migration/tui-parity``) for now. T0.6 moves them to ``scripts/tui`` and repoints it.

Env: ``HOOCODE_BIN`` runs that binary instead of building ``target/debug/hoocode``.
"""

from __future__ import annotations

import argparse
import base64
import difflib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
# Shared with the parity harness until T0.6 moves them here.
PARITY_DIR = ROOT / "migration" / "tui-parity"
SCENARIOS = PARITY_DIR / "scenarios"
NORMALIZE = PARITY_DIR / "normalize.json"
MOCKLLM = PARITY_DIR / "mockllm.py"
OUT = ROOT / "target" / "tui-goldens"
GOLDEN = ROOT / "tests" / "golden" / "tui"
# The project config dir (`{config}`) and the default run args.
CONFIG_DIR = ".hoocode"
DEFAULT_ARGS = ["--offline", "--provider", "mock", "--model", "mock-model"]

# Scenarios that cannot run without the hoocode-ts package. They seed a package dir
# from the pinned hoocode-ts build through `symlinks` and `HOOCODE_PACKAGE_DIR`
# (the changelog notice reads its CHANGELOG.md from there). The goldens do not
# depend on hoocode-ts, so they are not run. Add a scenario here only with a reason.
EXCLUDED = {
    "changelog-command": "needs the hoocode-ts package dir (symlinks to {HOOCODE_PKG}) for its CHANGELOG.md",
    "changelog-startup": "needs the hoocode-ts package dir (symlinks to {HOOCODE_PKG}) for its CHANGELOG.md",
    "changelog-startup-collapsed": "needs the hoocode-ts package dir (symlinks to {HOOCODE_PKG}) for its CHANGELOG.md",
}

# Tool names the parity harness maps between hoocode-ts and hoocode. Used only to drop
# those rules from normalize.json (see the module docstring).
TOOL_WORDS = ["read", "bash", "edit", "write", "SearchCodebase", "SearchHooCode", "ask_options",
              "webfetch", "websearch", "Task", "TaskOutput"]

Cell = tuple[str, str]
SGR_RE = re.compile(r"\x1b\[([0-9;:]*)m")
SGR_16 = ["#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
          "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff"]


class StepError(Exception):
    pass


# ---------------------------------------------------------------------------
# Binary
# ---------------------------------------------------------------------------

_BIN: list[str] | None = None


def hoocode_cmd() -> list[str]:
    """The hoocode binary. Built once per process with the debug profile the parity
    harness uses (`cargo build` is a no-op when up to date)."""
    global _BIN
    if _BIN is None:
        if "HOOCODE_BIN" in os.environ:
            _BIN = [os.environ["HOOCODE_BIN"]]
        else:
            subprocess.run(["cargo", "build", "-q", "-p", "hoocode-code-main", "--bin", "hoocode"],
                           cwd=ROOT, check=True)
            _BIN = [str(ROOT / "target" / "debug" / "hoocode")]
    return _BIN


# ---------------------------------------------------------------------------
# tmux driver
# ---------------------------------------------------------------------------


class Tmux:
    def __init__(self, name: str, cols: int, rows: int) -> None:
        self.name = name
        self.cols = cols
        self.rows = rows
        self.socket = f"hoocode-goldens-{os.getpid()}"

    def _run(self, *args: str, check: bool = True) -> str:
        res = subprocess.run(["tmux", "-L", self.socket, *args], capture_output=True, text=True)
        if check and res.returncode != 0:
            raise RuntimeError(f"tmux {' '.join(args)} failed: {res.stderr.strip()}")
        return res.stdout

    def start(self, argv: list[str], cwd: Path, env: dict[str, str], stdout: Path | None = None) -> None:
        env_args = ["env", "-i"] + [f"{k}={v}" for k, v in sorted(env.items())]
        # The options come from the server's config file so they apply before the app
        # starts (an app that exits within milliseconds would otherwise lose the
        # "Pane is dead" line). See harness.py for the sh wrapper's reason.
        conf = Path(tempfile.gettempdir()) / f"{self.socket}.conf"
        conf.write_text("set-option -g remain-on-exit on\nset-option -g history-limit 10000\n")
        wrapped = ["sh", "-c", '"$@"; exit $?', "sh", *argv]
        if stdout is not None:
            # `stdout_jsonl` scenarios: stdout goes to a file, stderr and the exit status
            # stay on the screen.
            wrapped = ["sh", "-c", 'out="$1"; shift; "$@" > "$out"; exit $?', "sh", str(stdout), *argv]
        self._run("-f", str(conf), "new-session", "-d", "-s", self.name, "-x", str(self.cols),
                  "-y", str(self.rows), "-c", str(cwd), *env_args, *wrapped)

    def send_text(self, text: str) -> None:
        self._run("send-keys", "-t", self.name, "-l", text)

    def send_keys(self, keys: list[str]) -> None:
        for k in keys:
            self._run("send-keys", "-t", self.name, k)

    def capture(self, styled: bool = False, history: bool = False) -> str:
        args = ["capture-pane", "-p", "-t", self.name]
        if styled:
            args.append("-e")
        if history:
            args += ["-S", "-"]
        return self._run(*args)

    def dead(self) -> bool:
        out = self._run("display-message", "-p", "-t", self.name, "#{pane_dead}", check=False)
        return out.strip() == "1"

    def kill(self) -> None:
        self._run("kill-server", check=False)


# ---------------------------------------------------------------------------
# Screen model: a grid of (char, style) cells parsed from `capture-pane -e`
# ---------------------------------------------------------------------------


def xterm256(n: int) -> str:
    if n < 16:
        return SGR_16[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return "#%02x%02x%02x" % (steps[n // 36], steps[(n // 6) % 6], steps[n % 6])
    v = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (v, v, v)


FLAGS = {1: "bold", 2: "dim", 3: "italic", 4: "underline", 21: "underline", 5: "blink", 7: "inverse",
         8: "hidden", 9: "strike"}
UNFLAGS = {22: ("bold", "dim"), 23: ("italic",), 24: ("underline",), 25: ("blink",), 27: ("inverse",),
           28: ("hidden",), 29: ("strike",)}


def apply_sgr(st: dict, raw: str) -> None:
    params = [int(p) if p.isdigit() else 0 for p in re.split("[;:]", raw or "0")]
    i = 0
    while i < len(params):
        p = params[i]
        if p == 0:
            st.clear()
        elif p in FLAGS:
            st[FLAGS[p]] = True
        elif p in UNFLAGS:
            for k in UNFLAGS[p]:
                st.pop(k, None)
        elif 30 <= p <= 37:
            st["fg"] = SGR_16[p - 30]
        elif 90 <= p <= 97:
            st["fg"] = SGR_16[p - 82]
        elif 40 <= p <= 47:
            st["bg"] = SGR_16[p - 40]
        elif 100 <= p <= 107:
            st["bg"] = SGR_16[p - 92]
        elif p == 39:
            st.pop("fg", None)
        elif p == 49:
            st.pop("bg", None)
        elif p in (38, 48, 58) and i + 1 < len(params):
            key = {38: "fg", 48: "bg", 58: "ul"}[p]
            if params[i + 1] == 5 and i + 2 < len(params):
                st[key] = xterm256(params[i + 2])
                i += 2
            elif params[i + 1] == 2 and i + 4 < len(params):
                st[key] = "#%02x%02x%02x" % tuple(params[i + 2 : i + 5])
                i += 4
        i += 1


def style_key(st: dict) -> str:
    return ";".join(f"{k}={v}" if v is not True else k for k, v in sorted(st.items()))


def parse_screen(styled: str) -> list[list[Cell]]:
    st: dict = {}
    grid = []
    for raw in styled.rstrip("\n").split("\n"):
        cells: list[Cell] = []
        pos = 0
        for m in SGR_RE.finditer(raw):
            key = style_key(st)
            cells.extend((ch, key) for ch in raw[pos : m.start()])
            apply_sgr(st, m.group(1))
            pos = m.end()
        key = style_key(st)
        cells.extend((ch, key) for ch in raw[pos:])
        grid.append(cells)
    return grid


def grid_text(grid: list[list[Cell]]) -> str:
    return "\n".join("".join(ch for ch, _ in line).rstrip() for line in grid).rstrip("\n") + "\n"


def grid_styled(grid: list[list[Cell]]) -> str:
    """Canonical style serialization: one line per row, runs as «style»text."""
    rows = []
    for line in grid:
        while line and line[-1] == (" ", ""):
            line = line[:-1]
        parts, cur = [], None
        for ch, key in line:
            if key != cur:
                parts.append(f"«{key}»")
                cur = key
            parts.append(ch)
        rows.append("".join(parts))
    return "\n".join(rows).rstrip("\n") + "\n"


@dataclass
class Normalizer:
    """Line-wise regex rules applied to the cell grid, so text and style stay aligned.

    A rule's replacement cells inherit the style of the first matched cell, or the
    rule's explicit ``style`` (masks a style that is itself random, such as a
    session-colour badge)."""

    rules: list[tuple[re.Pattern, str, str | None]] = field(default_factory=list)

    @classmethod
    def load(cls, extra: list[dict] | None, subs: dict[str, str]) -> "Normalizer":
        spec = json.loads(NORMALIZE.read_text())["rules"] + (extra or [])
        rules = []
        for r in spec:
            if is_tool_rule(r["pattern"]):
                continue
            pattern = r["pattern"]
            for key, value in subs.items():
                pattern = pattern.replace("{" + key + "}", re.escape(value))
            rules.append((re.compile(pattern), r["replace"], r.get("style")))
        return cls(rules)

    def apply(self, grid: list[list[Cell]]) -> list[list[Cell]]:
        out = []
        tail_pending = False  # the row after the note may hold the note's wrapped tail
        for cells in grid:
            text = "".join(ch for ch, _ in cells)
            if tail_pending:
                tail_pending = False
                if text.strip() and NOTE_TAIL.endswith(text.strip()):
                    out.append([])
                    continue
            if NOTE.search(text):
                # hoocode's --print / --mode json stderr note (see the module docstring).
                tail_pending = True
                out.append([])
                continue
            for pattern, repl, forced in self.rules:
                text = "".join(ch for ch, _ in cells)
                new: list[Cell] = []
                last = 0
                for m in pattern.finditer(text):
                    new.extend(cells[last : m.start()])
                    replaced = m.expand(repl)
                    matched = cells[m.start() : m.end()]
                    if forced is None and len(replaced) == len(matched):
                        # A same-length rewrite keeps each cell's own style.
                        new.extend((ch, cell[1]) for ch, cell in zip(replaced, matched))
                    else:
                        style = forced if forced is not None else (cells[m.start()][1] if m.start() < len(cells) else "")
                        new.extend((ch, style) for ch in replaced)
                    last = m.end()
                if last:
                    new.extend(cells[last:])
                    cells = new
            out.append(cells)
        return out


# hoocode's stderr note for --print and --mode json (crates/hoocode-code-cli/src/runtime.rs).
# It is blanked, row for row, on screen: the goldens are not compared with hoocode-ts, which
# prints no note, and the note's wrapped tail is blanked with it.
NOTE = re.compile(r"^Note: --(print|mode json) does not ask for tool approval;.*$")
NOTE_TAIL = "run without it."


def is_tool_rule(pattern: str) -> bool:
    """True for a normalize.json rule that maps hoocode-ts tool names (see TOOL_WORDS)."""
    return any(name in pattern for name in TOOL_WORDS)


# ---------------------------------------------------------------------------
# Scenario execution
# ---------------------------------------------------------------------------


def load_scenario(name: str) -> dict:
    path = SCENARIOS / f"{name}.json"
    if not path.exists():
        sys.exit(f"unknown scenario {name}")
    sc = json.loads(path.read_text())
    sc.setdefault("id", name)
    return sc


def all_scenarios() -> list[str]:
    return sorted(p.stem for p in SCENARIOS.glob("*.json") if p.stem not in EXCLUDED)


def write_models_json(home: Path, port: int, scenario: dict) -> None:
    models = scenario.get("models") or [{"id": "mock-model", "name": "Mock Model", "contextWindow": 128000, "maxTokens": 4096}]
    doc = {
        "providers": {
            "mock": {
                "baseUrl": f"http://127.0.0.1:{port}/v1",
                "api": "openai-completions",
                "apiKey": "mock-key",
                "models": models,
            }
        }
    }
    d = home / CONFIG_DIR
    d.mkdir(parents=True, exist_ok=True)
    (d / "models.json").write_text(json.dumps(doc, indent=2))
    if "settings" in scenario:
        (d / "settings.json").write_text(json.dumps(scenario["settings"], indent=2))


def start_mock(script: list, workdir: Path) -> tuple[subprocess.Popen, int, Path]:
    script_path = workdir / "llm-script.json"
    script_path.write_text(json.dumps(script))
    port_file = workdir / "llm-port"
    log = workdir / "requests.jsonl"
    proc = subprocess.Popen(
        [sys.executable, str(MOCKLLM), "--script", str(script_path), "--port-file", str(port_file), "--log", str(log)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(100):
        if port_file.exists() and port_file.read_text().strip():
            return proc, int(port_file.read_text()), log
        time.sleep(0.05)
    proc.kill()
    raise RuntimeError("mock LLM did not start")


def wait_for(tmux: Tmux, pattern: str, timeout: float, absent: bool = False) -> str:
    rx = re.compile(pattern, re.M)
    deadline = time.time() + timeout
    screen = ""
    while time.time() < deadline:
        screen = tmux.capture()
        found = bool(rx.search(screen))
        if found != absent:
            return screen
        if tmux.dead():
            break
        time.sleep(0.1)
    what = "disappear" if absent else "appear"
    raise StepError(f"timeout waiting for /{pattern}/ to {what}; screen was:\n{screen}")


def wait_stable(tmux: Tmux, quiet: float, timeout: float, normalizer: Normalizer) -> None:
    """Wait until the *normalized* screen stops changing (masked animations don't count)."""

    def snap() -> str:
        return grid_styled(normalizer.apply(parse_screen(tmux.capture(styled=True))))

    deadline = time.time() + timeout
    last = snap()
    since = time.time()
    while time.time() < deadline:
        time.sleep(0.05)
        cur = snap()
        if cur != last:
            last, since = cur, time.time()
        elif time.time() - since >= quiet:
            return
    raise StepError(f"screen did not settle for {quiet}s within {timeout}s")


def write_files(sc: dict, work: Path) -> None:
    """The scenario's `files` (text) and `binary_files` (base64) into `work`."""
    for rel, content in (sc.get("files") or {}).items():
        p = work / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content)
    for rel, content in (sc.get("binary_files") or {}).items():
        p = work / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(base64.b64decode(content))


def run_step(tmux: Tmux, step: dict, out: Path, normalizer: Normalizer, snapshots: dict,
             dirs: dict[str, Path], stdout_file: Path | None = None) -> None:
    timeout = float(step.get("timeout", 15))
    if "wait_stdout" in step:
        # `stdout_jsonl` scenarios: wait until the captured stdout matches.
        if stdout_file is None:
            raise StepError("wait_stdout needs stdout_jsonl")
        pattern = re.compile(step["wait_stdout"], re.M)
        deadline = time.time() + timeout
        while not (stdout_file.exists() and pattern.search(stdout_file.read_text(errors="replace"))):
            if time.time() > deadline:
                raise StepError(f"stdout did not match {step['wait_stdout']!r} within {timeout}s")
            time.sleep(0.05)
    elif "write_settings" in step:
        (dirs["HOME"] / CONFIG_DIR / "settings.json").write_text(json.dumps(step["write_settings"], indent=2))
    elif "write_files" in step:
        for rel, content in step["write_files"].items():
            p = dirs["WORK"] / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(content)
    elif "type" in step:
        tmux.send_text(step["type"])
    elif "keys" in step:
        tmux.send_keys(step["keys"] if isinstance(step["keys"], list) else [step["keys"]])
    elif "wait_for" in step:
        wait_for(tmux, step["wait_for"], timeout)
    elif "wait_gone" in step:
        wait_for(tmux, step["wait_gone"], timeout, absent=True)
    elif "wait_stable" in step:
        wait_stable(tmux, float(step["wait_stable"]), timeout, normalizer)
    elif "wait_exit" in step:
        deadline = time.time() + timeout
        while not tmux.dead():
            if time.time() > deadline:
                raise StepError(f"app did not exit within {timeout}s")
            time.sleep(0.1)
        # tmux flips #{pane_dead} before it draws the "Pane is dead" line. Wait for it.
        marker_deadline = time.time() + 5
        while "Pane is dead" not in tmux.capture(history=True) and time.time() < marker_deadline:
            time.sleep(0.05)
    elif "sleep" in step:
        time.sleep(float(step["sleep"]))
    elif "snapshot" in step:
        name = step["snapshot"]
        if name in snapshots:
            raise StepError(f"duplicate snapshot name {name!r} in one scenario")
        grid = normalizer.apply(parse_screen(tmux.capture(styled=True, history=bool(step.get("history", False)))))
        plain, styled = grid_text(grid), grid_styled(grid)
        snapshots[name] = (styled, plain)
        (out / f"{name}.txt").write_text(styled)
        (out / f"{name}.plain.txt").write_text(plain)
        for needle in step.get("contains", []):
            if needle not in plain:
                raise StepError(f"snapshot {name}: expected to contain {needle!r}")
        for needles in step.get("contains_line", []):
            if not any(all(n in line for n in needles) for line in plain.splitlines()):
                raise StepError(f"snapshot {name}: expected one line to contain all of {needles!r}")
        for needle in step.get("not_contains", []):
            if needle in plain:
                raise StepError(f"snapshot {name}: expected NOT to contain {needle!r}")
    else:
        raise StepError(f"unknown step {step}")


def run_scenario(name: str, out: Path) -> dict:
    """Run one scenario. Returns {"ok", "error", "snapshots": {name: (styled, plain)}}."""
    sc = load_scenario(name)
    out.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix="goldens-"))
    home, work = tmp / "home", tmp / "work"
    home.mkdir()
    work.mkdir()
    write_files(sc, work)
    for rel, target in (sc.get("symlinks") or {}).items():
        if "{HOOCODE_PKG}" in target:
            raise StepError(f"{name} links into the hoocode-ts package; add it to EXCLUDED")
        p = work / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.symlink_to(target)
    if sc.get("git"):
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=work, check=True)

    mock, port, log = start_mock(sc.get("llm", []), tmp)
    write_models_json(home, port, sc)

    term = sc.get("terminal", {})
    tmux = Tmux("hoocode", int(term.get("cols", 100)), int(term.get("rows", 30)))
    subs = {"HOME": str(home), "WORK": str(work), "TMP": str(tmp)}
    env = {
        "HOME": str(home),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "TERM": term.get("term", "xterm-256color"),
        "COLORTERM": term.get("colorterm", "truecolor"),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TZ": "UTC",
        # `{WORK}`, `{HOME}` and `{TMP}` in a value name this run's temp dirs.
        **{k: v.replace("{WORK}", subs["WORK"]).replace("{HOME}", subs["HOME"]).replace("{TMP}", subs["TMP"])
           for k, v in (sc.get("env") or {}).items()},
    }
    argv = hoocode_cmd() + list(sc.get("args", DEFAULT_ARGS))
    normalizer = Normalizer.load(sc.get("normalize"), subs)
    dirs = {"HOME": home, "WORK": work}
    snapshots: dict = {}
    result: dict = {"ok": True, "error": None, "snapshots": snapshots}
    stdout_file = tmp / "stdout.jsonl" if sc.get("stdout_jsonl") is not None else None
    try:
        # `pre_runs`: argument lists run to completion first, in the same workspace and
        # home (e.g. to leave a session behind for --continue).
        for pre in sc.get("pre_runs") or []:
            done = subprocess.run(hoocode_cmd() + list(pre), cwd=work, env=env, stdin=subprocess.DEVNULL,
                                  capture_output=True, text=True, timeout=120)
            if done.returncode != 0:
                raise StepError(f"pre_run {pre} exited {done.returncode}: {done.stderr[-2000:]}")
        tmux.start(argv, work, env, stdout_file)
        for i, step in enumerate(sc["steps"]):
            try:
                run_step(tmux, step, out, normalizer, snapshots, dirs, stdout_file)
            except StepError as e:
                raise StepError(f"step {i} {json.dumps(step)}: {e}") from None
    except (StepError, RuntimeError) as e:
        result["ok"] = False
        result["error"] = str(e)
        (out / "error.txt").write_text(str(e))
        try:
            (out / "last-screen.txt").write_text(tmux.capture(history=True))
        except RuntimeError:
            pass
    finally:
        tmux.kill()
        mock.send_signal(signal.SIGTERM)
        mock.wait(timeout=5)
        if log.exists():
            shutil.copy(log, out / "requests.jsonl")
        shutil.rmtree(tmp, ignore_errors=True)
    return result


# ---------------------------------------------------------------------------
# Golden files
# ---------------------------------------------------------------------------


def golden_dir(name: str) -> Path:
    return GOLDEN / name


def golden_files(name: str) -> dict[str, str]:
    """Snapshot name -> golden content, for the goldens on disk."""
    d = golden_dir(name)
    if not d.exists():
        return {}
    return {p.stem: p.read_text() for p in sorted(d.glob("*.txt"))}


def run_all(names: list[str]) -> dict[str, dict]:
    """Run each scenario, writing its screens to target/tui-goldens/<scenario>/."""
    results = {}
    for name in names:
        out = OUT / name
        shutil.rmtree(out, ignore_errors=True)
        t0 = time.time()
        try:
            res = run_scenario(name, out)
        except (StepError, RuntimeError) as e:
            res = {"ok": False, "error": str(e), "snapshots": {}}
        results[name] = res
        status = "ok" if res["ok"] else "FAILED"
        print(f"ran {status:6} {name:32} {len(res['snapshots']):2} snapshot(s)  {time.time() - t0:5.1f}s")
        if not res["ok"]:
            print("  " + res["error"].splitlines()[0][:300])
            print(f"  see {out / 'error.txt'} and {out / 'last-screen.txt'}")
    return results


def cmd_check(names: list[str]) -> int:
    results = run_all(names)
    bad = 0
    for name, res in results.items():
        if not res["ok"]:
            bad += 1
            continue
        golden = golden_files(name)
        problems = []
        for snap, (styled, _) in res["snapshots"].items():
            if snap not in golden:
                problems.append(f"no golden for snapshot {snap!r} (run update)")
                continue
            if golden[snap] != styled:
                diff = difflib.unified_diff(golden[snap].splitlines(), styled.splitlines(),
                                            f"golden/{name}/{snap}.txt", f"current/{name}/{snap}.txt", lineterm="")
                problems.append("\n".join(diff))
        for snap in sorted(set(golden) - set(res["snapshots"])):
            problems.append(f"stale golden {snap!r}: the scenario no longer has that snapshot (run update)")
        if problems:
            bad += 1
            print(f"DIFF   {name}")
            print("\n".join(problems))
    summary = "all goldens match" if bad == 0 else f"{bad} scenario(s) differ or failed"
    print(f"\n{summary} ({len(results)} scenarios)")
    return 1 if bad else 0


def cmd_update(names: list[str]) -> int:
    results = run_all(names)
    failed = [n for n, r in results.items() if not r["ok"]]
    for name, res in results.items():
        if not res["ok"]:
            print(f"kept old goldens for {name} (run failed)")
            continue
        d = golden_dir(name)
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir(parents=True)
        for snap, (styled, _) in res["snapshots"].items():
            (d / f"{snap}.txt").write_text(styled)
    print(f"updated {len(results) - len(failed)} scenario(s); {len(failed)} failed")
    return 1 if failed else 0


def cmd_run(names: list[str]) -> int:
    results = run_all(names)
    failed = [n for n, r in results.items() if not r["ok"]]
    print(f"{len(results) - len(failed)} ok, {len(failed)} failed (screens in {OUT})")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for cmd in ("run", "check", "update"):
        p = sub.add_parser(cmd)
        p.add_argument("scenario", help="a scenario name, or 'all'")
    args = ap.parse_args()

    if shutil.which("tmux") is None:
        sys.exit("tmux is required")
    names = all_scenarios() if args.scenario == "all" else [args.scenario]
    if args.scenario != "all" and args.scenario in EXCLUDED:
        sys.exit(f"{args.scenario} is excluded: {EXCLUDED[args.scenario]}")
    return {"run": cmd_run, "check": cmd_check, "update": cmd_update}[args.cmd](names)


if __name__ == "__main__":
    sys.exit(main())
