#!/usr/bin/env python3
"""Detached-session evals (SA-12): the handover, the park, the lock and the limits.

Every scenario runs the **real binary** end to end against the scripted mock provider
(`scripts/eval/mock_provider.py`). A parent session calls `SessionSpawn`, the harness
starts the detached child process, and the child's requests are routed by a marker the
handover prompt carries (`continuing work handed over`). The TUI scenarios drive the
real interactive binary in tmux (`scripts/tui/goldens.py` provides the tmux driver).

    scripts/eval/detached_evals.py                       # the whole suite
    scripts/eval/detached_evals.py --only handover_spawn
    scripts/eval/detached_evals.py --keep                # keep the sandboxes of passing runs too

Exit code is 0 only when every scenario passes. A skipped scenario (tmux missing) is
reported with its reason and is not a pass, so the exit code is non-zero.

Reports land in `target/detached-evals/`: `report.json`, `report.md`, and one directory per
scenario under `runs/<stamp>/<scenario>/` with the process outputs, the request logs, the
first request of each route, the session files and the logs, and the tmux pane captures.
A failing scenario keeps its sandbox; a passing one loses it unless `--keep` is given.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO / "scripts" / "tui"))

from mock_provider import MockProvider, Route, last_user_text  # noqa: E402
from goldens import Tmux  # noqa: E402

DEFAULT_BIN = REPO / "target" / "debug" / "hoocode"
OUT_DIR = REPO / "target" / "detached-evals"

MODEL = "mock-model"
CONFIG_DIR = ".hoocode"
# The child's first prompt carries this sentence (`detached.rs` `child_prompt`). Routing on
# it sends every detached child's requests to its own script, never to the parent's.
HANDOVER_MARK = "continuing work handed over"
GOAL = "Finish the handover test by writing done.txt."
BRIEF = (
    "## Goal\n"
    f"{GOAL}\n\n"
    "## Decisions\n"
    "Use the mock provider. Keep the Write tool for the output file.\n\n"
    "## Dead ends\n"
    "Do not use approach X: it hangs on the mock.\n\n"
    "## Next step\n"
    "Write done.txt containing ok, then report done.\n"
)
HANDOVER_PROMPT = (
    "You are continuing work handed over from another hoocode session. "
    "The brief below was approved by the user. Do the Next step.\n\n" + BRIEF
)
PARENT_PROMPT = "Hand the next step to a fresh session."
PARKED_PROBE = "parked_probe.txt"
PARK_SHELL = {"name": "Shell", "arguments": {"command": f"touch {PARKED_PROBE}"}}

SETTINGS = {
    "defaultProvider": "mock",
    "defaultModel": MODEL,
    "enableSubagent": True,
    "maxSubagentDepth": 1,
    # The scripted turns count requests; a naming call would take one.
    "autoNameSessions": False,
}


class Skip(Exception):
    """A scenario that cannot run here. Reported with its reason."""


# --------------------------------------------------------------------------
# Provider: routes by the first user message, keeps each route's first request.
# --------------------------------------------------------------------------


class DetachedProvider(MockProvider):
    def __init__(self, routes: dict[str, Route], log_path: str):
        super().__init__(routes, log_path=log_path, default="parent")
        self.first_bodies: dict[str, dict[str, Any]] = {}
        self._first_lock = threading.Lock()

    def pick_route(self, body: dict[str, Any]) -> str:
        users = [m for m in body.get("messages") or [] if m.get("role") == "user"][:1]
        first = last_user_text({"messages": users})
        for route in self.routes.values():
            if route.marker and route.marker in first:
                return route.name
        return self.default

    def record(self, route: str, turn: dict[str, Any], body: dict[str, Any]) -> None:
        super().record(route, turn, body)
        with self._first_lock:
            self.first_bodies.setdefault(route, body)

    def first_text(self, route: str) -> str:
        with self._first_lock:
            return json.dumps(self.first_bodies.get(route, {}), ensure_ascii=False)


def make_provider(c: "Ctx", label: str, routes: dict[str, Route]) -> tuple[DetachedProvider, int]:
    provider = DetachedProvider(routes, log_path=str(c.run_dir / f"requests-{label}.jsonl"))
    port = provider.start()
    c.providers.append(provider)
    return provider, port


# --------------------------------------------------------------------------
# Sandbox, process and file helpers
# --------------------------------------------------------------------------


def models_json(port: int) -> dict[str, Any]:
    return {
        "providers": {
            "mock": {
                "baseUrl": f"http://127.0.0.1:{port}/v1",
                "api": "openai-completions",
                "apiKey": "mock-key",
                "models": [{"id": MODEL, "name": "Mock", "contextWindow": 128000, "maxTokens": 4096}],
            }
        }
    }


def sandbox(root: Path, port: int, auto_allow: list[str] | None = None) -> tuple[Path, Path, Path]:
    """An isolated HOME, the project (cwd) and the session folder. Returns (home, work, sessions)."""
    home, work, sessions = root / "home", root / "work", root / "sessions"
    cfg = home / CONFIG_DIR
    cfg.mkdir(parents=True, exist_ok=True)
    work.mkdir(parents=True, exist_ok=True)
    sessions.mkdir(parents=True, exist_ok=True)
    (work / "marker.txt").write_text("hello\n", encoding="utf-8")
    (cfg / "models.json").write_text(json.dumps(models_json(port)), encoding="utf-8")
    (cfg / "settings.json").write_text(json.dumps(SETTINGS), encoding="utf-8")
    if auto_allow:
        config = {"modes": {"build": {"auto_allow": auto_allow}}}
        (cfg / "hoo-config.json").write_text(json.dumps(config), encoding="utf-8")
    return home, work, sessions


def env_for(home: Path, tui: bool = False, detached: bool = False) -> dict[str, str]:
    # Drop HOOCODE_* from the developer's shell, then set what the run needs.
    env = {k: v for k, v in os.environ.items() if not k.startswith("HOOCODE_")}
    env.update({"HOME": str(home), "LANG": "en_US.UTF-8"})
    if tui:
        env["TERM"] = "xterm-256color"
    else:
        env.update({"TERM": "dumb", "NO_COLOR": "1"})
    if detached:
        env["HOOCODE_DETACHED"] = "1"
    return env


def base_argv(binary: Path) -> list[str]:
    return [str(binary), "--offline", "--provider", "mock", "--model", MODEL]


def _text(value: Any) -> str:
    if value is None:
        return ""
    return value.decode("utf-8", "replace") if isinstance(value, bytes) else value


def run(argv: list[str], env: dict[str, str], cwd: Path, timeout: float) -> tuple[int | None, str, str]:
    """Runs to completion. Returns (exit code or None on timeout, stdout, stderr)."""
    try:
        done = subprocess.run(argv, env=env, cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return done.returncode, done.stdout, done.stderr
    except subprocess.TimeoutExpired as expired:
        return None, _text(expired.stdout), _text(expired.stderr) + f"\n[timed out after {timeout}s]"


def spawn(argv: list[str], env: dict[str, str], cwd: Path, log: Path) -> subprocess.Popen:
    """A background process in its own process group, stdout and stderr to `log`."""
    with open(log, "ab") as out:
        return subprocess.Popen(
            argv,
            env=env,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=out,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )


def entries(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    out: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            pass  # a line still being written
    return out


def header(path: Path) -> dict[str, Any]:
    found = entries(path)
    return found[0] if found and found[0].get("type") == "session" else {}


def customs(path: Path, custom_type: str) -> list[dict[str, Any]]:
    return [
        e.get("data") or {}
        for e in entries(path)
        if e.get("type") == "custom" and e.get("customType") == custom_type
    ]


def detached_exit(path: Path) -> dict[str, Any] | None:
    found = customs(path, "detached_exit")
    return found[-1] if found else None


def wait_exit(path: Path, timeout: float) -> dict[str, Any] | None:
    wait_until(lambda: detached_exit(path) is not None, timeout)
    return detached_exit(path)


def lock_file(path: Path) -> Path:
    return Path(str(path) + ".lock")


def lock_pid(path: Path) -> int | None:
    lock = lock_file(path)
    try:
        text = lock.read_text(encoding="utf-8").strip()
    except OSError:
        return None
    return int(text) if text.isdigit() else None


def lock_held(path: Path) -> bool:
    """Whether a process holds the session's flock. Probes it and releases it again."""
    lock = lock_file(path)
    if not lock.exists():
        return False
    fd = os.open(lock, os.O_RDWR)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    finally:
        os.close(fd)


