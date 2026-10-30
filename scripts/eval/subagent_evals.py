#!/usr/bin/env python3
"""Subagent evals: measure how often a dispatch actually succeeds.

Why this exists: subagents were failing on nearly every dispatch for a week and
the only evidence was whatever dispatch dirs survived on disk — and a clean
success deletes its own dir, so successes were invisible and the failures were
swept after 24h (`docs/design/subagents.md` §1, §7). The dispatch ledger fixed
the *recording*; this measures the thing being recorded.

Every scenario runs the **real binary** end to end: a parent session that calls
the `Agent` tool, the pool, a real child process, and a mock provider that
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
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from mock_provider import CHILD_MARKER, MockProvider, Route, last_user_text  # noqa: E402

REPO = HERE.parent.parent
DEFAULT_BIN = REPO / "target/debug/hoocode"
OUT_DIR = REPO / "target/subagent-evals"

MODEL = "mock-model"
PINNED_MODEL = "mock-pinned-model"
CONFIG_DIR = ".hoocode"

PARENT_SUMMARY = "The repo has crates/, scripts/ and docs/."
CHILD_SUMMARY = "crates/ holds one directory per Rust crate; scripts/ holds tooling; docs/ holds design notes."
# The background notification shows the child's result cut to about 80 characters
# (`explore#1 finished ✓ — crates/ holds one directory per Rust crate; scripts/….`),
# so the request that delivers it is recognised by the part that survives the cut.
CHILD_RESULT_NEEDLE = CHILD_SUMMARY[:30]


# What a request body is searched for, so a scenario can say what a model was told.
NEEDLES = {
    "child_result": CHILD_RESULT_NEEDLE,
    # The subagent's gated Shell call was denied: its parent has no UI to ask.
    "clearance_denied": "has no UI to ask",
    # Two background runs of one agent type (`parent_tells_parallel_runs_apart`).
    # Request bodies are JSON, so the quotes around a description arrive escaped.
    # Each call's placeholder names the run's handle and description, in call order:
    "placeholder_names_run_1": 'explore#1 (\\"audit the auth module\\") started in the background',
    "placeholder_names_run_2": 'explore#2 (\\"audit the db layer\\") started in the background',
    # and the roster lists both runs with their descriptions while they run.
    "roster_names_run_1": '- explore#1 (\\"audit the auth module\\")  running',
    "roster_names_run_2": '- explore#2 (\\"audit the db layer\\")  running',
}

# A global config that lets subagents run Shell without asking. A subagent whose
# parent has no UI is denied what needs approval, so a scenario that has one run
# Shell says so here.
SHELL_AUTO_ALLOW = json.dumps({"modes": {"build": {"auto_allow": ["Shell"]}}})


def task_call(tool: str = "Agent", **arguments: Any) -> dict[str, Any]:
    """A parent turn that delegates to a subagent.

    `tool` is the canonical `Agent` unless a scenario is specifically about
    the deprecated `Task` alias.
    """
    base = {
        "subagent_type": "explore",
        "description": "map the repository layout",
        "prompt": "List the top-level directories and what is in them.",
    }
    base.update(arguments)
    return {"text": "", "tool_calls": [{"name": tool, "arguments": base}]}


def child_route(turns: list[dict[str, Any]]) -> Route:
    return Route("child", turns, marker=CHILD_MARKER)


def parent_route(turns: list[dict[str, Any]]) -> Route:
    return Route("parent", turns)


# A user agent that may dispatch one level deeper (`delegate: true`). Nested
# scenarios chain it: whether a process at depth d gets the Agent tool is what
# `maxSubagentDepth` decides, and this agent is the only way to get one.
RELAY_AGENT = (
    "---\n"
    "name: relay\n"
    "description: Passes a task on to one deeper subagent and reports what comes back.\n"
    "delegate: true\n"
    "---\n"
    "Pass the task on to a subagent with the Agent tool, wait for its result, then report it.\n"
)


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
    # Minimum number of parent tool calls issued after the Agent dispatch and
    # before the request that delivers the child's result.
    parent_tool_calls_before_result: int | None = None
    # Writer scenarios (see `Scenario.worktree`).
    branch_commits: int | None = None
    parent_status_unchanged: bool | None = None
    nothing_pushed: bool | None = None
    # Per-route checks (route name -> value). `route_requests` is the exact
    # request count; `route_tools_have` / `route_tools_lack` are the tool names
    # the provider offered in that route's requests.
    route_requests: dict[str, int] | None = None
    route_tools_have: dict[str, list[str]] | None = None
    route_tools_lack: dict[str, list[str]] | None = None
    # Routes that must have received a child's result in some request.
    results_flow: list[str] | None = None
    # Needle names (see `NEEDLES`) a route's requests must have carried: what
    # the model on that route was told.
    route_sees: dict[str, list[str]] | None = None
    # Most requests in flight at once on a route, measured at the provider: a
    # subagent holds its pool slot for its whole run, so this is a lower bound
    # on how many subagents ran together.
    peak_inflight_max: dict[str, int] | None = None
    peak_inflight_min: dict[str, int] | None = None


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
    # Extra environment for the parent process (the lifeguard runs there).
    env: dict[str, str] = field(default_factory=dict)
    # Set up a temp git repo the parent works in, with a bare origin. The routes
    # use the placeholders <REPO>, <WT> and <ORIGIN> (see `writer_paths`).
    worktree: bool = False
    # Files written under the run's ~/.hoocode (path relative to it), e.g. user
    # agent definitions in `agents/`.
    home_files: dict[str, str] = field(default_factory=dict)


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
            "(silence, or no forward progress). A stall reap is SIGUSR1 to the child, then SIGTERM "
            "and SIGKILL to its process group after the grace, so the child gets to write its "
            "partial result on the way out."
        ),
        routes={
            "child": child_route([{"hang": True}]),
            "parent": parent_route(
                [task_call(background=False), {"text": PARENT_SUMMARY}]
            ),
        },
        prompt="Describe the repository layout using a subagent.",
        # The no-progress threshold is 20 minutes in production. The parent gets
        # HOOCODE_SUBAGENT_STALL_MS=20000 so the reap lands in about 25s (the
        # 5s lifeguard tick), then the child has its 30s SIGUSR1 grace. The
        # production value is 20 minutes: this scenario proves the path, not the
        # threshold.
        env={"HOOCODE_SUBAGENT_STALL_MS": "20000"},
        timeout_s=120,
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
        name="parent_works_while_background_runs",
        kind="parent",
        doc=(
            "A background dispatch with the parent doing its own work meanwhile. The child is "
            "slowed by three seconds so the parent's Read lands while it runs. The check: at "
            "least one parent tool call sits between the dispatch and the child's result. A "
            "barrier-waiting parent (orchestration P3, P4) would have none."
        ),
        routes={
            "child": child_route([{"text": CHILD_SUMMARY, "delay_s": 3}]),
            "parent": parent_route(
                [
                    task_call(background=True),
                    {"tool_calls": [{"name": "Read", "arguments": {"path": "marker.txt"}}]},
                    {"text": "Subagent dispatched and marker.txt read; I will report when it finishes."},
                    {"text": PARENT_SUMMARY},
                ]
            ),
        },
        prompt="Describe the repository layout using a background subagent, and read marker.txt while it runs.",
        expect=Expect(
            ledger_statuses=["complete"],
            ledger_ok=[True],
            ledger_mode="background",
            parent_output_contains="crates/",
            parent_tool_calls_before_result=1,
            parent_requests=4,
        ),
    ),
    Scenario(
        name="parent_tells_parallel_runs_apart",
        kind="parent",
        doc=(
            "Two background runs of one agent type, dispatched in one turn, then the roster. "
            "The user's report (2026-10-30): the model took one of two subagents for a "
            "duplicate, cancelled the other one and dispatched it again. Before the fix every "
            "background call was answered with the same `Started \"Agent\" in the background` "
            "line and the roster listed bare labels (`explore#1  running`, `explore#2  running`), "
            "so nothing the model saw said which run was which task. The check: each call's "
            "placeholder names its own handle and description in call order, and the roster "
            "lists both runs with their descriptions."
        ),
        routes={
            "child": child_route(
                [{"text": CHILD_SUMMARY, "delay_s": 3}, {"text": CHILD_SUMMARY, "delay_s": 3}]
            ),
            "parent": parent_route(
                [
                    {
                        "text": "",
                        "tool_calls": [
                            task_call(background=True, description="audit the auth module")[
                                "tool_calls"
                            ][0],
                            task_call(background=True, description="audit the db layer")[
                                "tool_calls"
                            ][0],
                        ],
                    },
                    {"tool_calls": [{"name": "AgentOutput", "arguments": {"list": True}}]},
                    {"text": "Both subagents are running; I will report when they finish."},
                    {"text": PARENT_SUMMARY},
                    {"text": PARENT_SUMMARY},
                    {"text": PARENT_SUMMARY},
                ]
            ),
        },
        prompt="Audit the auth module and the db layer with two background subagents.",
        expect=Expect(
            ledger_statuses=["complete", "complete"],
            ledger_mode="background",
            route_sees={
                "parent": [
                    "placeholder_names_run_1",
                    "placeholder_names_run_2",
                    "roster_names_run_1",
                    "roster_names_run_2",
                ]
            },
            results_flow=["parent"],
        ),
    ),
    Scenario(
        name="worktree_writer_commits_on_branch",
        kind="parent",
        doc=(
            "The writing protocol (orchestration §3) on a temp repo. The parent creates the "
            "worktree and branch under the HOME it was given (the D5 layout), then dispatches a "
            "general-purpose child in blocking mode. The child commits on agent/eval-writer from "
            "inside the worktree. Checks: the branch holds one commit, the parent's tree status "
            "is unchanged, and the origin has no new refs. Nothing outside the temp dir is touched."
        ),
        routes={
            "child": child_route(
                [
                    {
                        "tool_calls": [
                            {
                                "name": "Shell",
                                "arguments": {
                                    "command": (
                                        "echo 'eval note' > <WT>/eval-note.txt && "
                                        "git -C <WT> add eval-note.txt && "
                                        "git -C <WT> -c user.name=eval -c user.email=eval@example.invalid "
                                        "commit -q -m 'eval: writer commit'"
                                    )
                                },
                            }
                        ]
                    },
                    {"text": CHILD_SUMMARY},
                ]
            ),
            "parent": parent_route(
                [
                    {
                        "text": "",
                        "tool_calls": [
                            {
                                "name": "Shell",
                                "arguments": {"command": "git -C <REPO> worktree add -q -b agent/eval-writer <WT> HEAD"},
                            }
                        ],
                    },
                    task_call(
                        background=False,
                        subagent_type="general-purpose",
                        description="write eval note",
                        prompt="Workspace: <WT> · branch agent/eval-writer · base main. Write eval-note.txt and commit it.",
                    ),
                    {"text": PARENT_SUMMARY},
                ]
            ),
        },
        prompt="Write an eval note in a worktree using a writing subagent.",
        worktree=True,
        home_files={"hoo-config.json": SHELL_AUTO_ALLOW},
        expect=Expect(
            ledger_statuses=["complete"],
            ledger_ok=[True],
            ledger_agent="general-purpose",
            ledger_mode="blocking",
            child_requests=2,
            parent_requests=3,
            branch_commits=1,
            parent_status_unchanged=True,
            nothing_pushed=True,
        ),
    ),
    Scenario(
        name="subagent_shell_denied_without_a_ui",
        kind="parent",
        doc=(
            "A subagent's Shell call needs approval and its parent (--mode json) has no UI to "
            "ask. The call is denied, not silently run: the child is told so, carries on, and "
            "finishes. Nothing is auto-allowed in the config, so the denial is the gate's."
        ),
        routes={
            "child": child_route(
                [
                    {"tool_calls": [{"name": "Shell", "arguments": {"command": "echo hi"}}]},
                    {"text": CHILD_SUMMARY},
                ]
            ),
            "parent": parent_route(
                [
                    task_call(
                        background=False,
                        subagent_type="general-purpose",
                        description="run a command",
                        prompt="Run echo hi and report.",
                    ),
                    {"text": PARENT_SUMMARY},
                ]
            ),
        },
        prompt="Run a command using a subagent.",
        expect=Expect(
            ledger_statuses=["complete"],
            ledger_ok=[True],
            child_requests=2,
            parent_requests=2,
            route_sees={"child": ["clearance_denied"]},
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
                [task_call(background=False, model="capable"), {"text": PARENT_SUMMARY}]
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
        name="parent_unknown_model_name",
        kind="parent",
        doc=(
            "A typo in the Agent tool's model must never reach a child. The tool refuses a "
            "model name that matches no available model before anything is dispatched, so "
            "there is no dispatch and no ledger line."
        ),
        routes={
            "child": child_route([{"text": CHILD_SUMMARY}]),
            "parent": parent_route([task_call(background=False, model="fastt"), {"text": "done"}]),
        },
        prompt="Describe the repository layout using a subagent.",
        expect=Expect(ledger_count=0, parent_output_contains="unknown model"),
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
        name="parent_legacy_alias_rejected",
        kind="parent",
        doc=(
            "The pre-rename name `Task` is not accepted (decisions-2026-10-08: no aliases "
            "for old names). The call fails as an unknown tool, no child starts, and the "
            "parent still finishes its turn."
        ),
        routes={
            "child": child_route([{"text": CHILD_SUMMARY}]),
            "parent": parent_route(
                [task_call(background=False, tool="Task"), {"text": PARENT_SUMMARY}]
            ),
        },
        prompt="Describe the repository layout using a subagent.",
        expect=Expect(ledger_count=0, child_requests=0, parent_requests=2),
    ),
    Scenario(
        name="parent_queue_saturation",
        kind="parent",
        doc=(
            "More dispatches than the root pool has slots (fourteen against the default twelve, "
            "subagentConcurrency is not pinned so this tests the shipped cap): every one must "
            "settle and be recorded, and never more than twelve may run at once. Nothing may be "
            "left stuck in running."
        ),
        routes={
            # Each child holds its slot for five seconds, so the first wave of twelve overlaps.
            "child": child_route([{"text": CHILD_SUMMARY, "delay_s": 5} for _ in range(14)]),
            "parent": parent_route(
                [
                    {
                        "text": "",
                        "tool_calls": [
                            {"name": "Agent", "arguments": {
                                "subagent_type": "explore",
                                "description": f"map area {i}",
                                "prompt": f"List the top-level directories ({i}).",
                                "background": True,
                            }}
                            for i in range(14)
                        ],
                    },
                    {"text": "All dispatched."},
                    {"text": "All fourteen subagents reported the same layout: crates/, scripts/, docs/."},
                ]
            ),
        },
        prompt="Dispatch fourteen subagents in parallel and report when they are done.",
        timeout_s=240,
        expect=Expect(
            ledger_count=14,
            ledger_statuses=["complete"] * 14,
            attempts_usable_min=1.0,
            peak_inflight_max={"child": 12},
            peak_inflight_min={"child": 12},
        ),
    ),
    Scenario(
        name="nested_depth_chain",
        kind="parent",
        doc=(
            "A cap of three, set explicitly (the shipped default is four): a process at depth d "
            "may spawn while d < 3. The root's child (depth 1) and grandchild (depth 2) get the "
            "Agent tool and the great-grandchild (depth 3) does not. The chain is three blocking "
            "dispatches of a delegate:true agent. "
            "Checks: the depth-3 process is offered no Agent tool, each level's result reaches the "
            "level above it, and all three dispatches are recorded complete."
        ),
        routes={
            "level3": Route("level3", [{"text": CHILD_SUMMARY}], marker="LEVEL-3-MARK"),
            "level2": Route(
                "level2",
                [
                    task_call(
                        background=False,
                        subagent_type="relay",
                        description="relay to level 3",
                        prompt="LEVEL-3-MARK: answer the question directly.",
                    ),
                    {"text": f"Level 3 answered: {CHILD_SUMMARY}"},
                ],
                marker="LEVEL-2-MARK",
            ),
            "level1": Route(
                "level1",
                [
                    task_call(
                        background=False,
                        subagent_type="relay",
                        description="relay to level 2",
                        prompt="LEVEL-2-MARK: relay the task to one subagent.",
                    ),
                    {"text": f"Level 2 answered: {CHILD_SUMMARY}"},
                ],
                marker="LEVEL-1-MARK",
            ),
            "parent": parent_route(
                [
                    task_call(
                        background=False,
                        subagent_type="relay",
                        description="relay to level 1",
                        prompt="LEVEL-1-MARK: relay the task to one subagent.",
                    ),
                    {"text": PARENT_SUMMARY},
                ]
            ),
        },
        prompt="Pass a task down a chain of relay subagents and report what comes back.",
        settings={"maxSubagentDepth": 3},
        home_files={"agents/relay.md": RELAY_AGENT},
        expect=Expect(
            ledger_count=3,
            ledger_statuses=["complete"] * 3,
            ledger_ok=[True] * 3,
            ledger_agent="relay",
            ledger_mode="blocking",
            route_requests={"level1": 2, "level2": 2, "level3": 1, "parent": 2},
            route_tools_have={"level1": ["Agent"], "level2": ["Agent"]},
            route_tools_lack={"level3": ["Agent"]},
            results_flow=["level2", "level1", "parent"],
            parent_output_contains=PARENT_SUMMARY,
        ),
    ),
    Scenario(
        name="nested_fanout_saturation",
        kind="parent",
        doc=(
            "Depth 2 is enough: the root's child (depth 1) fans out five background grandchildren "
            "and its pool runs nestedSubagentConcurrency (3) at once. The grandchildren are slowed "
            "so they overlap. Checks: no more than three are in flight at once, they do overlap, "
            "and all five settle. The grandchildren are at the cap, so they get no Agent tool."
        ),
        routes={
            "grandchild": Route(
                "grandchild",
                [{"text": CHILD_SUMMARY, "delay_s": 5} for _ in range(5)],
                marker="GRANDCHILD-MARK",
            ),
            "fanout": Route(
                "fanout",
                [
                    {
                        "text": "",
                        "tool_calls": [
                            {"name": "Agent", "arguments": {
                                "subagent_type": "relay",
                                "description": f"fanout area {i}",
                                "prompt": f"GRANDCHILD-MARK {i}: list the top-level directories.",
                                "background": True,
                            }}
                            for i in range(5)
                        ],
                    },
                    {"text": "Five subagents dispatched; I will report when they finish."},
                    # Each completed child wakes the relay with its own notification,
                    # so it needs a turn per wake; an exhausted script is a 500.
                    *[{"text": f"All five reported: {CHILD_SUMMARY}"} for _ in range(12)],
                ],
                marker="FANOUT-MARK",
            ),
            "parent": parent_route(
                [
                    task_call(
                        background=False,
                        subagent_type="relay",
                        description="fan out",
                        prompt="FANOUT-MARK: dispatch five subagents, one per area, and report back.",
                    ),
                    {"text": PARENT_SUMMARY},
                ]
            ),
        },
        prompt="Fan a mapping task out to relay subagents and report when they are done.",
        settings={"maxSubagentDepth": 2, "nestedSubagentConcurrency": 3},
        home_files={"agents/relay.md": RELAY_AGENT},
        expect=Expect(
            ledger_count=6,
            ledger_statuses=["complete"] * 6,
            ledger_ok=[True] * 6,
            route_requests={"grandchild": 5, "parent": 2},
            route_tools_lack={"grandchild": ["Agent"]},
            peak_inflight_max={"grandchild": 3},
            peak_inflight_min={"grandchild": 2},
            parent_output_contains=PARENT_SUMMARY,
        ),
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
    peak_inflight: dict[str, int] = field(default_factory=dict)


class ProbedProvider(MockProvider):
    """A MockProvider that counts, per route, how many requests were in flight at once.

    The route is chosen in `pick_route` and the request is served on the same
    handler thread, so a thread-local carries the name from one to the other.
    """

    def __init__(self, *args: Any, **kwargs: Any):
        super().__init__(*args, **kwargs)
        self._local = threading.local()
        self._probe_lock = threading.Lock()
        self._inflight: dict[str, int] = {}
        self.peak_inflight: dict[str, int] = {}

    def pick_route(self, body: dict[str, Any]) -> str:
        # Route on the first user message before the last one. A background
        # subagent is woken by its children's notifications, which are user
        # messages without the task's marker; routing on the last message alone
        # would send that child's later requests to someone else's script.
        first = last_user_text({"messages": [m for m in body.get("messages") or [] if m.get("role") == "user"][:1]})
        name = next(
            (r.name for r in self.routes.values() if r.marker and r.marker in first),
            None,
        ) or super().pick_route(body)
        self._local.route = name
        return name

    def serve(self, handler: Any, turn: dict[str, Any]) -> None:
        route = getattr(self._local, "route", "?")
        with self._probe_lock:
            now = self._inflight.get(route, 0) + 1
            self._inflight[route] = now
            self.peak_inflight[route] = max(self.peak_inflight.get(route, 0), now)
        try:
            super().serve(handler, turn)
        finally:
            with self._probe_lock:
                self._inflight[route] -= 1


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
    for rel, text in scenario.home_files.items():
        path = home / CONFIG_DIR / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
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
        env=dict(scenario.env),
        worktree=scenario.worktree,
        home_files=dict(scenario.home_files),
    )


WRITER_LABEL = "eval-writer"
GIT_ID = ["-c", "user.name=eval", "-c", "user.email=eval@example.invalid"]


def writer_paths(root: Path) -> dict[str, str]:
    """The placeholders a writer scenario's scripts use. All of them are under `root`.

    The worktree sits where orchestration D5 puts it (`<HOME>/.hoocode/worktrees/
    <repo>/<label>`), and HOME is the run's temp home, so it is removed with the run.
    """
    return {
        "<REPO>": str(root / "work"),
        "<WT>": str(root / "home" / CONFIG_DIR / "worktrees" / "work" / WRITER_LABEL),
        "<ORIGIN>": str(root / "origin.git"),
    }


def bind_paths(value: Any, paths: dict[str, str]) -> Any:
    """A copy of a route script with every placeholder replaced by its path."""
    if isinstance(value, str):
        for key, path in paths.items():
            value = value.replace(key, path)
        return value
    if isinstance(value, list):
        return [bind_paths(item, paths) for item in value]
    if isinstance(value, dict):
        return {key: bind_paths(item, paths) for key, item in value.items()}
    return value


def bind_routes(routes: dict[str, Route], paths: dict[str, str]) -> dict[str, Route]:
    return {
        name: Route(name, bind_paths(route.turns, paths), marker=route.marker)
        for name, route in routes.items()
    }


def git(repo: Path, *args: str) -> str:
    """git in `repo`, stdout back. A fixture that fails to build is a harness bug, so it raises."""
    completed = subprocess.run(
        ["git", "-C", str(repo), *GIT_ID, *args], capture_output=True, text=True
    )
    if completed.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {completed.stderr.strip()}")
    return completed.stdout


def origin_refs(origin: Path) -> str:
    completed = subprocess.run(
        ["git", "-C", str(origin), "for-each-ref"], capture_output=True, text=True
    )
    return completed.stdout


def setup_writer_repo(work: Path, origin: Path) -> dict[str, str]:
    """The parent's repo: one commit on main, and a bare origin so a push would be visible.

    `.hoocode/` is excluded because the binary writes its ledger there. Without the
    exclude, every run would change `git status` for reasons that have nothing to do
    with the child under test.
    """
    git(work, "init", "-q", "-b", "main")
    with open(work / ".git" / "info" / "exclude", "a", encoding="utf-8") as handle:
        handle.write(f"{CONFIG_DIR}/\n")
    git(work, "add", "marker.txt")
    git(work, "commit", "-q", "-m", "base")
    subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True, capture_output=True)
    git(work, "remote", "add", "origin", str(origin))
    git(work, "push", "-q", "origin", "main")
    return {"status_before": git(work, "status", "--porcelain"), "origin_before": origin_refs(origin)}


def inspect_writer_repo(work: Path, origin: Path, before: dict[str, str]) -> dict[str, Any]:
    """What the writer left behind, measured against the state before the run."""
    completed = subprocess.run(
        ["git", "-C", str(work), "rev-list", "--count", f"main..agent/{WRITER_LABEL}"],
        capture_output=True,
        text=True,
    )
    return {
        "branch_commits": int(completed.stdout) if completed.returncode == 0 else -1,
        "parent_status_unchanged": git(work, "status", "--porcelain") == before["status_before"],
        "nothing_pushed": origin_refs(origin) == before["origin_before"],
    }


def tool_calls_before_result(entries: list[dict[str, Any]]) -> int:
    """Parent tool calls issued after the first Agent dispatch and before the first
    request that carries the child's result. Zero when either point is missing."""
    dispatch = next((i for i, e in enumerate(entries) if "Agent" in e.get("tool_calls", [])), None)
    result = next((i for i, e in enumerate(entries) if "child_result" in e.get("sees", [])), None)
    if dispatch is None or result is None or result <= dispatch:
        return 0
    return sum(len(e.get("tool_calls", [])) for e in entries[dispatch + 1 : result])


