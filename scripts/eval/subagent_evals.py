#!/usr/bin/env python3
"""Subagent evals: measure how often a dispatch actually succeeds.

Why this exists: subagents were failing on nearly every dispatch for a week and
the only evidence was whatever dispatch dirs survived on disk — and a clean
success deletes its own dir, so successes were invisible and the failures were
swept after 24h (`docs/design/subagents.md` §1, §7). The dispatch ledger fixed
the *recording*; this measures the thing being recorded.

Every scenario runs the **real binary** end to end: a parent session that calls
the `Task` tool, the pool, a real child process, and a mock provider that
scripts what the model says. Nothing is stubbed inside the product. Faults are
injected from the provider side — a region error, a stream that dies mid-sentence,
a tool call with the wrong argument types, silence until the deadline — because
that is where failures actually come from.

    scripts/eval/subagent_evals.py                     # the whole suite
    scripts/eval/subagent_evals.py --only child_deadline_wrapup
    scripts/eval/subagent_evals.py --repeat 3          # flakiness, not just fate
    scripts/eval/subagent_evals.py --min-success 0.9   # gate a build

Exit code is 0 when every scenario met its declared expectation and the
measured usable rate is at or above `--min-success`; 1 otherwise. A scenario
marked `known_issue` documents today's behaviour and is reported separately: it
fails the suite only if reality changes, so a fix has to be made deliberately.

Reports land in `target/subagent-evals/`: `report.json`, `report.md` and one
directory per run with the request log and both transcripts.
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
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from mock_provider import CHILD_MARKER, MockProvider, Route  # noqa: E402

REPO = HERE.parent.parent
DEFAULT_BIN = REPO / "target/debug/cortex"
OUT_DIR = REPO / "target/subagent-evals"

MODEL = "mock-model"
PINNED_MODEL = "mock-pinned-model"
CONFIG_DIR = ".cortexcode"

PARENT_SUMMARY = "The repo has crates/, scripts/ and docs/."
CHILD_SUMMARY = "crates/ holds one directory per Rust crate; scripts/ holds tooling; docs/ holds design notes."


def task_call(**arguments: Any) -> dict[str, Any]:
    """A parent turn that delegates to a subagent."""
    base = {
        "subagent_type": "explore",
        "description": "map the repository layout",
        "prompt": "List the top-level directories and what is in them.",
    }
    base.update(arguments)
    return {"text": "", "tool_calls": [{"name": "Task", "arguments": base}]}


def child_route(turns: list[dict[str, Any]]) -> Route:
    return Route("child", turns, marker=CHILD_MARKER)


def parent_route(turns: list[dict[str, Any]]) -> Route:
    return Route("parent", turns)


@dataclass
class Expect:
    """What a scenario declares should happen. Absent means "do not check"."""

    ledger_statuses: list[str] | None = None
    ledger_ok: list[bool] | None = None
    ledger_count: int | None = None
    ledger_agent: str | None = None
    ledger_mode: str | None = None
    result_status: str | None = None
    result_confidence_min: float | None = None
    summary_contains: str | None = None
    error_contains: str | None = None
    parent_output_contains: str | None = None
    child_requests: int | None = None
    parent_requests: int | None = None
    attempts_usable_min: float | None = None


@dataclass
class Scenario:
    name: str
    kind: str  # "child" | "parent"
    doc: str
    routes: dict[str, Route]
    expect: Expect = field(default_factory=Expect)
    argv: list[str] = field(default_factory=list)
    prompt: str | None = None
    settings: dict[str, Any] = field(default_factory=dict)
    models: list[dict[str, Any]] | None = None
    timeout_s: int = 120
    slow: bool = False
    known_issue: str | None = None


# --------------------------------------------------------------------------
# Scenarios
# --------------------------------------------------------------------------

SCENARIOS: list[Scenario] = [
    Scenario(
        name="child_direct_answer",
        kind="child",
        doc="The floor: a child that answers in one turn writes a verifiable result.",
        routes={"child": child_route([{"text": CHILD_SUMMARY}])},
        argv=["--max-turns", "6"],
        expect=Expect(
            result_status="complete",
            result_confidence_min=0.5,
            summary_contains="crates/",
            child_requests=1,
        ),
    ),
    Scenario(
        name="child_tool_call_then_answer",
        kind="child",
        doc="A tool call inside the child: the loop must resume and still settle complete.",
        routes={
            "child": child_route(
                [
                    {"tool_calls": [{"name": "read", "arguments": {"path": "marker.txt"}}]},
                    {"text": f"marker.txt says hello. {CHILD_SUMMARY}"},
                ]
            )
        },
        argv=["--max-turns", "6"],
        expect=Expect(result_status="complete", child_requests=2),
    ),
    Scenario(
        name="child_turn_limit_settles_partial",
        kind="child",
        doc="A child that never stops asking for tools hits the turn limit and must still report a usable partial result.",
        routes={
            "child": child_route(
                [
                    {"tool_calls": [{"name": "read", "arguments": {"path": "marker.txt"}}]}
                    for _ in range(5)
                ]
            )
        },
        argv=["--max-turns", "2"],
        expect=Expect(result_status="partial", result_confidence_min=0.5),
    ),
    Scenario(
        name="child_invalid_tool_arguments",
        kind="child",
        doc="A tool call with the wrong argument types must not take the run down.",
        routes={
            "child": child_route(
                [
                    {"tool_calls": [{"name": "read", "arguments": {"path": 12345}}]},
                    {"text": f"The read failed as expected. {CHILD_SUMMARY}"},
                ]
            )
        },
        argv=["--max-turns", "6"],
        expect=Expect(result_status="complete"),
    ),
    Scenario(
        name="child_stream_abort_mid_answer",
        kind="child",
        doc=(
            "The provider dies mid-sentence. Reality: the runtime finalises what arrived, so "
            "the run settles complete on a partial answer. Recorded here as the baseline for "
            "'what does a truncated stream cost us', not as an endorsement."
        ),
        routes={
            "child": child_route(
                [
                    {"text": CHILD_SUMMARY * 8, "abort_after": 200},
                ]
            )
        },
        argv=["--max-turns", "6"],
        expect=Expect(result_status="complete"),
    ),
    Scenario(
        name="parent_stall_reaped",
        kind="parent",
        doc=(
            "A child that never answers. Before 2026-10-05 this was the worst case in the "
            "system: the child kept writing its heartbeat while blocked inside a provider call, "
            "so the stall watchdog could not see it and only the ten-minute hard deadline ended "
            "the run — measured alive at 95s with an empty ledger. Liveness is two-tier now "
            "(silence, or no forward progress) and a reap is SIGTERM-then-grace-then-SIGKILL, so "
            "the child gets to write its partial result on the way out."
        ),
        routes={
            "child": child_route([{"hang": True}]),
            "parent": parent_route(
                [task_call(background=False), {"text": PARENT_SUMMARY}]
            ),
        },
        prompt="Describe the repository layout using a subagent.",
        # Reaped at ~150s (the no-progress threshold) plus the 30s SIGTERM grace.
        timeout_s=260,
        slow=True,
        # Reaped, and the child wrote its partial result on the way out: the
        # ledger says `partial` because the child's own result.json says so,
        # even though the pool's verdict on the file is `complete`.
        expect=Expect(ledger_statuses=["partial"], ledger_ok=[True]),
    ),
    Scenario(
        name="child_deadline_wrapup_partial",
        kind="child",
        doc=(
            "The recovery added on 2026-10-03: a child told its deadline up front wraps up "
            "before the kill and reports a usable partial result instead of vanishing. The "
            "model has to keep working for this to mean anything, so the script loops on tool "
            "calls; the steer lands before the final turn."
        ),
        routes={
            "child": child_route(
                [
                    {"tool_calls": [{"name": "read", "arguments": {"path": "marker.txt"}}], "delay_s": 8}
                    for _ in range(6)
                ]
                + [{"text": f"Wrapping up. {CHILD_SUMMARY}"}]
            )
        },
        # The steer is sent 90s before the deadline, so the deadline has to be
        # comfortably above that for the wrap-up path to exist at all.
        argv=["--deadline-ms", "120000", "--max-turns", "50"],
        timeout_s=180,
        slow=True,
        expect=Expect(
            result_status="partial",
            result_confidence_min=0.5,
            summary_contains="crates/",
        ),
    ),
    Scenario(
        name="parent_dispatch_blocking",
        kind="parent",
        doc="The ordinary case: the parent delegates and waits for the answer inline.",
        routes={
            "child": child_route([{"text": CHILD_SUMMARY}]),
            "parent": parent_route(
                [task_call(background=False), {"text": PARENT_SUMMARY}]
            ),
        },
        prompt="Describe the repository layout using a subagent.",
        expect=Expect(
            ledger_statuses=["complete"],
            ledger_ok=[True],
            ledger_agent="explore",
            ledger_mode="blocking",
            parent_output_contains="crates/",
        ),
    ),
    Scenario(
        name="parent_dispatch_background",
        kind="parent",
        doc=(
            "A background dispatch: the parent gets a notification, keeps working, and is "
            "woken when the result lands."
        ),
        routes={
            "child": child_route([{"text": CHILD_SUMMARY}]),
            "parent": parent_route(
                [
                    task_call(background=True),
                    {"text": "Subagent dispatched; I will report when it finishes."},
                    {"text": PARENT_SUMMARY},
                ]
            ),
        },
        prompt="Describe the repository layout using a background subagent.",
        expect=Expect(
            ledger_statuses=["complete"],
            ledger_ok=[True],
            ledger_mode="background",
            parent_output_contains="crates/",
        ),
    ),
    Scenario(
        name="parent_region_error_falls_back",
        kind="parent",
        doc=(
            "The preferred model is refused by region. The pool must retry on the "
            "dispatching session's own model and the run must still succeed."
        ),
        routes={
            "child": child_route(
                [
                    {
                        "error": {
                            "status": 400,
                            "message": "400 Upstream request failed: This Go model requires Global regions",
                        }
                    },
                    {"text": CHILD_SUMMARY},
                ]
            ),
            "parent": parent_route(
                [task_call(background=False, complexity="capable"), {"text": PARENT_SUMMARY}]
            ),
        },
        prompt="Describe the repository layout using a subagent.",
        settings={"modelCategories": {"capable": PINNED_MODEL, "fast": PINNED_MODEL, "standard": MODEL}},
        expect=Expect(
            ledger_statuses=["failed", "complete"],
            ledger_ok=[False, True],
            ledger_count=2,
        ),
    ),
    Scenario(
        name="parent_unknown_complexity_tier",
        kind="parent",
        doc=(
            "A typo in the complexity tier must never reach a child. It does not: the tool "
            "schema rejects it in the parent, so no dispatch and no ledger line. The pool "
            "still does not validate a tier on the paths that skip the schema."
        ),
        routes={
            "child": child_route([{"text": CHILD_SUMMARY}]),
            "parent": parent_route([task_call(background=False, complexity="fastt"), {"text": "done"}]),
        },
        prompt="Describe the repository layout using a subagent.",
        expect=Expect(ledger_count=0, parent_output_contains="complexity"),
    ),
    Scenario(
        name="child_rate_limited_then_answers",
        kind="child",
        doc=(
            "The provider answers 429 with a retry-after. The runtime retries the call; the run "
            "must still settle on the retry, with the retry visible in the transcript."
        ),
        routes={
            "child": child_route(
                [
                    {"error": {"status": 429, "message": "429 rate limited", "retry_after": 1}},
                    {"text": CHILD_SUMMARY},
                ]
            )
        },
        argv=["--max-turns", "6"],
        expect=Expect(result_status="complete", child_requests=2),
    ),
    Scenario(
        name="child_corrupt_jsonl_line",
        kind="child",
        doc=(
            "The stream dies mid-line. A JSONL reader must skip the torn line rather than treat it "
            "as a turn boundary and lose the run."
        ),
        routes={
            "child": child_route([{"text": CHILD_SUMMARY * 6, "corrupt_after": 160}])
        },
        argv=["--max-turns", "6"],
        expect=Expect(result_status="complete"),
    ),
    Scenario(
        name="child_context_overflow",
        kind="child",
        doc=(
            "The context no longer fits. Nothing can recover this, and the point is that it fails "
            "cleanly with a cause the parent can read, rather than hanging or claiming success."
        ),
        routes={
            "child": child_route(
                [
                    {
                        "error": {
                            "status": 400,
                            "type": "invalid_request_error",
                            "code": "context_length_exceeded",
                            "message": "This model's maximum context length is 200000 tokens",
                        }
                    }
                ]
            )
        },
        argv=["--max-turns", "6"],
        expect=Expect(result_status="failed"),
    ),
    Scenario(
        name="parent_queue_saturation",
        kind="parent",
        doc=(
            "More dispatches than pool slots: every one must settle and every one must be "
            "recorded. Nothing may be left stuck in running."
        ),
        routes={
            "child": child_route([{"text": CHILD_SUMMARY} for _ in range(8)]),
            "parent": parent_route(
                [
                    {
                        "text": "",
                        "tool_calls": [
                            {"name": "Task", "arguments": {
                                "subagent_type": "explore",
                                "description": f"map area {i}",
                                "prompt": f"List the top-level directories ({i}).",
                                "background": True,
                            }}
                            for i in range(8)
                        ],
                    },
                    {"text": "All dispatched."},
                    {"text": "All eight subagents reported the same layout: crates/, scripts/, docs/."},
                ]
            ),
        },
        prompt="Dispatch eight subagents in parallel and report when they are done.",
        timeout_s=180,
        expect=Expect(ledger_count=8, attempts_usable_min=1.0),
    ),
]


# --------------------------------------------------------------------------
# Harness
# --------------------------------------------------------------------------


@dataclass
class Outcome:
    scenario: str
    passed: bool
    known_issue_hit: bool
    duration_s: float
    failures: list[str]
    ledger: list[dict[str, Any]]
    result: dict[str, Any] | None
    child_requests: int
    parent_requests: int
    exit_code: int | None
    run_dir: str


def models_json(port: int, models: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "providers": {
            "mock": {
                "baseUrl": f"http://127.0.0.1:{port}/v1",
                "api": "openai-completions",
                "apiKey": "mock-key",
                "models": models,
            }
        }
    }


def default_models() -> list[dict[str, Any]]:
    return [
        {"id": MODEL, "name": "Mock", "contextWindow": 128000, "maxTokens": 4096},
        {
            "id": PINNED_MODEL,
            "name": "Mock pinned",
            "contextWindow": 128000,
            "maxTokens": 4096,
        },
    ]


def write_project(root: Path, port: int, scenario: Scenario) -> tuple[Path, Path]:
    """An isolated HOME and project. Returns (home, work)."""
    home = root / "home"
    work = root / "work"
    (home / CONFIG_DIR).mkdir(parents=True, exist_ok=True)
    work.mkdir(parents=True, exist_ok=True)
    (work / "marker.txt").write_text("hello\n", encoding="utf-8")
    (home / CONFIG_DIR / "models.json").write_text(
        json.dumps(models_json(port, scenario.models or default_models())), encoding="utf-8"
    )
    settings = {
        "defaultProvider": "mock",
        "defaultModel": MODEL,
        "enableSubagent": True,
        "maxSubagentDepth": 1,
    }
    settings.update(scenario.settings)
    (home / CONFIG_DIR / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
    return home, work


def build_argv(binary: Path, scenario: Scenario) -> list[str]:
    argv = [
        str(binary),
        "--offline",
        "--mode",
        "json",
        "--provider",
        "mock",
        "--model",
        MODEL,
    ]
    if scenario.kind == "child":
        argv += ["--task-id", "eval-child", "--session", "session.jsonl"]
    else:
        argv += ["--enable-subagents"]
    argv += scenario.argv
    if scenario.prompt:
        argv.append(scenario.prompt)
    elif scenario.kind == "child":
        argv.append("Task: List the top-level directories and what is in them.")
    return argv


# Faults the matrix injects into whichever route the scenario is not asserting on.
# `--matrix` runs every scenario once per entry; only `none` has a declared
# expectation, the rest are observational ("does this break the run cleanly?").
MATRIX_FAULTS: list[tuple[str, dict[str, Any]]] = [
    ("none", {}),
    ("rate-limited", {"error": {"status": 429, "message": "429 slow down", "retry_after": 1}}),
    ("mid-stream-abort", {"abort_after": 120}),
    ("corrupt-line", {"corrupt_after": 120}),
    ("provider-error", {"error": {"status": 500, "message": "500 upstream exploded"}}),
]


def with_fault(scenario: Scenario, fault: dict[str, Any]) -> Scenario:
    """A copy of `scenario` with `fault` prepended to the script under test.

    The fault goes to the *child* when there is one: what matters is what a
    provider does to a subagent, not what it does to the session that dispatched
    it (a parent that never dispatches exercises nothing).
    """
    target = "child" if "child" in scenario.routes else next(iter(scenario.routes))
    routes = {}
    for name, route in scenario.routes.items():
        turns = [dict(turn) for turn in route.turns]
        if name == target:
            turns.insert(0, dict(fault))
        routes[name] = Route(name, turns, marker=route.marker)
    return Scenario(
        name=scenario.name,
        kind=scenario.kind,
        doc=scenario.doc,
        routes=routes,
        expect=scenario.expect,
        argv=list(scenario.argv),
        prompt=scenario.prompt,
        settings=dict(scenario.settings),
        models=scenario.models,
        timeout_s=scenario.timeout_s,
        slow=scenario.slow,
        known_issue=scenario.known_issue,
    )


def run_scenario(binary: Path, scenario: Scenario, run_dir: Path) -> Outcome:
    started = time.time()
    root = Path(tempfile.mkdtemp(prefix=f"subagent-eval-{scenario.name}-", dir=run_dir))
    log_path = root / "requests.jsonl"
    provider = MockProvider(scenario.routes, log_path=str(log_path))
    port = provider.start()
    failures: list[str] = []
    exit_code: int | None = None
    try:
        home, work = write_project(root, port, scenario)
        argv = build_argv(binary, scenario)
        env = dict(os.environ)
        env.update(
            {
                "HOME": str(home),
                "TERM": "dumb",
                "NO_COLOR": "1",
                "LANG": "en_US.UTF-8",
                # The pool reads depth from the environment; a stale value from
                # the developer's shell would silently change what is tested.
                "CORTEXCODE_SUBAGENT_DEPTH": "0",
            }
        )
        for leaked in ("CORTEXCODE_CODING_AGENT_DIR", "CORTEX_", "HOOCODE_", "CORTEXCODE_"):
            for key in [k for k in env if k.startswith(leaked)]:
                env.pop(key)
        try:
            completed = subprocess.run(
                argv,
                env=env,
                cwd=work,
                capture_output=True,
                text=True,
                timeout=scenario.timeout_s,
            )
            exit_code = completed.returncode
            stdout, stderr = completed.stdout, completed.stderr
        except subprocess.TimeoutExpired as expired:
            stdout = (expired.stdout or b"").decode("utf-8", "replace") if isinstance(expired.stdout, bytes) else (expired.stdout or "")
            stderr = (expired.stderr or b"").decode("utf-8", "replace") if isinstance(expired.stderr, bytes) else (expired.stderr or "")
            failures.append(f"the run did not finish within {scenario.timeout_s}s")
        (root / "stdout.txt").write_text(stdout, encoding="utf-8")
        (root / "stderr.txt").write_text(stderr, encoding="utf-8")

        ledger = read_ledger(work)
        result = read_result(work)
        child_requests = len(provider.requests("child"))
        parent_requests = len(provider.requests("parent"))
        failures += check(scenario.expect, ledger, result, stdout, child_requests, parent_requests)
    finally:
        provider.stop()

    duration = time.time() - started
    known_issue_hit = bool(scenario.known_issue) and failures != []
    return Outcome(
        scenario=scenario.name,
        passed=not failures,
        known_issue_hit=known_issue_hit,
        duration_s=round(duration, 1),
        failures=failures,
        ledger=ledger,
        result=result,
        child_requests=child_requests,
        parent_requests=parent_requests,
        exit_code=exit_code,
        run_dir=str(root),
    )


def read_ledger(work: Path) -> list[dict[str, Any]]:
    path = work / CONFIG_DIR / "dispatch/ledger.jsonl"
    if not path.exists():
        return []
    entries = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            entries.append(json.loads(line))
        except json.JSONDecodeError:
            entries.append({"status": "<unparseable>", "error": line[:200]})
    return entries


def read_result(work: Path) -> dict[str, Any] | None:
    base = work / CONFIG_DIR / "dispatch"
    if not base.exists():
        return None
    for child in sorted(base.iterdir()):
        candidate = child / "result.json"
        if candidate.exists():
            try:
                return json.loads(candidate.read_text(encoding="utf-8"))
            except json.JSONDecodeError:
                return {"status": "<unparseable>"}
    return None


def check(
    expect: Expect,
    ledger: list[dict[str, Any]],
    result: dict[str, Any] | None,
    stdout: str,
    child_requests: int,
    parent_requests: int,
) -> list[str]:
    failures: list[str] = []

    def want(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)

    if expect.ledger_count is not None:
        want(
            len(ledger) == expect.ledger_count,
            f"expected {expect.ledger_count} ledger line(s), got {len(ledger)}: "
            f"{[entry.get('status') for entry in ledger]}",
        )
    if expect.ledger_statuses is not None:
        got = [entry.get("status") for entry in ledger]
        want(
            got == expect.ledger_statuses,
            f"ledger statuses {got} != expected {expect.ledger_statuses}",
        )
    if expect.ledger_ok is not None:
        got = [bool(entry.get("ok")) for entry in ledger]
        want(got == expect.ledger_ok, f"ledger ok {got} != expected {expect.ledger_ok}")
    if expect.ledger_agent is not None:
        agents = {entry.get("agent_type") for entry in ledger}
        want(agents == {expect.ledger_agent}, f"ledger agents {agents} != {expect.ledger_agent}")
    if expect.ledger_mode is not None:
        modes = {entry.get("mode") for entry in ledger}
        want(modes == {expect.ledger_mode}, f"ledger modes {modes} != {expect.ledger_mode}")
    if expect.attempts_usable_min is not None:
        usable = sum(1 for entry in ledger if entry.get("ok"))
        rate = usable / len(ledger) if ledger else 0.0
        want(
            rate >= expect.attempts_usable_min,
            f"usable rate {rate:.2f} below {expect.attempts_usable_min} ({usable}/{len(ledger)})",
        )
    if expect.result_status is not None:
        want(result is not None, "no result.json was written")
        if result is not None:
            want(
                result.get("status") == expect.result_status,
                f"result.json status {result.get('status')} != {expect.result_status}",
            )
    if expect.result_confidence_min is not None and result is not None:
        want(
            float(result.get("confidence") or 0) >= expect.result_confidence_min,
            f"confidence {result.get('confidence')} below {expect.result_confidence_min}",
        )
    if expect.summary_contains is not None and result is not None:
        want(
            expect.summary_contains in str(result.get("summary", "")),
            f"summary {str(result.get('summary'))[:120]!r} lacks {expect.summary_contains!r}",
        )
    if expect.error_contains is not None:
        blob = json.dumps(ledger) + json.dumps(result or {})
        want(expect.error_contains in blob, f"no record mentions {expect.error_contains!r}")
    if expect.parent_output_contains is not None:
        want(
            expect.parent_output_contains in stdout,
            f"the parent's output never contained {expect.parent_output_contains!r}",
        )
    if expect.child_requests is not None:
        want(
            child_requests == expect.child_requests,
            f"child made {child_requests} request(s), expected {expect.child_requests}",
        )
    if expect.parent_requests is not None:
        want(
            parent_requests == expect.parent_requests,
            f"parent made {parent_requests} request(s), expected {expect.parent_requests}",
        )
    return failures


def count_leaked_children(run_dir: Path) -> int:
    """Child processes from these runs still alive after they settled."""
    try:
        listing = subprocess.run(
            ["pgrep", "-f", "task-id dispatch-"], capture_output=True, text=True, timeout=5
        )
    except (OSError, subprocess.SubprocessError):
        return 0
    return len([line for line in listing.stdout.splitlines() if line.strip()])


def summarize(outcomes: list[Outcome]) -> dict[str, Any]:
    attempts = [entry for outcome in outcomes for entry in outcome.ledger]
    usable = sum(1 for entry in attempts if entry.get("ok"))
    statuses: dict[str, int] = {}
    for entry in attempts:
        status = str(entry.get("status"))
        statuses[status] = statuses.get(status, 0) + 1
    return {
        "scenarios": len(outcomes),
        "passed": sum(1 for o in outcomes if o.passed),
        "known_issues_hit": sum(1 for o in outcomes if o.known_issue_hit),
        "failed": sum(1 for o in outcomes if not o.passed and not o.known_issue_hit),
        "attempts": len(attempts),
        "usable_attempts": usable,
        "usable_rate": round(usable / len(attempts), 3) if attempts else 0.0,
        "statuses": statuses,
        "total_duration_s": round(sum(o.duration_s for o in outcomes), 1),
        "leaked_children": sum(
            1 for o in outcomes for f in o.failures if f.startswith("leaked child")
        ),
        "unsettled": sum(
            1 for o in outcomes for f in o.failures if f.startswith("the run produced")
        ),
    }


def markdown_report(summary: dict[str, Any], outcomes: list[Outcome], meta: dict[str, Any]) -> str:
    lines = [
        "# Subagent evals",
        "",
        f"- generated: {meta['generated']}",
        f"- binary: `{meta['binary']}`",
        f"- repeat: {meta['repeat']}",
        f"- scenarios: {summary['passed']}/{summary['scenarios']} as declared, "
        f"{summary['known_issues_hit']} known-issue hit(s), {summary['failed']} failure(s)",
        f"- attempts recorded: {summary['attempts']} · usable: {summary['usable_attempts']} "
        f"({summary['usable_rate'] * 100:.0f}%)",
        f"- statuses: {summary['statuses']}",
        f"- wall clock: {summary['total_duration_s']}s",
        "",
        "| scenario | result | s | child req | ledger | note |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for outcome in outcomes:
        verdict = "pass" if outcome.passed else ("KNOWN" if outcome.known_issue_hit else "FAIL")
        statuses = ",".join(str(entry.get("status")) for entry in outcome.ledger) or "-"
        note = "; ".join(outcome.failures)[:160] if outcome.failures else ""
        lines.append(
            f"| {outcome.scenario} | {verdict} | {outcome.duration_s} | {outcome.child_requests} "
            f"| {statuses} | {note} |"
        )
    lines.append("")
    if meta.get("matrix"):
        lines += [
            "",
            "## Fault matrix",
            "",
            "| scenario | " + " | ".join(meta["matrix"]["faults"]) + " |",
            "| --- | " + " | ".join("---" for _ in meta["matrix"]["faults"]) + " |",
        ]
        for name, row in meta["matrix"]["rows"].items():
            lines.append(
                f"| {name} | " + " | ".join(row.get(f, "-") for f in meta["matrix"]["faults"]) + " |"
            )
        lines.append("")
    for outcome in outcomes:
        if outcome.failures:
            lines.append(f"## {outcome.scenario}")
            lines.append("")
            for failure in outcome.failures:
                lines.append(f"- {failure}")
            lines.append(f"- run dir: `{outcome.run_dir}`")
            lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", default=str(DEFAULT_BIN), help="the cortex binary to exercise")
    parser.add_argument("--only", action="append", default=[], help="run only these scenarios")
    parser.add_argument("--repeat", type=int, default=1, help="run each scenario N times")
    parser.add_argument("--include-slow", action="store_true", help="include scenarios marked slow")
    parser.add_argument(
        "--matrix",
        action="store_true",
        help="run every scenario once per fault (none, 429, mid-stream abort, corrupt line) "
        "and report the combination table; the declared expectation still applies to the "
        "no-fault column",
    )
    parser.add_argument(
        "--soak",
        type=int,
        default=0,
        metavar="SECONDS",
        help="run the suite in a loop for this long with randomised faults, asserting that "
        "nothing leaks and nothing stays stuck",
    )
    parser.add_argument("--min-success", type=float, default=0.0, help="fail below this usable rate")
    parser.add_argument("--keep", action="store_true", help="keep the per-run directories")
    parser.add_argument("--out", default=str(OUT_DIR), help="report directory")
    args = parser.parse_args()

    binary = Path(args.bin).resolve()
    if not binary.exists():
        print(f"error: no binary at {binary}. Build it first:\n  cargo build -p cortexcode-code-main", file=sys.stderr)
        return 2

    selected = [s for s in SCENARIOS if not args.only or s.name in args.only]
    if args.only and len(selected) != len(args.only):
        missing = set(args.only) - {s.name for s in selected}
        print(f"error: unknown scenario(s): {', '.join(sorted(missing))}", file=sys.stderr)
        return 2
    if not args.include_slow:
        skipped = [s.name for s in selected if s.slow]
        selected = [s for s in selected if not s.slow]
        if skipped:
            print(f"note: skipping slow scenario(s) {', '.join(skipped)} (--include-slow)")

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="run-", dir=out_dir))

    def run(scenario: Scenario, label: str) -> Outcome:
        outcome = run_scenario(binary, scenario, run_dir)
        outcome.scenario = label
        return outcome

    def report_one(outcome: Outcome) -> None:
        verdict = "pass" if outcome.passed else ("KNOWN" if outcome.known_issue_hit else "FAIL")
        statuses = ",".join(str(entry.get("status")) for entry in outcome.ledger) or "-"
        print(
            f"{verdict:>5}  {outcome.scenario:<38} {outcome.duration_s:>6}s  "
            f"child={outcome.child_requests}  {statuses}"
        )
        for failure in outcome.failures:
            print(f"         - {failure}")

    outcomes: list[Outcome] = []
    matrix: dict[str, dict[str, str]] = {}
    if args.soak:
        # Long-running and unstructured on purpose: the assertions are about the
        # process, not about any one scenario's expectation.
        deadline = time.time() + args.soak
        rounds = 0
        while time.time() < deadline:
            rounds += 1
            scenario = selected[rounds % len(selected)]
            fault = MATRIX_FAULTS[rounds % len(MATRIX_FAULTS)][1]
            outcome = run(with_fault(scenario, fault), f"soak-{rounds} {scenario.name}")
            # A soak asserts about the *process*, not about one scenario's
            # request counts: every run must settle, nothing may leak, and the
            # ledger must have a line for it. Scenario-specific expectations do
            # not apply once the script has been perturbed.
            leaked = count_leaked_children(run_dir)
            findings = []
            if leaked:
                findings.append(f"leaked child processes: {leaked}")
            if not outcome.ledger and outcome.result is None and outcome.exit_code is None:
                findings.append("the run produced neither a ledger line nor a result")
            outcome.failures = findings
            outcome.passed = not findings
            outcomes.append(outcome)
            if findings:
                report_one(outcome)
        print(f"soak: {rounds} rounds over {args.soak}s, {len(outcomes)} attempts recorded")
    elif args.matrix:
        for scenario in selected:
            row: dict[str, str] = {}
            for label, fault in MATRIX_FAULTS:
                variant = with_fault(scenario, fault) if fault else scenario
                outcome = run(variant, f"{scenario.name} [{label}]")
                # Only the no-fault column carries a declared expectation; the
                # rest are recorded, not judged, except that a run which neither
                # settles nor leaves a usable result is a finding.
                if label == "none":
                    outcome.passed = outcome.passed and not outcome.known_issue_hit
                else:
                    # Either the run settled with something to show, or it
                    # behaved exactly as the scenario already declares (a
                    # dispatch that never happens is not a broken fault cell).
                    settled = bool(outcome.ledger) or outcome.result is not None
                    outcome.passed = (settled or not outcome.failures) and not outcome.known_issue_hit
                row[label] = (
                    "ok"
                    if outcome.passed
                    else ("known" if outcome.known_issue_hit else "BROKEN")
                ) + f" {outcome.duration_s:.0f}s"
                outcomes.append(outcome)
            matrix[scenario.name] = row
            print(f"{scenario.name:<34} " + "  ".join(f"{k}={v}" for k, v in row.items()))
    else:
        for scenario in selected:
            for iteration in range(args.repeat):
                label = (
                    f"{scenario.name}#{iteration + 1}" if args.repeat > 1 else scenario.name
                )
                outcome = run(scenario, label)
                outcomes.append(outcome)
                report_one(outcome)

    summary = summarize(outcomes)
    meta: dict[str, Any] = {
        "generated": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "binary": str(binary),
        "repeat": args.repeat,
    }
    if matrix:
        meta["matrix"] = {"faults": [label for label, _ in MATRIX_FAULTS], "rows": matrix}
    (out_dir / "report.json").write_text(
        json.dumps(
            {
                "summary": summary,
                "meta": meta,
                "outcomes": [
                    {
                        "scenario": o.scenario,
                        "passed": o.passed,
                        "known_issue_hit": o.known_issue_hit,
                        "duration_s": o.duration_s,
                        "failures": o.failures,
                        "ledger": o.ledger,
                        "result": o.result,
                        "child_requests": o.child_requests,
                        "parent_requests": o.parent_requests,
                        "exit_code": o.exit_code,
                        "run_dir": o.run_dir,
                    }
                    for o in outcomes
                ],
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    (out_dir / "report.md").write_text(markdown_report(summary, outcomes, meta), encoding="utf-8")
    if not args.keep:
        shutil.rmtree(run_dir, ignore_errors=True)
    else:
        print(f"run dirs kept under {run_dir}")

    print()
    print(
        f"{summary['passed']}/{summary['scenarios']} scenarios as declared · "
        f"{summary['attempts']} attempts · usable {summary['usable_attempts']} "
        f"({summary['usable_rate'] * 100:.0f}%) · {summary['total_duration_s']}s"
    )
    print(f"report: {out_dir / 'report.md'}")

    if summary["leaked_children"]:
        print(
            f"FAIL: {summary['leaked_children']} run(s) left a child process behind",
            file=sys.stderr,
        )
        return 1
    if summary["failed"]:
        print("FAIL: a scenario did not match its declared expectation", file=sys.stderr)
        return 1
    if summary["usable_rate"] < args.min_success:
        print(
            f"FAIL: usable rate {summary['usable_rate']:.2f} below --min-success {args.min_success}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())