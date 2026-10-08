#!/usr/bin/env python3
"""Phase 0 load scenario: the real `hoocode` in tmux, under load, with --perf-log.

The mock model's first turn calls `bash` eight times in one message. Each call
prints a lot of output, and the eight run in parallel. While they run, keys are
typed into the prompt at a steady rate. The run writes a perf log (one JSON line
a second), then sends `/perf` so the notice is captured, and prints a summary.

    scripts/perf/load_scenario.py                      # defaults
    scripts/perf/load_scenario.py --typing-seconds 30 --lines 20000
    scripts/perf/load_scenario.py --binary /path/to/hoocode

Artifacts land in `target/perf/load-<time>/`: `perf.jsonl`, `pane-after-perf.txt`
(the `/perf` notice as the terminal drew it) and `summary.json`.

Not in this scenario yet (see docs/design/concurrency.md §5): the five subagents,
the stdio MCP server that never answers, and the paused-pty run (a terminal that
stops reading). Those need the subagent pool and an MCP fixture; they are TODO.
TODO(subagents): dispatch five `Agent` children from the parent route.
TODO(mcp): configure a stdio MCP server that accepts a request and never replies.
TODO(paused-pty): a second run where the pty is not read for N seconds.

Limits: the mock answers at full speed (no delay_s), so the first bash output
arrives as fast as the app can take it. macOS is not measured by this script yet.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(REPO / "scripts/eval"))

from mock_provider import MockProvider, Route  # noqa: E402

DEFAULT_BINARY = REPO / "target/debug/hoocode"
OUT_ROOT = REPO / "target/perf"
MODEL = "mock-model"
SESSION = "perf-load"
PROMPT_TEXT = "run the eight load commands"
TURN_DONE = "All eight commands finished."
PERF_MARKER = "Performance (the last"
# Characters typed during the turn; any printable keys do, one per send-keys.
TYPED = "abcdefghijklmnopqrstuvwxyz"


def tmux(socket: str, *args: str, check: bool = True) -> str:
    completed = subprocess.run(
        ["tmux", "-L", socket, "-f", "/dev/null", *args],
        capture_output=True,
        text=True,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(f"tmux {' '.join(args)} failed: {completed.stderr.strip()}")
    return completed.stdout


def bash_turn(calls: int, lines: int) -> dict[str, Any]:
    """One assistant message with `calls` parallel bash calls, each printing a lot.

    With no calls the message is the whole turn, so it carries the done marker.
    """
    if calls == 0:
        return {"text": TURN_DONE, "usage": {"prompt_tokens": 1200, "completion_tokens": 300}}
    tool_calls = []
    for index in range(1, calls + 1):
        command = (
            f"seq 1 {lines} | awk '{{ print \"call {index} line \" $0 \" "
            + "-" * 64
            + "\" }'"
        )
        tool_calls.append({"id": f"call_load_{index}", "name": "bash", "arguments": {"command": command}})
    return {"text": "Running the load commands.", "tool_calls": tool_calls, "usage": {"prompt_tokens": 1200, "completion_tokens": 300}}


def write_project(root: Path, port: int) -> tuple[Path, Path]:
    home = root / "home"
    work = root / "work"
    (home / ".hoocode").mkdir(parents=True)
    (work / ".hoocode").mkdir(parents=True)
    models = {
        "providers": {
            "mock": {
                "baseUrl": f"http://127.0.0.1:{port}/v1",
                "api": "openai-completions",
                "apiKey": "mock-key",
                "models": [{"id": MODEL, "name": "Mock", "contextWindow": 128000, "maxTokens": 4096}],
            }
        }
    }
    (home / ".hoocode" / "models.json").write_text(json.dumps(models), encoding="utf-8")
    settings = {"defaultProvider": "mock", "defaultModel": MODEL}
    (home / ".hoocode" / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
    # bash runs without an approval prompt in this scenario; the prompt would
    # otherwise be part of what is measured.
    hoo = {"active_mode": "build", "modes": {"build": {"auto_allow": ["bash"]}}}
    (work / ".hoocode" / "hoo-config.json").write_text(json.dumps(hoo), encoding="utf-8")
    (work / "README.txt").write_text("load scenario workspace\n", encoding="utf-8")
    return home, work


def wait_for(socket: str, text: str, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if text in tmux(socket, "capture-pane", "-p", "-t", SESSION, check=False):
            return True
        time.sleep(0.2)
    return False


def pane(socket: str) -> str:
    return tmux(socket, "capture-pane", "-p", "-t", SESSION, check=False)


def drive(socket: str, args: argparse.Namespace, log: Path, binary: Path, home: Path, work: Path) -> dict[str, Any]:
    env = {
        "HOME": str(home),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "TERM": "xterm-256color",
        "COLORTERM": "truecolor",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "HOOCODE_SUBAGENT_DEPTH": "0",
    }
    env_args = ["env", "-i", *[f"{k}={v}" for k, v in sorted(env.items())]]
    argv = [str(binary), "--offline", "--provider", "mock", "--model", MODEL, "--perf-log", str(log)]
    wrapped = ["sh", "-c", '"$@"; exit $?', "sh", *env_args, *argv]
    tmux(socket, "new-session", "-d", "-s", SESSION, "-x", str(args.cols), "-y", str(args.rows), "-c", str(work), *wrapped)

    if not wait_for(socket, "❯", timeout=60):
        raise RuntimeError("the prompt never appeared; pane:\n" + pane(socket))
    time.sleep(1.0)

    started = time.monotonic()
    tmux(socket, "send-keys", "-t", SESSION, "-l", PROMPT_TEXT)
    tmux(socket, "send-keys", "-t", SESSION, "Enter")

    typed = 0
    done_at: float | None = None
    interval = 1.0 / args.keys_per_second
    next_key = time.monotonic()
    last_check = 0.0
    while time.monotonic() - started < args.typing_seconds:
        now = time.monotonic()
        if now >= next_key:
            tmux(socket, "send-keys", "-t", SESSION, "-l", TYPED[typed % len(TYPED)])
            typed += 1
            next_key += interval
        if done_at is None and now - last_check >= 0.5:
            last_check = now
            if TURN_DONE in pane(socket):
                done_at = now - started
        time.sleep(max(0.0, next_key - time.monotonic()))

    if done_at is None:
        if not wait_for(socket, TURN_DONE, timeout=args.turn_timeout):
            raise RuntimeError("the turn never finished; pane:\n" + pane(socket))
        done_at = time.monotonic() - started
    typing_window = time.monotonic() - started

    # Clear the typed text, then ask for the counters. Keys sent with /perf are
    # part of the same measurement.
    tmux(socket, "send-keys", "-t", SESSION, "C-u")
    time.sleep(0.5)
    tmux(socket, "send-keys", "-t", SESSION, "-l", "/perf")
    tmux(socket, "send-keys", "-t", SESSION, "Enter")
    shown = wait_for(socket, PERF_MARKER, timeout=30)
    time.sleep(1.5)  # one more publish tick, so the log holds the final numbers
    screen = pane(socket)
    (args.out / "pane-after-perf.txt").write_text(screen, encoding="utf-8")

    tmux(socket, "send-keys", "-t", SESSION, "-l", "/quit")
    tmux(socket, "send-keys", "-t", SESSION, "Enter")
    time.sleep(1.0)
    return {
        "keys_typed": typed,
        "keys_per_second_target": args.keys_per_second,
        "turn_done_after_s": round(done_at, 2),
        "typing_window_s": round(typing_window, 2),
        "perf_notice_shown": shown,
    }


def read_log(log: Path) -> list[dict[str, Any]]:
    if not log.exists():
        return []
    return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines() if line.strip()]


def dist_row(name: str, dist: dict[str, Any] | None) -> str:
    if not dist:
        return f"  {name:<24} no samples"
    return (
        f"  {name:<24} n={dist['n']:<6} min {dist['min']:8.2f}  p50 {dist['p50']:8.2f}  p99 {dist['p99']:8.2f}  (ms)"
    )


def summarize(lines: list[dict[str, Any]], drive_info: dict[str, Any]) -> dict[str, Any]:
    last = lines[-1] if lines else {}
    threads = [line["threads"] for line in lines if line.get("threads") is not None]
    rss = [line["rss_kib"] for line in lines if line.get("rss_kib") is not None]
    p99_peak = {}
    for key in ("frame_build_ms", "frame_write_ms", "key_latency_ms", "loop_iteration_ms"):
        values = [line[key]["p99"] for line in lines if line.get(key)]
        p99_peak[key] = max(values) if values else None
    return {
        "samples_seconds": len(lines),
        "frames": last.get("frames"),
        "stalls_over_500ms": last.get("stalls"),
        "threads_max": max(threads) if threads else None,
        "threads_last": threads[-1] if threads else None,
        "rss_mib_max": round(max(rss) / 1024, 1) if rss else None,
        "rss_mib_last": round(rss[-1] / 1024, 1) if rss else None,
        "p99_peak_per_second_ms": p99_peak,
        "final_window": {key: last.get(key) for key in ("frame_build_ms", "frame_write_ms", "key_latency_ms", "loop_iteration_ms")},
        **drive_info,
    }


def print_summary(summary: dict[str, Any], out: Path) -> None:
    print()
    print(f"perf log: {out / 'perf.jsonl'} ({summary['samples_seconds']} lines, one a second)")
    print(f"keys typed: {summary['keys_typed']} over {summary['typing_window_s']} s; turn done at {summary['turn_done_after_s']} s")
    print(f"frames: {summary['frames']}   stalls (loop > 500 ms): {summary['stalls_over_500ms']}")
    print(f"threads: max {summary['threads_max']}, last {summary['threads_last']}")
    print(f"rss: max {summary['rss_mib_max']} MiB, last {summary['rss_mib_last']} MiB")
    print("final window (last 1024 samples of each):")
    final = summary["final_window"]
    print(dist_row("frame build", final["frame_build_ms"]))
    print(dist_row("frame write", final["frame_write_ms"]))
    print(dist_row("keystroke to frame", final["key_latency_ms"]))
    print(dist_row("loop iteration", final["loop_iteration_ms"]))
    peaks = summary["p99_peak_per_second_ms"]
    print("worst p99 seen in any one second (ms): " + ", ".join(f"{k.removesuffix('_ms')} {v:.2f}" for k, v in peaks.items() if v is not None))
    print(f"/perf notice shown: {summary['perf_notice_shown']}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY, help="hoocode binary (default: target/debug/hoocode)")
    parser.add_argument("--calls", type=int, default=8, help="parallel bash calls in the turn (default 8)")
    parser.add_argument("--lines", type=int, default=5000, help="lines each bash call prints (default 5000)")
    parser.add_argument("--keys-per-second", type=float, default=20.0, help="typing rate during the turn (default 20)")
    parser.add_argument("--typing-seconds", type=float, default=15.0, help="minimum time keys are typed (default 15)")
    parser.add_argument("--turn-timeout", type=float, default=120.0, help="seconds to wait for the turn to finish")
    parser.add_argument("--cols", type=int, default=120)
    parser.add_argument("--rows", type=int, default=40)
    parser.add_argument("--out", type=Path, default=None, help="artifact directory (default target/perf/load-<time>)")
    parser.add_argument("--keep", action="store_true", help="leave the tmux server running on exit")
    args = parser.parse_args()

    binary = args.binary.resolve()
    if not binary.exists():
        print(f"error: {binary} does not exist; run `cargo build -p hoocode-code-main` first", file=sys.stderr)
        return 2
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    args.out = (args.out or OUT_ROOT / f"load-{stamp}").resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    log = args.out / "perf.jsonl"

    root = Path(tempfile.mkdtemp(prefix=f"perf-load-{stamp}-"))
    provider = MockProvider(
        {"parent": Route("parent", [bash_turn(args.calls, args.lines), {"text": TURN_DONE}])},
        log_path=str(root / "requests.jsonl"),
    )
    port = provider.start()
    socket = f"perf-load-{os.getpid()}"
    try:
        home, work = write_project(root, port)
        drive_info = drive(socket, args, log, binary, home, work)
    finally:
        if not args.keep:
            tmux(socket, "kill-server", check=False)
        provider.stop()
        shutil.rmtree(root, ignore_errors=True)

    lines = read_log(log)
    summary = summarize(lines, drive_info)
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print_summary(summary, args.out)
    return 0 if lines else 1


if __name__ == "__main__":
    sys.exit(main())
