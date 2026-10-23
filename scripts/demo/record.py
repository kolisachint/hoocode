#!/usr/bin/env python3
"""Record the README demo: the real hoocode binary, a scripted model, an animated GIF.

The run reuses the screen-golden harness (``scripts/tui/goldens.py``): hoocode starts in
a fixed-size tmux terminal, in a throwaway workspace and home, against the scripted mock
LLM (``scripts/tui/mockllm.py``). Only the model is scripted; the tools run for real
(the Edit changes the file, the Shell call runs ``cargo test``). While the steps run, the
screen is captured about ten times a second. Each distinct screen is drawn as HTML,
screenshotted with Playwright's Chromium (``render_frames.mjs``), and ffmpeg joins the
frames, each held as long as it was on screen, into one GIF.

Usage::

    scripts/demo/record.sh                 # writes docs/assets/demo.gif
    scripts/demo/record.py [--scenario scripts/demo/demo.json] [--out docs/assets/demo.gif]
                           [--keep DIR]    # keep the frames and HTML for a look

Needs: tmux, node with the ``playwright`` package (``npm root -g``), a Chromium for it
(``PLAYWRIGHT_CHROMIUM``, default /opt/pw-browsers/chromium-1194/chrome-linux/chrome),
ffmpeg, and cargo/rustup (the demo runs ``cargo test`` in its workspace).
``HOOCODE_BIN`` runs that binary instead of building ``target/debug/hoocode``.

Scenario format: ``scripts/tui/scenarios/README.md``, plus ``model`` (the provider and
model the HUD names), ``workspace`` (the project folder's name, under the home),
``prepare`` (commands run in the workspace before recording), and the steps
``type_slowly`` (with ``cps``, characters per second) and ``pause`` (seconds, recorded
as they pass).
"""

from __future__ import annotations

import argparse
import html
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unicodedata
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(ROOT / "scripts" / "tui"))

import goldens  # noqa: E402  (the harness: tmux driver, screen parser, waits)

BG = "#070b18"
FG = "#e6ecff"
CAPTURE_EVERY_S = 0.1


# ---------------------------------------------------------------------------
# Capture
# ---------------------------------------------------------------------------


class Recorder:
    """Captures the styled screen in the background; keeps (time, screen) on change."""

    def __init__(self, tmux: goldens.Tmux) -> None:
        self.tmux = tmux
        self.frames: list[tuple[float, str]] = []
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self.t0 = time.time()
        self.thread.start()

    def _run(self) -> None:
        while not self.stop.is_set():
            try:
                screen = self.tmux.capture(styled=True)
            except RuntimeError:
                break
            if not self.frames or self.frames[-1][1] != screen:
                self.frames.append((time.time() - self.t0, screen))
            time.sleep(CAPTURE_EVERY_S)

    def finish(self) -> float:
        self.stop.set()
        self.thread.join(timeout=5)
        return time.time() - self.t0