def same_path(a: Any, b: Path) -> bool:
    return isinstance(a, str) and os.path.realpath(a) == os.path.realpath(b)


def wait_until(pred: Callable[[], bool], timeout: float, interval: float = 0.2) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(interval)
    return pred()


def pane_wait(tmux: Tmux, pattern: str, timeout: float) -> tuple[bool, str]:
    rx = re.compile(pattern, re.M)
    screen = ""
    end = time.time() + timeout
    while time.time() < end:
        screen = tmux.capture()
        if rx.search(screen):
            return True, screen
        if tmux.dead():
            break
        time.sleep(0.2)
    return False, screen


def start_tui(c: "Ctx", label: str, binary: Path, home: Path, work: Path, session: Path) -> Tmux:
    if not shutil.which("tmux"):
        raise Skip("tmux is not installed")
    tmux = Tmux(f"det-{label}-{os.getpid()}", cols=120, rows=40)
    c.tmuxes.append(tmux)
    tmux.start([*base_argv(binary), "--session", str(session)], cwd=work, env=env_for(home, tui=True))
    return tmux


def first_session(sessions: Path, exclude: Path | None = None) -> Path | None:
    found = [p for p in sessions.glob("*.jsonl") if p != exclude]
    return found[0] if len(found) == 1 else None


