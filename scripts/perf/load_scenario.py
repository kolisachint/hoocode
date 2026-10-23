#!/usr/bin/env python3
"""Load scenarios for the hoocode performance numbers (docs/design/concurrency.md §5).

The real `hoocode` runs against the mock model, with `--perf-log`. Scenarios:

  bash        The Phase 0 load run: the first turn calls `bash` eight times in one
              message, each call printing a lot. Keys are typed into the prompt at a
              steady rate while they run (tmux).
  load        `bash`, plus five `Agent` subagents dispatched in the same message, plus
              one call to a stdio MCP server that accepts the request and never answers
              (scripts/perf/mcp_stall_server.py). Keys are typed for longer than the
              MCP call's 60 s deadline, so the stall happens under typing.
  stream      One long streamed reply (STREAM_CHUNKS deltas, paced), keys typed
              while it streams. Measures keystroke-to-frame and frame build with a
              growing message, and how long the reply takes to show on screen.
  long-transcript
              The stream scenario on top of a resumed session of TRANSCRIPT_TURNS (200)
              generated turns: markdown, code blocks and bash output (`--session`). Frame
              cost should not depend on transcript length; compare with `stream`.
  paused-pty  A terminal that stops reading. The app runs in a pty; after a bash
              turn the pty is not read for N seconds (its output blocks), then Ctrl+C
              is sent. The turn must abort within 500 ms: the mock sees the client
              close the stream.
  memory      A forced 3 GiB allocation inside the hoocode process (an LD_PRELOAD shim,
              scripts/perf/alloc_shim.c, because the product has no allocation hook),
              with the soft limit lowered to its 256 MB floor. Shedding must start, then
              lift after the memory is freed. The hard limit stays at its default.
  all         Every scenario above (default).

    scripts/perf/load_scenario.py                       # all scenarios
    scripts/perf/load_scenario.py --scenario load
    scripts/perf/load_scenario.py --binary /path/to/hoocode

The binary defaults to `$CARGO_TARGET_DIR/release/hoocode` (build it with
`cargo build -p hoocode-code-main --release`). Artifacts land in
`$CARGO_TARGET_DIR/perf/load-<time>/<scenario>/`: `perf.jsonl`, `pane-*.txt`,
`summary.json`, plus `summary.json` for the whole run.

Measured against the targets in concurrency.md §6: keystroke-to-frame p99 under 16 ms
(final window), Ctrl+C aborts within 500 ms with the paused terminal, the thread count
stays under the 26-thread ceiling, and the memory check sheds then recovers. The
targets are not tuned here; a miss is printed as MISS.

Not measured: macOS. The subagents are child processes; their threads and RSS are not
in the parent's perf log (the parent's numbers are what the frame checks use).
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(REPO / "scripts/eval"))

from mock_provider import MockProvider, Route  # noqa: E402

TARGET_DIR = Path(os.environ.get("CARGO_TARGET_DIR") or (REPO / "target"))
DEFAULT_BINARY = TARGET_DIR / "release/hoocode"
OUT_ROOT = TARGET_DIR / "perf"
MCP_FIXTURE = HERE / "mcp_stall_server.py"
SHIM_SOURCE = HERE / "alloc_shim.c"
MODEL = "mock-model"
SESSION = "perf-load"
PROMPT_TEXT = "run the eight load commands"
TURN_DONE = "All eight commands finished."
STREAM_CHUNKS = 3000
STREAM_DELAY_S = 0.005
TRANSCRIPT_TURNS = 200
PERF_MARKER = "Performance (the last"
START_MARKER = "name your mission"
PROMPT_GLYPH = "❯"
PROMPT_BYTES = PROMPT_GLYPH.encode("utf-8")
CHILD_MARKER = "Task: "
CHILD_TEXT = "crates/ holds one directory per Rust crate; scripts/ and docs/ hold the rest."
SUBAGENTS = 5
MCP_CALL = "mcp_stall_wait"  # naming.rs: mcp_<server>_<tool>, server "stall", tool "wait"
SHED_NOTICE = "Memory above the soft limit"
# Characters typed during the turn; any printable keys do, one per send-keys.
TYPED = "abcdefghijklmnopqrstuvwxyz"
# concurrency.md §3: the ceiling under load is about 26 threads.
THREAD_CEILING = 26
KEY_P99_TARGET_MS = 16.0
ABORT_TARGET_MS = 500.0
# perf.rs keeps the last 1024 keystroke latencies. A run that types more than that
# only reports its tail, so the load run types 75 s x 13 keys/s = 975 keys.
KEY_WINDOW = 1024
LOAD_KEYS_PER_SECOND = 13.0


# --------------------------------------------------------------------------
# Mock turns
# --------------------------------------------------------------------------

def usage() -> dict[str, int]:
    return {"prompt_tokens": 1200, "completion_tokens": 300}


def bash_calls(calls: int, lines: int) -> list[dict[str, Any]]:
    tool_calls = []
    for index in range(1, calls + 1):
        command = (
            f"seq 1 {lines} | awk '{{ print \"call {index} line \" $0 \" "
            + "-" * 64
            + "\" }'"
        )
        tool_calls.append({"id": f"call_load_{index}", "name": "bash", "arguments": {"command": command}})
    return tool_calls


def agent_calls(count: int) -> list[dict[str, Any]]:
    return [
        {
            "id": f"call_agent_{index}",
            "name": "Agent",
            "arguments": {
                "subagent_type": "explore",
                "description": f"map the repository layout {index}",
                "prompt": f"List the top-level directories and what is in them ({index}).",
            },
        }
        for index in range(1, count + 1)
    ]


def mcp_calls() -> list[dict[str, Any]]:
    return [{"id": "call_mcp_1", "name": MCP_CALL, "arguments": {}}]


def first_turn(tool_calls: list[dict[str, Any]]) -> dict[str, Any]:
    return {"text": "Running the load commands.", "tool_calls": tool_calls, "usage": usage()}


def done_turn() -> dict[str, Any]:
    return {"text": TURN_DONE, "usage": usage()}


def child_turn() -> dict[str, Any]:
    # A child answers after a delay, so the five children are still running while
    # keys are typed (the parent has already returned from its dispatch).
    return {"text": CHILD_TEXT, "delay_s": 3, "usage": usage()}


class WatchedProvider(MockProvider):
    """A mock that notices when the client hangs up on a `watch_disconnect` turn.

    The turn sends a keepalive comment every 20 ms and never finishes. The first
    write that fails (the client closed the connection) is the time the abort reached
    the HTTP client. `disconnected_at` is on `time.monotonic()`, the same clock the
    driver uses for Ctrl+C.
    """

    def __init__(self, routes: dict[str, Route], log_path: str | None = None):
        super().__init__(routes, log_path=log_path)
        self.hang_started = threading.Event()
        self.disconnect_event = threading.Event()
        self.disconnected_at: float | None = None

    def serve(self, handler, turn):  # noqa: ANN001 - BaseHTTPRequestHandler
        if not turn.get("watch_disconnect"):
            return super().serve(handler, turn)
        handler.send_response(200)
        handler.send_header("Content-Type", "text/event-stream")
        handler.send_header("Cache-Control", "no-cache")
        handler.send_header("Connection", "close")
        handler.end_headers()
        self.hang_started.set()
        try:
            while True:
                handler.wfile.write(b": keepalive\n\n")
                handler.wfile.flush()
                time.sleep(0.02)
        except OSError:
            self.disconnected_at = time.monotonic()
            self.disconnect_event.set()


# --------------------------------------------------------------------------
# Project files
# --------------------------------------------------------------------------

def write_project(root: Path, port: int, performance: dict[str, Any] | None = None, mcp: bool = False) -> tuple[Path, Path]:
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
    settings: dict[str, Any] = {"defaultProvider": "mock", "defaultModel": MODEL}
    if performance is not None:
        settings["performance"] = performance
    (home / ".hoocode" / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
    if mcp:
        # User-level MCP servers are always trusted (config.rs), so no prompt.
        servers = {
            "mcpServers": {
                "stall": {
                    "command": sys.executable,
                    "args": ["-I", str(MCP_FIXTURE), str(root / "mcp-accepted.jsonl")],
                }
            }
        }
        (home / ".agents").mkdir(parents=True)
        (home / ".agents" / "mcp.json").write_text(json.dumps(servers), encoding="utf-8")
    # bash runs without an approval prompt in these scenarios; the prompt would
    # otherwise be part of what is measured.
    hoo = {"active_mode": "build", "modes": {"build": {"auto_allow": ["bash"]}}}
    (work / ".hoocode" / "hoo-config.json").write_text(json.dumps(hoo), encoding="utf-8")
    (work / "README.txt").write_text("load scenario workspace\n", encoding="utf-8")
    return home, work


def base_env(home: Path, extra: dict[str, str]) -> dict[str, str]:
    env = {
        "HOME": str(home),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "TERM": "xterm-256color",
        "COLORTERM": "truecolor",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "HOOCODE_SUBAGENT_DEPTH": "0",
    }
    env.update(extra)
    return env


def hoo_argv(binary: Path, log: Path, extra_args: list[str]) -> list[str]:
    return [str(binary), "--offline", "--provider", "mock", "--model", MODEL, "--perf-log", str(log), *extra_args]


# --------------------------------------------------------------------------
# Generated session (long-transcript)
# --------------------------------------------------------------------------

def _entry_id(n: int) -> str:
    return f"{n:08x}"


def write_session(path: Path, cwd: Path, turns: int) -> None:
    """A session file of `turns` turns: markdown, a code block, a bash call and its output.

    Format: agent-session entries (version 3), a chain of parentId links.
    """
    stamp = "2026-10-01T00:00:00.000Z"
    ms = 1790000000000
    entries: list[dict[str, Any]] = [
        {"type": "session", "version": 3, "id": "01a1675d-e33c-7279-863b-0f8f5fefbd00", "timestamp": stamp, "cwd": str(cwd)},
    ]
    counter = 0
    parent: str | None = None

    def add(kind: str, **fields: Any) -> None:
        nonlocal counter, parent
        counter += 1
        eid = _entry_id(counter)
        entries.append({"type": kind, "id": eid, "parentId": parent, "timestamp": stamp, **fields})
        parent = eid

    add("model_change", provider="mock", modelId=MODEL)
    add("thinking_level_change", thinkingLevel="off")
    cost = {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}
    use = {"input": 1200, "output": 300, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 1500, "cost": cost}

    def assistant(content: list[dict[str, Any]], stop: str) -> dict[str, Any]:
        return {"role": "assistant", "content": content, "api": "openai-completions", "provider": "mock",
                "model": MODEL, "usage": use, "stopReason": stop, "timestamp": ms, "responseId": "mock"}

    for i in range(1, turns + 1):
        add("message", message={"role": "user", "timestamp": ms,
                                "content": [{"type": "text", "text": f"Turn {i}: explain how module {i} handles retries and show the code."}]})
        explain = (
            f"## Module {i}\n\n"
            f"Module {i} retries **three times** with *exponential* backoff. See `retry_{i}()` below; "
            "the delay doubles each attempt and is capped at 30 s.\n\n"
            f"```rust\nfn retry_{i}(mut attempt: u32) -> Result<(), Error> {{\n"
            "    loop {\n        match call() {\n            Ok(v) => return Ok(v),\n"
            "            Err(e) if attempt < 3 => { attempt += 1; sleep(backoff(attempt)); }\n"
            "            Err(e) => return Err(e),\n        }\n    }\n}\n```\n\n"
            "- the first failure is logged at `warn`\n- later failures are logged at `error`\n"
            "- the final error carries the attempt count\n\n"
            "| attempt | delay |\n|---|---|\n| 1 | 1 s |\n| 2 | 2 s |\n| 3 | 4 s |\n"
        )
        call_id = f"call_hist_{i}"
        command = f"seq 1 12 | awk '{{ print \"turn {i} line \" $0 }}'"
        add("message", message=assistant([{"type": "text", "text": explain},
                                          {"type": "toolCall", "id": call_id, "name": "bash", "arguments": {"command": command}}], "toolUse"))
        output = "\n".join(f"turn {i} line {n}" for n in range(1, 13))
        add("message", message={"role": "toolResult", "toolCallId": call_id, "toolName": "bash", "isError": False, "details": {},
                                "content": [{"type": "text", "text": output}], "timestamp": ms})
        add("message", message=assistant([{"type": "text", "text": f"Done with module {i}. The output above shows twelve lines; "
                                                                  "nothing else needs to change."}], "stop"))
    path.write_text("\n".join(json.dumps(e) for e in entries) + "\n", encoding="utf-8")


# --------------------------------------------------------------------------
# tmux driver (bash, load, memory)
# --------------------------------------------------------------------------

def tmux(socket: str, *args: str, check: bool = True) -> str:
    completed = subprocess.run(
        ["tmux", "-L", socket, "-f", "/dev/null", *args],
        capture_output=True,
        text=True,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(f"tmux {' '.join(args)} failed: {completed.stderr.strip()}")
    return completed.stdout


def wait_for(socket: str, text: str, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if text in pane(socket):
            return True
        time.sleep(0.2)
    return False


def wait_gone(socket: str, text: str, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if text not in pane(socket):
            return True
        time.sleep(0.2)
    return False


def pane(socket: str) -> str:
    return tmux(socket, "capture-pane", "-p", "-t", SESSION, check=False)


def start_tmux(socket: str, args: argparse.Namespace, binary: Path, home: Path, work: Path,
               log: Path, extra_env: dict[str, str], extra_args: list[str]) -> None:
    env = base_env(home, extra_env)
    env_args = ["env", "-i", *[f"{k}={v}" for k, v in sorted(env.items())]]
    wrapped = ["sh", "-c", '"$@"; exit $?', "sh", *env_args, *hoo_argv(binary, log, extra_args)]
    tmux(socket, "new-session", "-d", "-s", SESSION, "-x", str(args.cols), "-y", str(args.rows), "-c", str(work), *wrapped)
    if not wait_for(socket, START_MARKER, timeout=60):
        raise RuntimeError("the prompt never appeared; pane:\n" + pane(socket))
    time.sleep(1.0)


def drive_tmux(socket: str, args: argparse.Namespace, out: Path, typing_seconds: float, extra_args: list[str],
               binary: Path, home: Path, work: Path) -> dict[str, Any]:
    log = out / "perf.jsonl"
    start_tmux(socket, args, binary, home, work, log, {}, extra_args)

    started = time.monotonic()
    tmux(socket, "send-keys", "-t", SESSION, "-l", PROMPT_TEXT)
    tmux(socket, "send-keys", "-t", SESSION, "Enter")

    typed = 0
    done_at: float | None = None
    interval = 1.0 / args.keys_per_second
    next_key = time.monotonic()
    last_check = 0.0
    while time.monotonic() - started < typing_seconds:
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
    (out / "pane-after-perf.txt").write_text(pane(socket), encoding="utf-8")

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


def run_memory(socket: str, args: argparse.Namespace, out: Path, binary: Path, root: Path, port: int) -> dict[str, Any]:
    """Forced allocation: shed while the memory is resident, recover after it is freed."""
    home, work = write_project(root, port, performance={"memorySoftLimitMb": 256})
    shim = root / "alloc_shim.so"
    cc = shutil.which("cc") or shutil.which("gcc") or shutil.which("clang")
    if cc is None:
        raise RuntimeError("the memory check needs a C compiler (cc) for the allocation shim")
    subprocess.run([cc, "-shared", "-fPIC", "-O2", "-pthread", "-o", str(shim), str(SHIM_SOURCE)], check=True)
    trigger = root / "alloc-trigger"
    release = root / "alloc-release"
    done = root / "alloc-done"
    extra_env = {
        "LD_PRELOAD": str(shim),
        "HOOCODE_PERF_ALLOC_MB": str(args.alloc_mb),
        "HOOCODE_PERF_ALLOC_TRIGGER": str(trigger),
        "HOOCODE_PERF_ALLOC_RELEASE": str(release),
        "HOOCODE_PERF_ALLOC_DONE": str(done),
    }
    log = out / "perf.jsonl"
    start_tmux(socket, args, binary, home, work, log, extra_env, [])
    time.sleep(args.memory_settle)
    before = pane(socket)

    trigger.write_text("go", encoding="utf-8")
    shed_seen = wait_for(socket, SHED_NOTICE, timeout=args.memory_timeout)
    # The notice can appear while the pages are still being touched, so hold only
    # once the shim reports the whole allocation resident.
    deadline = time.monotonic() + args.memory_timeout
    while time.monotonic() < deadline and not (done.exists() and done.read_text(encoding="utf-8") in ("allocated", "failed")):
        time.sleep(0.2)
    time.sleep(args.memory_hold)  # the notice stays up while the memory is resident
    shed_pane = pane(socket)
    (out / "pane-shedding.txt").write_text(shed_pane, encoding="utf-8")
    allocated = done.read_text(encoding="utf-8") if done.exists() else None

    release.write_text("go", encoding="utf-8")
    lifted = wait_gone(socket, SHED_NOTICE, timeout=args.memory_timeout)
    time.sleep(1.5)  # one more publish tick so the log shows the freed memory
    (out / "pane-recovered.txt").write_text(pane(socket), encoding="utf-8")
    tmux(socket, "send-keys", "-t", SESSION, "-l", "/quit")
    tmux(socket, "send-keys", "-t", SESSION, "Enter")
    time.sleep(1.0)
    return {
        "alloc_mib": args.alloc_mb,
        "soft_limit_mb": 256,
        "shedding_notice_before_alloc": SHED_NOTICE in before,
        "shed_seen": shed_seen,
        "allocation_state_at_shed": allocated,
        "shed_notice_lifted": bool(shed_seen and lifted),
        "allocation_state_final": done.read_text(encoding="utf-8") if done.exists() else None,
    }


# --------------------------------------------------------------------------
# Paused terminal (pty, not tmux): the pty is not read for N seconds
# --------------------------------------------------------------------------

def kill_group(proc: subprocess.Popen) -> None:
    """Kill the app and anything it started (the run used start_new_session)."""
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    proc.wait(timeout=5)


def fionread(fd: int) -> int:
    return struct.unpack("i", fcntl.ioctl(fd, termios.FIONREAD, b"\0\0\0\0"))[0]


def drain(fd: int, seconds: float) -> bytes:
    chunks = []
    deadline = time.monotonic() + seconds
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        ready, _, _ = select.select([fd], [], [], min(0.1, remaining))
        if ready:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            chunks.append(data)
    return b"".join(chunks)


def run_paused(args: argparse.Namespace, binary: Path, root: Path, provider: WatchedProvider, out: Path) -> dict[str, Any]:
    home, work = write_project(root, provider.port)
    log = out / "perf.jsonl"
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", args.rows, args.cols, 0, 0))
    env = base_env(home, {})
    proc = subprocess.Popen(
        hoo_argv(binary, log, []),
        stdin=slave,
        stdout=slave,
        stderr=slave,
        cwd=work,
        env=env,
        start_new_session=True,
        close_fds=True,
    )
    os.close(slave)
    transcript = bytearray()
    try:
        deadline = time.monotonic() + 60
        while PROMPT_BYTES not in transcript:
            if time.monotonic() > deadline or proc.poll() is not None:
                raise RuntimeError("the prompt never appeared in the pty")
            transcript += drain(master, 0.2)
        time.sleep(1.0)
        os.write(master, PROMPT_TEXT.encode("utf-8") + b"\r")

        # Keep reading until the watched turn is in flight (the bash output comes first).
        deadline = time.monotonic() + args.turn_timeout
        while not provider.hang_started.is_set():
            if time.monotonic() > deadline:
                raise RuntimeError("the watched turn never started")
            transcript += drain(master, 0.2)
        # The terminal stops reading. The app's writes now block once the pty is full.
        queued_start = fionread(master)
        pause_until = time.monotonic() + args.pause_seconds
        while time.monotonic() < pause_until:
            time.sleep(0.1)
        queued_end = fionread(master)

        # Ctrl+C, with the terminal still not read. The abort must reach the client.
        t0 = time.monotonic()
        os.write(master, b"\x03")
        aborted = provider.disconnect_event.wait(timeout=5.0)
        latency_ms = round((provider.disconnected_at - t0) * 1000.0, 1) if aborted else None

        # Start reading again: the backlog drains and the app should show its prompt.
        tail = drain(master, 4.0)
        transcript += tail
        prompt_back = PROMPT_BYTES in tail
        (out / "pty-after-abort.bin").write_bytes(bytes(transcript[-65536:]))

        os.write(master, b"/quit\r")
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            kill_group(proc)
        return {
            "pause_s": args.pause_seconds,
            "pty_bytes_queued_at_pause_start": queued_start,
            "pty_bytes_queued_at_pause_end": queued_end,
            "ctrl_c_to_client_disconnect_ms": latency_ms,
            "abort_seen": aborted,
            "abort_within_500ms": latency_ms is not None and latency_ms < ABORT_TARGET_MS,
            "prompt_back_after_drain": prompt_back,
            "exit_code": proc.returncode,
        }
    finally:
        if proc.poll() is None:
            kill_group(proc)
        os.close(master)


# --------------------------------------------------------------------------
# Results
# --------------------------------------------------------------------------

def read_log(log: Path) -> list[dict[str, Any]]:
    if not log.exists():
        return []
    return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines() if line.strip()]


def summarize(lines: list[dict[str, Any]]) -> dict[str, Any]:
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
        "worst_rolling_p99_ms": p99_peak,
        "final_window": {key: last.get(key) for key in ("frame_build_ms", "frame_write_ms", "key_latency_ms", "loop_iteration_ms")},
    }


def dist_row(name: str, dist: dict[str, Any] | None) -> str:
    if not dist:
        return f"  {name:<24} no samples"
    return (
        f"  {name:<24} n={dist['n']:<6} min {dist['min']:8.2f}  p50 {dist['p50']:8.2f}  p99 {dist['p99']:8.2f}  (ms)"
    )


def verdict(ok: bool | None) -> str:
    if ok is None:
        return "NOT MEASURED"
    return "PASS" if ok else "MISS"


def print_turn_summary(name: str, summary: dict[str, Any], out: Path) -> None:
    whole = summary["keys_typed"] <= KEY_WINDOW
    print()
    print(f"[{name}] perf log: {out / 'perf.jsonl'} ({summary['samples_seconds']} lines, one a second)")
    print(f"keys typed: {summary['keys_typed']} over {summary['typing_window_s']} s; turn done at {summary['turn_done_after_s']} s")
    print(
        f"key window: the last {KEY_WINDOW} keys. This run typed {summary['keys_typed']}, "
        + ("so the figures cover the whole run." if whole else "so the figures cover only the last 1,024 keys.")
    )
    print(f"frames: {summary['frames']}   stalls (loop > 500 ms): {summary['stalls_over_500ms']}")
    print(f"threads: max {summary['threads_max']} (ceiling {THREAD_CEILING}), last {summary['threads_last']}")
    print(f"rss: max {summary['rss_mib_max']} MiB, last {summary['rss_mib_last']} MiB")
    final = summary["final_window"]
    print("rolling window (the last 1024 samples at the end of the run):")
    print(dist_row("frame build", final["frame_build_ms"]))
    print(dist_row("frame write", final["frame_write_ms"]))
    print(dist_row("keystroke to frame", final["key_latency_ms"]))
    print(dist_row("loop iteration", final["loop_iteration_ms"]))
    peaks = summary["worst_rolling_p99_ms"]
    print("worst rolling-window p99 at any sample (ms): " + ", ".join(f"{k.removesuffix('_ms')} {v:.2f}" for k, v in peaks.items() if v is not None))
    key_p99 = (final["key_latency_ms"] or {}).get("p99")
    key_ok = (key_p99 < KEY_P99_TARGET_MS) if (whole and key_p99 is not None) else None
    print(f"  target keystroke-to-frame p99 < {KEY_P99_TARGET_MS:.0f} ms, whole run: {key_p99} ms -> {verdict(key_ok)}")
    threads = summary["threads_max"]
    thread_ok = None if threads is None else threads <= THREAD_CEILING
    print(f"  target thread ceiling <= {THREAD_CEILING}: {threads} -> {verdict(thread_ok)}")
    summary["targets"] = {
        "keys_typed": summary["keys_typed"],
        "keystroke_window": "whole run" if whole else f"last {KEY_WINDOW}",
        "keystroke_p99_ms": key_p99,
        "keystroke_p99_pass": key_ok,
        "threads_max": threads,
        "thread_ceiling_pass": thread_ok,
    }


def run_turn_scenario(name: str, args: argparse.Namespace, routes: dict[str, Route], typing_seconds: float,
                      extra_args: list[str], mcp: bool, out: Path, binary: Path,
                      session_turns: int = 0) -> dict[str, Any]:
    root = Path(tempfile.mkdtemp(prefix=f"perf-{name}-"))
    provider = MockProvider(routes, log_path=str(root / "requests.jsonl"))
    port = provider.start()
    socket = f"perf-{name}-{os.getpid()}"
    try:
        home, work = write_project(root, port, mcp=mcp)
        if session_turns:
            session_file = root / "history.jsonl"
            write_session(session_file, work, session_turns)
            extra_args = [*extra_args, "--session", str(session_file)]
        drive_info = drive_tmux(socket, args, out, typing_seconds, extra_args, binary, home, work)
        summary = summarize(read_log(out / "perf.jsonl"))
        summary.update(drive_info)
        summary["parent_requests"] = len(provider.requests("parent"))
        child_records = provider.requests("child")
        summary["child_requests"] = len(child_records)
        # A child that ran out of script answers 500, and its record says so ("error").
        summary["child_answers_ok"] = sum(1 for r in child_records if "error" not in r.get("served", []))
        if mcp:
            accepted = root / "mcp-accepted.jsonl"
            summary["mcp_calls_accepted"] = len(accepted.read_text(encoding="utf-8").splitlines()) if accepted.exists() else 0
    finally:
        if not args.keep:
            tmux(socket, "kill-server", check=False)
        provider.stop()
        shutil.rmtree(root, ignore_errors=True)
    print_turn_summary(name, summary, out)
    if name == "load":
        accepted = summary["mcp_calls_accepted"]
        turn_s = summary["turn_done_after_s"]
        mcp_pass = accepted == 1 and turn_s >= 60.0
        children_pass = summary["child_answers_ok"] == SUBAGENTS
        summary["gates"] = {
            "mcp_stall": {"calls_accepted": accepted, "turn_done_after_s": turn_s, "pass": mcp_pass},
            "subagents": {"successful_answers": summary["child_answers_ok"], "required": SUBAGENTS, "pass": children_pass},
        }
        print(f"  MCP stall: {accepted} call(s) accepted, turn ended at {turn_s} s (gate: 1 call and >= 60 s) -> {verdict(mcp_pass)}")
        print(f"  subagents: {summary['child_answers_ok']} of {SUBAGENTS} answered, {summary['child_requests']} child requests (gate: {SUBAGENTS} of {SUBAGENTS}) -> {verdict(children_pass)}")
    return summary


def run_paused_scenario(args: argparse.Namespace, binary: Path, out: Path) -> dict[str, Any]:
    root = Path(tempfile.mkdtemp(prefix="perf-paused-"))
    routes = {
        "parent": Route("parent", [
            first_turn(bash_calls(args.calls, args.lines)),
            {"text": "Waiting on the terminal.", "watch_disconnect": True, "usage": usage()},
            done_turn(),
        ]),
    }
    provider = WatchedProvider(routes, log_path=str(root / "requests.jsonl"))
    provider.port = provider.start()
    try:
        result = run_paused(args, binary, root, provider, out)
    finally:
        provider.stop()
        shutil.rmtree(root, ignore_errors=True)
    lines = read_log(out / "perf.jsonl")
    summary = summarize(lines)
    final = summary["final_window"]
    fb = (final["frame_build_ms"] or {}).get("p99")
    lp = (final["loop_iteration_ms"] or {}).get("p99")
    result["frame_build_p99_ms"] = fb
    result["loop_iteration_p99_ms"] = lp
    summary["paused"] = result
    print()
    print(f"[paused-pty] perf log: {out / 'perf.jsonl'} ({len(lines)} lines)")
    print(f"  pty bytes queued: {result['pty_bytes_queued_at_pause_start']} at pause start, {result['pty_bytes_queued_at_pause_end']} at pause end (sampled twice; about one pty buffer means the app was probably blocked on output)")
    print(f"  frame build p99 {fb} ms, loop iteration p99 {lp} ms (rolling window at the end of the paused run)")
    print(f"  Ctrl+C to client disconnect: {result['ctrl_c_to_client_disconnect_ms']} ms (abort seen: {result['abort_seen']})")
    print(f"  target Ctrl+C aborts within {ABORT_TARGET_MS:.0f} ms: {verdict(result['abort_within_500ms'])}")
    print(f"  prompt back after the backlog drained: {result['prompt_back_after_drain']}; exit code {result['exit_code']}")
    summary["targets"] = {"abort_ms": result["ctrl_c_to_client_disconnect_ms"], "abort_pass": result["abort_within_500ms"]}
    return summary


def run_memory_scenario(args: argparse.Namespace, binary: Path, out: Path) -> dict[str, Any]:
    root = Path(tempfile.mkdtemp(prefix="perf-memory-"))
    provider = MockProvider({"parent": Route("parent", [done_turn()])}, log_path=str(root / "requests.jsonl"))
    port = provider.start()
    socket = f"perf-memory-{os.getpid()}"
    try:
        result = run_memory(socket, args, out, binary, root, port)
    finally:
        if not args.keep:
            tmux(socket, "kill-server", check=False)
        provider.stop()
        shutil.rmtree(root, ignore_errors=True)
    lines = read_log(out / "perf.jsonl")
    summary = summarize(lines)
    summary["memory"] = result
    shed_ok = bool(
        result["shed_seen"]
        and result["shed_notice_lifted"]
        and not result["shedding_notice_before_alloc"]
        and result["allocation_state_at_shed"] == "allocated"
    )
    summary["targets"] = {"shed_then_recover_pass": shed_ok}
    print()
    print(f"[memory] perf log: {out / 'perf.jsonl'} ({len(lines)} lines), rss max {summary['rss_mib_max']} MiB")
    print(f"  forced allocation: {result['alloc_mib']} MiB, soft limit {result['soft_limit_mb']} MiB")
    print(f"  shedding notice shown while resident: {result['shed_seen']}; allocation state then: {result['allocation_state_at_shed']}")
    print(f"  shedding notice lifted after release: {result['shed_notice_lifted']}; final state: {result['allocation_state_final']}")
    print(f"  target shed, then recover: {verdict(shed_ok)}")
    return summary


# --------------------------------------------------------------------------
# Main
# --------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--scenario", choices=["bash", "load", "stream", "long-transcript", "paused-pty", "memory", "all"], default="all")
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY, help="hoocode binary (default: $CARGO_TARGET_DIR/release/hoocode)")
    parser.add_argument("--calls", type=int, default=8, help="parallel bash calls in the turn (default 8)")
    parser.add_argument("--lines", type=int, default=5000, help="lines each bash call prints (default 5000)")
    parser.add_argument("--keys-per-second", type=float, default=None, help="typing rate (default 20; load 13, so the whole run fits the 1,024-key window)")
    parser.add_argument("--typing-seconds", type=float, default=None,
                        help="minimum time keys are typed (default 15 for bash; 75 for load, past the MCP 60 s deadline)")
    parser.add_argument("--turn-timeout", type=float, default=150.0, help="seconds to wait for the turn to finish")
    parser.add_argument("--pause-seconds", type=float, default=5.0, help="paused-pty: seconds the terminal is not read (default 5)")
    parser.add_argument("--alloc-mb", type=int, default=3072, help="memory: forced allocation in MiB (default 3072 = 3 GiB)")
    parser.add_argument("--memory-settle", type=float, default=3.0, help="memory: seconds before the allocation trigger (default 3)")
    parser.add_argument("--memory-hold", type=float, default=6.0, help="memory: seconds the memory stays resident (default 6)")
    parser.add_argument("--memory-timeout", type=float, default=60.0, help="memory: seconds to wait for the notice to appear or lift")
    parser.add_argument("--transcript-turns", type=int, default=TRANSCRIPT_TURNS, help="long-transcript: generated turns in the resumed session (default 200)")
    parser.add_argument("--cols", type=int, default=120)
    parser.add_argument("--rows", type=int, default=40)
    parser.add_argument("--out", type=Path, default=None, help="artifact directory (default $CARGO_TARGET_DIR/perf/load-<time>)")
    parser.add_argument("--keep", action="store_true", help="leave the tmux servers running on exit")
    args = parser.parse_args()

    binary = args.binary.resolve()
    if not binary.exists():
        print(f"error: {binary} does not exist; run `cargo build -p hoocode-code-main --release` first", file=sys.stderr)
        return 2
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_root = (args.out or OUT_ROOT / f"load-{stamp}").resolve()
    out_root.mkdir(parents=True, exist_ok=True)

    wanted = ["bash", "load", "stream", "long-transcript", "paused-pty", "memory"] if args.scenario == "all" else [args.scenario]
    # Each scenario starts from the user's rate (or its own default), not the last one's.
    user_keys_per_second = args.keys_per_second
    results: dict[str, Any] = {"binary": str(binary), "started": stamp}
    for name in wanted:
        out = out_root / name
        out.mkdir(parents=True, exist_ok=True)
        if name == "bash":
            routes = {"parent": Route("parent", [first_turn(bash_calls(args.calls, args.lines)), done_turn(), done_turn(), done_turn()])}
            args.keys_per_second = user_keys_per_second or 20.0
            typing = args.typing_seconds or 15.0
            results[name] = run_turn_scenario(name, args, routes, typing, [], False, out, binary)
        elif name == "load":
            calls = bash_calls(args.calls, args.lines) + agent_calls(SUBAGENTS) + mcp_calls()
            routes = {
                "parent": Route("parent", [first_turn(calls), done_turn(), done_turn(), done_turn()]),
                # Five children; a child retries on a 500, so give the route room for
                # retries. A route that runs out answers 500 and the child fails.
                "child": Route("child", [child_turn() for _ in range(40)], marker=CHILD_MARKER),
            }
            args.keys_per_second = user_keys_per_second or LOAD_KEYS_PER_SECOND
            typing = args.typing_seconds or 75.0
            results[name] = run_turn_scenario(name, args, routes, typing, ["--enable-subagents"], True, out, binary)
        elif name in ("stream", "long-transcript"):
            pieces = [f"word{i} " + ("lorem ipsum " if i % 7 else "\n\n") for i in range(STREAM_CHUNKS)]
            body = {"text_chunks": pieces[:-1] + [TURN_DONE], "chunk_delay_s": STREAM_DELAY_S, "usage": usage()}
            routes = {"parent": Route("parent", [body, done_turn(), done_turn()])}
            args.keys_per_second = user_keys_per_second or 20.0
            typing = args.typing_seconds or 15.0
            turns = args.transcript_turns if name == "long-transcript" else 0
            results[name] = run_turn_scenario(name, args, routes, typing, [], False, out, binary, session_turns=turns)
        elif name == "paused-pty":
            results[name] = run_paused_scenario(args, binary, out)
        elif name == "memory":
            results[name] = run_memory_scenario(args, binary, out)

    (out_root / "summary.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    print()
    print(f"summary: {out_root / 'summary.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