def start_mock(script: list, workdir: Path) -> tuple[subprocess.Popen, int]:
    script_path = workdir / "llm-script.json"
    script_path.write_text(json.dumps(script))
    port_file = workdir / "llm-port"
    # Small chunks so the reply visibly streams.
    proc = subprocess.Popen(
        [sys.executable, str(goldens.MOCKLLM), "--script", str(script_path), "--port-file", str(port_file),
         "--chunk-size", "3", "--log", str(workdir / "requests.jsonl")],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(100):
        if port_file.exists() and port_file.read_text().strip():
            return proc, int(port_file.read_text())
        time.sleep(0.05)
    proc.kill()
    raise RuntimeError("mock LLM did not start")


def write_home(home: Path, port: int, model: dict) -> None:
    doc = {
        "providers": {
            model["provider"]: {
                "baseUrl": f"http://127.0.0.1:{port}/v1",
                "api": "openai-completions",
                "apiKey": "demo-key",
                "models": [{"id": model["id"], "name": model["name"], "contextWindow": 200000,
                            "maxTokens": 8192}],
            }
        }
    }
    d = home / goldens.CONFIG_DIR
    d.mkdir(parents=True, exist_ok=True)
    (d / "models.json").write_text(json.dumps(doc, indent=2))
    # No tips or update notices in the recording: the band stays for what the run does.
    # No semantic index either: the demo machine has no embsearch, and its notice is noise.
    (d / "settings.json").write_text(json.dumps({"tips": {"enabled": False}, "enableSemanticIndex": False},
                                                indent=2))


def run_steps(tmux: goldens.Tmux, steps: list[dict]) -> None:
    quiet = goldens.Normalizer()
    for i, step in enumerate(steps):
        timeout = float(step.get("timeout", 15))
        try:
            if "pause" in step:
                time.sleep(float(step["pause"]))
            elif "type_slowly" in step:
                delay = 1.0 / float(step.get("cps", 25))
                for ch in step["type_slowly"]:
                    tmux.send_text(ch)
                    time.sleep(delay)
            elif "type" in step:
                tmux.send_text(step["type"])
            elif "keys" in step:
                tmux.send_keys(step["keys"])
            elif "wait_for" in step:
                goldens.wait_for(tmux, step["wait_for"], timeout)
            elif "wait_stable" in step:
                goldens.wait_stable(tmux, float(step["wait_stable"]), timeout, quiet)
            else:
                raise goldens.StepError(f"unknown step {step}")
        except goldens.StepError as e:
            raise goldens.StepError(f"step {i} {json.dumps(step)}: {e}") from None


def record(sc: dict, work_root: Path) -> tuple[list[tuple[float, str]], float, int, int]:
    # The workspace sits in the home, so the screen shows `~/<name>`, not a temp path.
    home = work_root / "home"
    work = home / sc.get("workspace", "project")
    work.mkdir(parents=True)
    goldens.write_files(sc, work)
    if sc.get("git"):
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=work, check=True)
        subprocess.run(["git", "-c", "user.name=demo", "-c", "user.email=demo@example.com", "add", "-A"],
                       cwd=work, check=True)
        subprocess.run(["git", "-c", "user.name=demo", "-c", "user.email=demo@example.com", "commit", "-qm",
                        "init"], cwd=work, check=True)
    model = sc["model"]
    mock, port = start_mock(sc["llm"], work_root)
    write_home(home, port, model)
    term = sc.get("terminal", {})
    cols, rows = int(term.get("cols", 96)), int(term.get("rows", 30))
    tmux = goldens.Tmux("hoocode-demo", cols, rows)
    real_home = Path(os.environ.get("HOME", "/root"))
    env = {
        "HOME": str(home),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "TERM": "xterm-256color",
        "COLORTERM": "truecolor",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TZ": "UTC",
        "HOOCODE_OFFLINE": "1",
        # The demo's `cargo test` uses the real toolchain, outside the throwaway home.
        "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(real_home / ".rustup")),
        "CARGO_HOME": os.environ.get("CARGO_HOME", str(real_home / ".cargo")),
        "CARGO_TARGET_DIR": str(work_root / "cargo-target"),
    }
    # Build the crate once before recording, so the demo's `cargo test` only recompiles
    # the edited file instead of spending the recording on a first build.
    for cmd in sc.get("prepare") or []:
        subprocess.run(cmd, cwd=work, env={**os.environ, **{k: v for k, v in env.items() if k != "HOME"}},
                       check=True, capture_output=True)
    argv = goldens.hoocode_cmd() + ["--offline", "--provider", model["provider"], "--model", model["id"]]
    rec = Recorder(tmux)
    try:
        tmux.start(argv, work, env)
        rec.start()
        run_steps(tmux, sc["steps"])
    finally:
        total = rec.finish()
        tmux.kill()
        mock.send_signal(signal.SIGTERM)
        mock.wait(timeout=5)
    return rec.frames, total, cols, rows


# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------


def cell_css(key: str) -> str:
    st: dict[str, str | bool] = {}
    for part in key.split(";"):
        if not part:
            continue
        k, _, v = part.partition("=")
        st[k] = v or True
    fg, bg = st.get("fg", FG), st.get("bg", "")
    if st.get("inverse"):
        fg, bg = (bg or BG), fg
    decls = [f"color:{fg}"]
    if bg:
        decls.append(f"background:{bg}")
    if st.get("bold"):
        decls.append("font-weight:700")
    if st.get("dim"):
        decls.append("opacity:.6")
    if st.get("italic"):
        decls.append("font-style:italic")
    if st.get("underline"):
        decls.append("text-decoration:underline")
    if st.get("strike"):
        decls.append("text-decoration:line-through")
    return ";".join(decls)