# --------------------------------------------------------------------------
# Per-scenario context
# --------------------------------------------------------------------------


@dataclass
class Ctx:
    name: str
    run_dir: Path
    binary: Path
    failures: list[str] = field(default_factory=list)
    facts: dict[str, Any] = field(default_factory=dict)
    providers: list[DetachedProvider] = field(default_factory=list)
    tmuxes: list[Tmux] = field(default_factory=list)

    def check(self, condition: Any, message: str) -> None:
        if not condition:
            self.failures.append(message)

    def save(self, rel: str, data: str) -> None:
        path = self.run_dir / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(data, encoding="utf-8")

    def save_proc(self, tag: str, rc: int | None, out: str, err: str) -> None:
        self.save(f"{tag}-stdout.txt", out)
        self.save(f"{tag}-stderr.txt", err)
        self.facts[f"{tag}_exit_code"] = rc

    def close(self) -> None:
        for provider in self.providers:
            provider.stop()
        for tmux in self.tmuxes:
            tmux.kill()


def requests_of(provider: DetachedProvider, route: str) -> list[dict[str, Any]]:
    return provider.requests(route)


def tools_of(entry: dict[str, Any]) -> list[str]:
    return [name for name in entry.get("tools", []) if name]


# --------------------------------------------------------------------------
# Scenarios
# --------------------------------------------------------------------------