def run_scenario(binary: Path, scenario: Scenario, run_dir: Path) -> Outcome:
    started = time.time()
    root = Path(tempfile.mkdtemp(prefix=f"subagent-eval-{scenario.name}-", dir=run_dir))
    log_path = root / "requests.jsonl"
    routes = bind_routes(scenario.routes, writer_paths(root)) if scenario.worktree else scenario.routes
    provider = ProbedProvider(
        routes, log_path=str(log_path), needles=NEEDLES
    )
    port = provider.start()
    failures: list[str] = []
    exit_code: int | None = None
    before: dict[str, str] = {}
    facts: dict[str, Any] = {}
    origin = root / "origin.git"
    try:
        home, work = write_project(root, port, scenario)
        if scenario.worktree:
            before = setup_writer_repo(work, origin)
        argv = build_argv(binary, scenario)
        # Drop HOOCODE_* from the developer's shell first, then set what the
        # run needs, so the depth below is not stripped with the rest.
        env = {k: v for k, v in os.environ.items() if not k.startswith("HOOCODE_")}
        env.update(
            {
                "HOME": str(home),
                "TERM": "dumb",
                "NO_COLOR": "1",
                "LANG": "en_US.UTF-8",
                # The pool reads depth from the environment; a stale value from
                # the developer's shell would silently change what is tested.
                "HOOCODE_SUBAGENT_DEPTH": "0",
            }
        )
        env.update(scenario.env)
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
        if scenario.worktree:
            facts = inspect_writer_repo(work, origin, before)
        facts["peak_inflight"] = dict(provider.peak_inflight)
        facts["route_entries"] = {name: provider.requests(name) for name in routes}
        failures += check(
            scenario.expect,
            ledger,
            result,
            stdout,
            child_requests,
            parent_requests,
            facts=facts,
            parent_entries=provider.requests("parent"),
        )
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
        peak_inflight=dict(facts.get("peak_inflight", {})),
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
    facts: dict[str, Any] | None = None,
    parent_entries: list[dict[str, Any]] | None = None,
) -> list[str]:
    failures: list[str] = []
    facts = facts or {}

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
    if expect.parent_tool_calls_before_result is not None:
        between = tool_calls_before_result(parent_entries or [])
        want(
            between >= expect.parent_tool_calls_before_result,
            f"parent made {between} tool call(s) between the dispatch and the child's result, "
            f"expected at least {expect.parent_tool_calls_before_result}",
        )
    if expect.branch_commits is not None:
        want(
            facts.get("branch_commits") == expect.branch_commits,
            f"agent/{WRITER_LABEL} has {facts.get('branch_commits')} commit(s) past main, "
            f"expected {expect.branch_commits}",
        )
    if expect.parent_status_unchanged is not None:
        want(
            facts.get("parent_status_unchanged") is expect.parent_status_unchanged,
            "the parent's git status changed during the run",
        )
    if expect.nothing_pushed is not None:
        want(
            facts.get("nothing_pushed") is expect.nothing_pushed,
            "a ref appeared in the origin during the run",
        )
    route_entries: dict[str, list[dict[str, Any]]] = facts.get("route_entries", {})
    peaks: dict[str, int] = facts.get("peak_inflight", {})
    if expect.route_requests is not None:
        for route, count in expect.route_requests.items():
            got = len(route_entries.get(route, []))
            want(got == count, f"{route} made {got} request(s), expected {count}")
    for route, names in (expect.route_tools_have or {}).items():
        offered = {t for e in route_entries.get(route, []) for t in e.get("tools", [])}
        missing = [n for n in names if n not in offered]
        want(not missing, f"{route} was not offered {missing}")
    for route, names in (expect.route_tools_lack or {}).items():
        offered = {t for e in route_entries.get(route, []) for t in e.get("tools", [])}
        present = [n for n in names if n in offered]
        want(not present, f"{route} was offered {present}, expected none")
    for route, names in (expect.route_sees or {}).items():
        seen = {n for e in route_entries.get(route, []) for n in e.get("sees", [])}
        missing = [n for n in names if n not in seen]
        want(not missing, f"{route} was never told {missing}")
    for route in expect.results_flow or []:
        received = any("child_result" in e.get("sees", []) for e in route_entries.get(route, []))
        want(received, f"{route} never received a child's result")
    for route, cap in (expect.peak_inflight_max or {}).items():
        got = peaks.get(route, 0)
        want(got <= cap, f"{route} had {got} requests in flight at once, cap {cap}")
    for route, floor in (expect.peak_inflight_min or {}).items():
        got = peaks.get(route, 0)
        want(got >= floor, f"{route} peaked at {got} in flight, expected at least {floor}")
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
    parser.add_argument("--bin", default=str(DEFAULT_BIN), help="the hoocode binary to exercise")
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
        print(f"error: no binary at {binary}. Build it first:\n  cargo build -p hoocode-code-main", file=sys.stderr)
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
                        "peak_inflight": o.peak_inflight,
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