def screen_html(styled: str, cols: int, rows: int) -> str:
    grid = goldens.parse_screen(styled)
    out_rows = []
    for r in range(rows):
        cells = grid[r] if r < len(grid) else []
        spans, width = [], 0
        for ch, key in cells:
            if width >= cols:
                break
            w = 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
            css = cell_css(key)
            text = "&#160;" if ch == " " else html.escape(ch)
            spans.append(f'<span class="c{w}" style="{css}">{text}</span>')
            width += w
        out_rows.append(f'<div class="r">{"".join(spans)}</div>')
    return "".join(out_rows)


PAGE = """<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;padding:0;background:%(bg)s}
#t{display:inline-block;padding:14px 18px;background:%(bg)s;border-radius:10px;
  font:15px/20px "JetBrains Mono","DejaVu Sans Mono","FreeMono",monospace;
  font-variant-ligatures:none;color:%(fg)s}
.r{display:block;height:20px;white-space:pre;width:%(cols)sch}
.r span{display:inline-block;height:20px;vertical-align:top;overflow:hidden;text-align:center}
.c1{width:1ch}.c2{width:2ch}
</style></head><body><div id="t"></div></body></html>"""


def render(frames: list[tuple[float, str]], total: float, cols: int, rows: int, tmp: Path, out: Path) -> None:
    # Drop leading frames before hoocode draws (an empty pane).
    while frames and not frames[0][1].strip():
        frames.pop(0)
    t_start = frames[0][0]
    bodies = [screen_html(s, cols, rows) for _, s in frames]
    durations = [max(0.04, (frames[i + 1][0] if i + 1 < len(frames) else total) - frames[i][0])
                 for i in range(len(frames))]
    (tmp / "page.html").write_text(PAGE % {"bg": BG, "fg": FG, "cols": cols})
    (tmp / "frames.json").write_text(json.dumps(bodies))
    frames_dir = tmp / "png"
    frames_dir.mkdir(exist_ok=True)
    node_path = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True, check=True).stdout.strip()
    subprocess.run(["node", str(HERE / "render_frames.mjs"), str(tmp / "page.html"), str(tmp / "frames.json"),
                    str(frames_dir)], check=True, env={**os.environ, "NODE_PATH": node_path})
    concat = ["ffconcat version 1.0"]
    for i, d in enumerate(durations):
        concat += [f"file '{frames_dir / f'{i:04d}.png'}'", f"duration {d:.3f}"]
    concat.append(f"file '{frames_dir / f'{len(durations) - 1:04d}.png'}'")
    (tmp / "frames.ffconcat").write_text("\n".join(concat) + "\n")
    out.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(tmp / "frames.ffconcat"),
         "-vf", "fps=10,split[a][b];[a]palettegen=max_colors=128:stats_mode=full[p];"
                "[b][p]paletteuse=dither=none:diff_mode=rectangle",
         "-loop", "0", str(out)],
        check=True,
    )
    print(f"{out}: {len(frames)} screens, {total - t_start:.1f}s, {out.stat().st_size / 1e6:.2f} MB")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scenario", type=Path, default=HERE / "demo.json")
    ap.add_argument("--out", type=Path, default=ROOT / "docs" / "assets" / "demo.gif")
    ap.add_argument("--keep", type=Path, help="keep the workspace, frames and HTML here")
    args = ap.parse_args()
    for tool in ("tmux", "node", "npm", "ffmpeg"):
        if shutil.which(tool) is None:
            sys.exit(f"record.py needs {tool} on PATH")
    sc = json.loads(args.scenario.read_text())
    tmp = Path(tempfile.mkdtemp(prefix="hoocode-demo-")).resolve()
    try:
        frames, total, cols, rows = record(sc, tmp)
        render(frames, total, cols, rows, tmp, args.out.resolve())
    finally:
        if args.keep:
            if args.keep.exists():
                shutil.rmtree(args.keep)
            shutil.copytree(tmp, args.keep, ignore=shutil.ignore_patterns("cargo-target"))
        shutil.rmtree(tmp, ignore_errors=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