def s_handover_spawn(c: Ctx) -> None:
    """A parent's SessionSpawn starts a child with the brief; the child finishes unattended."""
    routes = {
        "parent": Route(
            "parent",
            [
                {"text": "", "tool_calls": [{"name": "SessionSpawn", "arguments": {"brief": BRIEF}}]},
                {"text": "Handed the next step to a detached session."},
            ],
        ),
        "child": Route(
            "child",
            [
                {"tool_calls": [{"name": "Write", "arguments": {"path": "done.txt", "content": "ok\n"}}]},
                {"text": "Wrote done.txt."},
            ],
            marker=HANDOVER_MARK,
        ),
    }
    provider, port = make_provider(c, "main", routes)
    home, work, sessions = sandbox(c.run_dir / "sandbox", port, auto_allow=["Write"])
    parent = sessions / "parent.jsonl"
    rc, out, err = run(
        [*base_argv(c.binary), "--mode", "json", "-p", PARENT_PROMPT, "--session", str(parent)],
        env_for(home),
        work,
        timeout=120,
    )
    c.save_proc("parent", rc, out, err)
    c.check(rc == 0, f"parent run exited {rc}")

    child = first_session(sessions, exclude=parent)
    c.check(child is not None, f"expected one child session file, found {sorted(p.name for p in sessions.iterdir())}")
    if child is None:
        return
    exit_record = wait_exit(child, 30)
    c.facts["child_exit"] = exit_record
    c.check(exit_record and exit_record.get("status") == "done", f"child detached_exit is {exit_record}, expected done")

    c.check(same_path(header(child).get("parentSession"), parent), f"parentSession is {header(child).get('parentSession')!r}, expected {parent}")
    infos = [e for e in entries(child) if e.get("type") == "session_info"]
    c.check(infos and infos[0].get("name") == GOAL, f"session name is {infos[0].get('name') if infos else None!r}, expected the Goal line")
    c.check(infos and infos[0].get("source") == "auto", "session name source is not auto")

    first = provider.first_text("child")
    c.save("first-request-child.json", first)
    for needle in ["Use the mock provider", "Do not use approach X", "## Facts (added by hoocode)"]:
        c.check(needle in first, f"child's first request lacks {needle!r}")

    c.check((work / "done.txt").is_file() and (work / "done.txt").read_text().strip() == "ok", "done.txt is missing or not 'ok'")

    child_log = requests_of(provider, "child")
    c.check(child_log and all("SessionSpawn" not in e.get("tool_calls", []) for e in child_log), "the child made a SessionSpawn call")
    c.check(child_log and "SessionSpawn" not in tools_of(child_log[0]), "the child's request tool list offers SessionSpawn")

    parent_log = requests_of(provider, "parent")
    c.check(parent_log and "SessionSpawn" in tools_of(parent_log[0]), "the parent's request tool list lacks SessionSpawn")
    c.check(len(parent_log) == 2, f"parent made {len(parent_log)} request(s), expected 2")
    tool_text = parent_log[1].get("last_tool", "") if len(parent_log) > 1 else ""
    c.save("parent-tool-result.txt", tool_text)
    c.check(f"hoocode --session {child}" in tool_text, "the parent's tool result does not name the child's path")
    c.check(not lock_held(child) and not lock_held(parent), "a lock is still held at the end")
    c.facts["child_file"] = str(child)


def _park_child(c: Ctx, label: str) -> tuple[DetachedProvider, Path, Path, Path, int | None]:
    """A detached child that calls an ungated Shell and parks on it. Returns the parked session."""
    routes = {
        "child": Route(
            "child",
            [{"tool_calls": [PARK_SHELL]}, {"text": "Probe ran; done."}],
            marker=HANDOVER_MARK,
        )
    }
    provider, port = make_provider(c, label, routes)
    root = c.run_dir / label
    home, work, sessions = sandbox(root / "sandbox", port)
    path = sessions / "parked.jsonl"
    rc, out, err = run(
        [*base_argv(c.binary), "-p", HANDOVER_PROMPT, "--session", str(path)],
        env_for(home, detached=True),
        work,
        timeout=60,
    )
    c.save_proc(f"{label}-park", rc, out, err)
    c.check(rc == 3, f"parked child exited {rc}, expected 3")
    parked = customs(path, "parked_call")
    c.check(len(parked) == 1 and parked[0].get("toolName") == "Shell", f"parked_call records are {parked}")
    c.check(parked and bool(parked[0].get("toolCallId")), "parked_call has no toolCallId")
    exit_record = detached_exit(path)
    c.check(exit_record and exit_record.get("status") == "parked", f"detached_exit is {exit_record}, expected parked")
    c.check(not (work / PARKED_PROBE).exists(), "the parked Shell call ran before the answer")
    c.check(len(requests_of(provider, "child")) == 1, "the child made more than one request before parking")
    return provider, home, work, path, rc


def s_park_then_resume_tui(c: Ctx) -> None:
    """Parked call, opened in the TUI, approved with `y`: the call runs and the run continues."""
    provider, home, work, path, _ = _park_child(c, "park")
    tmux = start_tui(c, "yes", c.binary, home, work, path)
    shown, screen = pane_wait(tmux, r"approve and detach", 30)
    c.save("tui-card.txt", screen)
    c.check(shown, "the parked-call card never appeared")
    if not shown:
        return
    tmux.send_text("y")
    ran = wait_until(lambda: (work / PARKED_PROBE).exists(), 30)
    c.check(ran, f"{PARKED_PROBE} did not appear after y")
    continued = wait_until(lambda: len(requests_of(provider, "child")) >= 2, 30)
    c.check(continued, "no next child turn was requested after y")
    c.check(len(requests_of(provider, "child")) == 2, f"child made {len(requests_of(provider, 'child'))} request(s), expected 2")
    _, after = pane_wait(tmux, r"Probe ran", 15)
    c.save("tui-after-yes.txt", after)


def s_park_then_detach_tui(c: Ctx) -> None:
    """Parked call, opened in the TUI, `d` (approve and detach): the call runs, a new detached process finishes."""
    provider, home, work, path, _ = _park_child(c, "detach")
    tmux = start_tui(c, "detach", c.binary, home, work, path)
    shown, screen = pane_wait(tmux, r"approve and detach", 30)
    c.save("tui-card.txt", screen)
    c.check(shown, "the parked-call card never appeared")
    if not shown:
        return
    tmux.send_text("d")
    ran = wait_until(lambda: (work / PARKED_PROBE).exists(), 30)
    c.check(ran, f"{PARKED_PROBE} did not appear after d")
    exit_record = wait_exit(path, 40)
    c.facts["final_exit"] = exit_record
    c.check(exit_record and exit_record.get("status") == "done", f"the detached continuation's detached_exit is {exit_record}, expected done")
    exits = [e.get("status") for e in customs(path, "detached_exit")]
    c.check(exits[-1:] == ["done"], f"detached_exit history is {exits}")
    c.check(len(requests_of(provider, "child")) == 2, f"child made {len(requests_of(provider, 'child'))} request(s), expected 2")
    c.check(not lock_held(path), "the lock is still held after the continuation ended")


def s_follow_view_while_running(c: Ctx) -> None:
    """The TUI opened on a running detached session follows it, and says so when it ends."""
    routes = {"child": Route("child", [{"text": "Still working, then done.", "delay_s": 4}], marker=HANDOVER_MARK)}
    provider, port = make_provider(c, "main", routes)
    home, work, sessions = sandbox(c.run_dir / "sandbox", port)
    path = sessions / "follow.jsonl"
    proc = spawn(
        [*base_argv(c.binary), "-p", HANDOVER_PROMPT, "--session", str(path)],
        env_for(home, detached=True),
        work,
        c.run_dir / "child.log",
    )
    c.check(wait_until(lambda: lock_held(path), 15), "the running child never took the lock")
    tmux = start_tui(c, "follow", c.binary, home, work, path)
    shown, screen = pane_wait(tmux, rf"following pid {proc.pid}\b", 15)
    c.save("tui-following.txt", screen)
    c.check(shown, f"the TUI did not show 'following pid {proc.pid}'")
    try:
        proc.wait(timeout=60)
    except subprocess.TimeoutExpired:
        proc.kill()
        c.check(False, "the child did not end within 60s")
    c.check(proc.returncode == 0, f"child exited {proc.returncode}")
    ended, screen = pane_wait(tmux, rf"Process {proc.pid} ended; the session is yours", 20)
    c.save("tui-ended.txt", screen)
    c.check(ended, f"the TUI did not show 'Process {proc.pid} ended; the session is yours'")
    c.check(detached_exit(path) and detached_exit(path).get("status") == "done", "the child's detached_exit is not done")


def s_lock_refuses_second_writer(c: Ctx) -> None:
    """While a child holds its session, a second writer is refused before any run starts."""
    routes = {"child": Route("child", [{"text": "Finished.", "delay_s": 6}], marker=HANDOVER_MARK)}
    provider, port = make_provider(c, "main", routes)
    home, work, sessions = sandbox(c.run_dir / "sandbox", port)
    path = sessions / "locked.jsonl"
    proc = spawn(
        [*base_argv(c.binary), "-p", HANDOVER_PROMPT, "--session", str(path)],
        env_for(home, detached=True),
        work,
        c.run_dir / "child.log",
    )
    c.check(wait_until(lambda: lock_held(path), 15), "the running child never took the lock")
    rc, out, err = run([*base_argv(c.binary), "-p", "x", "--session", str(path)], env_for(home), work, timeout=30)
    c.save_proc("second-writer", rc, out, err)
    c.check(rc == 1, f"second writer exited {rc}, expected 1")
    c.check("Session is in use by process" in out + err, "the refusal does not say 'Session is in use by process'")
    c.check(len(requests_of(provider, "parent")) == 0, "the refused run made a model request")
    proc.wait(timeout=60)
    c.check(proc.returncode == 0, f"child exited {proc.returncode}")
    c.check(detached_exit(path) and detached_exit(path).get("status") == "done", "the child did not finish done")


def s_killed_child_leaves_no_lock(c: Ctx) -> None:
    """A SIGKILLed child leaves no lock: the file is free and a later run is not refused."""
    routes = {
        "child": Route(
            "child",
            [{"text": "Late answer.", "delay_s": 60}, {"text": "Resumed and finished."}],
            marker=HANDOVER_MARK,
        )
    }
    provider, port = make_provider(c, "main", routes)
    home, work, sessions = sandbox(c.run_dir / "sandbox", port)
    path = sessions / "killed.jsonl"
    proc = spawn(
        [*base_argv(c.binary), "-p", HANDOVER_PROMPT, "--session", str(path)],
        env_for(home, detached=True),
        work,
        c.run_dir / "child.log",
    )
    c.check(wait_until(lambda: lock_held(path), 15), "the running child never took the lock")
    c.facts["lock_pid"] = lock_pid(path)
    c.check(lock_pid(path) == proc.pid, f"lock names pid {lock_pid(path)}, the child is {proc.pid}")
    os.killpg(proc.pid, signal.SIGKILL)
    proc.wait(timeout=10)
    c.check(wait_until(lambda: not lock_held(path), 5), "the lock is still held after SIGKILL")
    c.check(detached_exit(path) is None, "a killed child wrote a detached_exit record")
    # The status the file implies with no lock and no exit record is `failed` ("exited without a result").
    c.facts["status_inputs"] = {"lock_held": lock_held(path), "detached_exit": detached_exit(path)}

    rc, out, err = run([*base_argv(c.binary), "-p", "--session", str(path)], env_for(home), work, timeout=60)
    c.save_proc("resume-after-kill", rc, out, err)
    c.check("Session is in use" not in out + err, "a later run was refused after the kill")
    c.check(rc == 0, f"the later run exited {rc}, expected 0")


def s_depth_one(c: Ctx) -> None:
    """A detached child is offered no SessionSpawn (and cannot spawn); a plain parent is offered it."""
    # A plain parent (not detached) is offered SessionSpawn.
    routes = {"parent": Route("parent", [{"text": "Plain answer."}])}
    provider, port = make_provider(c, "parent", routes)
    home, work, sessions = sandbox(c.run_dir / "parent-sandbox", port)
    path = sessions / "plain.jsonl"
    rc, out, err = run([*base_argv(c.binary), "--mode", "json", "-p", "Say hello.", "--session", str(path)], env_for(home), work, timeout=60)
    c.save_proc("plain-parent", rc, out, err)
    c.check(rc == 0, f"plain parent exited {rc}")
    parent_log = requests_of(provider, "parent")
    c.check(parent_log and "SessionSpawn" in tools_of(parent_log[0]), "the plain parent's request tool list lacks SessionSpawn")

    # A detached child asked to spawn anyway: it has no such tool, and the next step must not start a session.
    routes = {
        "child": Route(
            "child",
            [
                {"text": "", "tool_calls": [{"name": "SessionSpawn", "arguments": {"brief": BRIEF}}]},
                {"text": "I cannot spawn a session from here."},
            ],
            marker=HANDOVER_MARK,
        )
    }
    provider, port = make_provider(c, "child", routes)
    home, work, sessions = sandbox(c.run_dir / "child-sandbox", port)
    path = sessions / "child.jsonl"
    rc, out, err = run(
        [*base_argv(c.binary), "-p", HANDOVER_PROMPT, "--session", str(path)],
        env_for(home, detached=True),
        work,
        timeout=60,
    )
    c.save_proc("detached-child", rc, out, err)
    child_log = requests_of(provider, "child")
    c.check(child_log and "SessionSpawn" not in tools_of(child_log[0]), "the detached child's request tool list offers SessionSpawn")
    tool_text = child_log[1].get("last_tool", "") if len(child_log) > 1 else ""
    c.save("child-tool-result.txt", tool_text)
    c.check("Started a detached session" not in tool_text, "the detached child started a session")
    files = sorted(p.name for p in sessions.glob("*.jsonl"))
    c.check(files == ["child.jsonl"], f"session folder holds {files}, expected only the child's own file")


def s_turn_cap_exit(c: Ctx) -> None:
    """A detached run past its turn cap exits 1 with the reason in its detached_exit."""
    read = {"name": "Read", "arguments": {"path": "marker.txt"}}
    routes = {"child": Route("child", [{"tool_calls": [read]} for _ in range(5)], marker=HANDOVER_MARK)}
    provider, port = make_provider(c, "main", routes)
    home, work, sessions = sandbox(c.run_dir / "sandbox", port, auto_allow=["Read"])
    path = sessions / "capped.jsonl"
    rc, out, err = run(
        [*base_argv(c.binary), "-p", HANDOVER_PROMPT, "--session", str(path), "--max-turns", "2"],
        env_for(home, detached=True),
        work,
        timeout=60,
    )
    c.save_proc("capped", rc, out, err)
    c.check(rc == 1, f"capped run exited {rc}, expected 1")
    exit_record = detached_exit(path)
    c.facts["detached_exit"] = exit_record
    c.check(exit_record and exit_record.get("status") == "failed", f"detached_exit is {exit_record}, expected failed")
    c.check(exit_record and exit_record.get("reason") == "turn cap of 2 reached", f"reason is {exit_record and exit_record.get('reason')!r}")
    c.check(len(requests_of(provider, "child")) == 2, f"child made {len(requests_of(provider, 'child'))} request(s), expected 2")


def s_deadline_exit(c: Ctx) -> None:
    """A detached run past its deadline exits 1 with the reason in its detached_exit."""
    routes = {
        "child": Route(
            "child",
            [{"text": "Too late.", "delay_s": 5}, {"text": "Never reached."}],
            marker=HANDOVER_MARK,
        )
    }
    provider, port = make_provider(c, "main", routes)
    home, work, sessions = sandbox(c.run_dir / "sandbox", port)
    path = sessions / "deadline.jsonl"
    rc, out, err = run(
        [*base_argv(c.binary), "-p", HANDOVER_PROMPT, "--session", str(path), "--deadline-ms", "1500"],
        env_for(home, detached=True),
        work,
        timeout=60,
    )
    c.save_proc("deadline", rc, out, err)
    c.check(rc == 1, f"deadline run exited {rc}, expected 1")
    exit_record = detached_exit(path)
    c.facts["detached_exit"] = exit_record
    c.check(exit_record and exit_record.get("status") == "failed", f"detached_exit is {exit_record}, expected failed")
    c.check(exit_record and exit_record.get("reason") == "deadline of 1500 ms reached", f"reason is {exit_record and exit_record.get('reason')!r}")


SCENARIOS: list[tuple[str, Callable[[Ctx], None], str]] = [
    ("handover_spawn", s_handover_spawn, "SessionSpawn starts a child with the brief; the child finishes unattended."),
    ("park_then_resume_tui", s_park_then_resume_tui, "A parked call opened in the TUI and approved with y runs and the run continues."),
    ("park_then_detach_tui", s_park_then_detach_tui, "A parked call approved with d runs, and a new detached process finishes the run."),
    ("follow_view_while_running", s_follow_view_while_running, "The TUI follows a running detached session and reports when it ends."),
    ("lock_refuses_second_writer", s_lock_refuses_second_writer, "A second writer is refused while a child holds the session."),
    ("killed_child_leaves_no_lock", s_killed_child_leaves_no_lock, "A SIGKILLed child leaves no lock and a later run is not refused."),
    ("depth_one", s_depth_one, "A detached child is offered no SessionSpawn; a plain parent is offered it."),
    ("turn_cap_exit", s_turn_cap_exit, "A detached run past its turn cap exits 1 with the reason."),
    ("deadline_exit", s_deadline_exit, "A detached run past its deadline exits 1 with the reason."),
]


# --------------------------------------------------------------------------
# Harness
# --------------------------------------------------------------------------


def run_scenario(name: str, fn: Callable[[Ctx], None], run_dir: Path, binary: Path, keep: bool) -> dict[str, Any]:
    run_dir.mkdir(parents=True, exist_ok=True)
    c = Ctx(name=name, run_dir=run_dir, binary=binary)
    started = time.time()
    status = "passed"
    skip_reason = None
    try:
        fn(c)
    except Skip as skip:
        status = "skipped"
        skip_reason = str(skip)
    except Exception:  # a harness bug or an unexpected product crash: evidence, not a silent pass
        c.failures.append("exception: " + traceback.format_exc(limit=8))
    finally:
        c.close()
    duration = round(time.time() - started, 1)
    if status != "skipped" and c.failures:
        status = "failed"
    c.save("checks.json", json.dumps({"status": status, "failures": c.failures, "facts": c.facts}, indent=2, default=str))
    if status == "passed" and not keep:
        # Every sandbox (HOME, project and session folder) under the run dir; the evidence stays.
        for sandbox_dir in run_dir.rglob("*sandbox"):
            if sandbox_dir.is_dir():
                shutil.rmtree(sandbox_dir, ignore_errors=True)
    return {
        "scenario": name,
        "status": status,
        "duration_s": duration,
        "failures": c.failures,
        "skip_reason": skip_reason,
        "run_dir": str(run_dir),
    }


def write_reports(out: Path, results: list[dict[str, Any]], stamp: str) -> None:
    out.mkdir(parents=True, exist_ok=True)
    summary = {
        "stamp": stamp,
        "passed": sum(1 for r in results if r["status"] == "passed"),
        "failed": sum(1 for r in results if r["status"] == "failed"),
        "skipped": sum(1 for r in results if r["status"] == "skipped"),
        "total": len(results),
        "results": results,
    }
    (out / "report.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    lines = [
        f"# Detached-session evals ({stamp})",
        "",
        f"{summary['passed']} passed, {summary['failed']} failed, {summary['skipped']} skipped of {summary['total']}.",
        "",
        "| scenario | result | seconds | run dir |",
        "|---|---|---:|---|",
    ]
    for r in results:
        lines.append(f"| {r['scenario']} | {r['status']} | {r['duration_s']} | `{r['run_dir']}` |")
    for r in results:
        if r["failures"] or r["skip_reason"]:
            lines += ["", f"## {r['scenario']}: {r['status']}"]
            if r["skip_reason"]:
                lines.append(f"Skipped: {r['skip_reason']}")
            for failure in r["failures"]:
                lines.append(f"- {failure}")
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", default=str(DEFAULT_BIN), help="the hoocode binary to exercise")
    parser.add_argument("--only", action="append", default=[], help="run only these scenarios (repeatable)")
    parser.add_argument("--keep", action="store_true", help="keep the sandboxes of passing scenarios too")
    parser.add_argument("--out", default=str(OUT_DIR), help="report directory")
    args = parser.parse_args()

    binary = Path(args.bin)
    if not binary.is_file():
        print(f"no binary at {binary}: run `cargo build -p hoocode-code-main` first", file=sys.stderr)
        return 1
    known = {name for name, _, _ in SCENARIOS}
    unknown = [name for name in args.only if name not in known]
    if unknown:
        print(f"unknown scenario(s): {', '.join(unknown)}; known: {', '.join(sorted(known))}", file=sys.stderr)
        return 1

    out = Path(args.out).resolve()
    stamp = time.strftime("%Y%m%d-%H%M%S")
    runs = out / "runs" / stamp
    runs.mkdir(parents=True, exist_ok=True)
    # Temp files (the tmux config) stay in the report folder, not in /tmp.
    tmp = out / "tmp"
    tmp.mkdir(parents=True, exist_ok=True)
    tempfile.tempdir = str(tmp)

    results = []
    for name, fn, _doc in SCENARIOS:
        if args.only and name not in args.only:
            continue
        print(f"== {name}", flush=True)
        result = run_scenario(name, fn, runs / name, binary, args.keep)
        results.append(result)
        detail = result["skip_reason"] or "; ".join(result["failures"])[:300]
        print(f"   {result['status']} in {result['duration_s']}s" + (f": {detail}" if detail else ""), flush=True)

    write_reports(out, results, stamp)
    passed = sum(1 for r in results if r["status"] == "passed")
    print(f"\n{passed}/{len(results)} passed. Report: {out / 'report.md'}")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
