#!/usr/bin/env python3
"""A routable mock OpenAI-compatible provider for the subagent evals.

`scripts/tui/mockllm.py` serves one global, strictly ordered script,
which is the right shape for a parity transcript and the wrong shape for evals:
a subagent dispatch interleaves the parent's and the child's requests, and the
order in which two processes happen to make calls is not something a test
should depend on.

So this mock routes on the request instead. Every turn the pool builds carries
`Context from the calling agent:` and then `Task: …`, so a body containing
`Task: ` is a child and everything else is a dispatching session. Each route
has its own script and its own cursor.

Turn keys (all optional):

    text             assistant text for this turn
    reasoning        reasoning/thinking text
    tool_calls       [{"name": …, "arguments": {…}}]
    usage            {"prompt_tokens": n, "completion_tokens": n}
    delay_s          sleep before the first byte of the stream
    hang             never answer: the caller's own timeout/lifeguard must fire
    abort_after      send this many bytes of SSE, then close the connection
    corrupt_after    send this many bytes of SSE, then a half-written line
    error            {"status": n, "message": "…"} instead of a stream, with an
                     optional "retry_after" header
    context_overflow answer 400 with the shape providers use when the context no
                     longer fits, which is the one error a subagent cannot
                     recover from by trying again

An exhausted route answers `500 mock: script exhausted` rather than looping, so
a runaway run fails loudly instead of hanging.
"""

from __future__ import annotations

import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

# The pool builds every child's prompt ending in `Task: <task>` (pool.rs
# `build_args`), and the dispatching session's last user message is the user's
# own prompt. Routing on the *last user message only* is what keeps a child's
# request from matching a Task tool description in the parent's body.
CHILD_MARKER = "Task: "


class Route:
    """One scripted conversation.

    `marker` is a substring of the last user message that selects this route.
    The first matching route wins; `MockProvider.default` catches the rest.
    """

    def __init__(
        self,
        name: str,
        turns: list[dict[str, Any]],
        marker: str | None = None,
    ):
        self.name = name
        self.turns = list(turns)
        self.marker = marker
        self.cursor = 0
        self.lock = threading.Lock()

    def next_turn(self) -> dict[str, Any]:
        with self.lock:
            if self.cursor >= len(self.turns):
                return {"error": {"status": 500, "message": "mock: script exhausted"}}
            turn = self.turns[self.cursor]
            self.cursor += 1
            return turn


class QuietServer(ThreadingHTTPServer):
    """A client that hangs up mid-stream is a scenario, not a server error."""

    daemon_threads = True

    def handle_error(self, request, client_address):
        pass


class MockProvider:
    def __init__(
        self,
        routes: dict[str, Route],
        log_path: str | None = None,
        default: str = "parent",
    ):
        self.routes = routes
        self.log_path = log_path
        self.default = default if default in routes else next(iter(routes))
        self.log_lock = threading.Lock()
        self.server: ThreadingHTTPServer | None = None
        self.thread: threading.Thread | None = None

    # -- lifecycle ---------------------------------------------------------
    def start(self) -> int:
        provider = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):  # noqa: A003 - silence stderr spam
                pass

            def do_GET(self):  # noqa: N802
                if self.path.endswith("/models"):
                    self._json(200, {"data": [{"id": "mock-model"}]})
                else:
                    self._json(200, {"ok": True})

            def do_POST(self):  # noqa: N802
                length = int(self.headers.get("content-length") or 0)
                raw = self.rfile.read(length) if length else b"{}"
                try:
                    body = json.loads(raw)
                except json.JSONDecodeError:
                    self._json(400, {"error": {"message": "mock: body is not JSON"}})
                    return
                route_name = provider.pick_route(body)
                route = provider.routes[route_name]
                turn = route.next_turn()
                provider.record(route_name, turn, body)
                provider.serve(self, turn)

        self.server = QuietServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        return self.server.server_address[1]

    def stop(self) -> None:
        if self.server is not None:
            self.server.shutdown()
            self.server.server_close()

    def __enter__(self) -> "MockProvider":
        self.start()
        return self

    def __exit__(self, *exc) -> None:
        self.stop()

    # -- routing -----------------------------------------------------------
    def pick_route(self, body: dict[str, Any]) -> str:
        text = last_user_text(body)
        for route in self.routes.values():
            if route.marker and route.marker in text:
                return route.name
        return self.default

    def record(self, route: str, turn: dict[str, Any], body: dict[str, Any]) -> None:
        if self.log_path is None:
            return
        entry = {
            "route": route,
            "turn_index": self.routes[route].cursor - 1,
            "model": body.get("model"),
            "stream": body.get("stream"),
            "messages": len(body.get("messages") or []),
            "tools": [t.get("function", {}).get("name") for t in body.get("tools") or []],
            "served": sorted(k for k in turn if k not in ("text", "reasoning")),
            "last_user": last_user_text(body)[-120:],
        }
        with self.log_lock:
            with open(self.log_path, "a", encoding="utf-8") as handle:
                handle.write(json.dumps(entry) + "\n")

    def requests(self, route: str | None = None) -> list[dict[str, Any]]:
        if self.log_path is None or not self.logs_written():
            return []
        with self.log_lock, open(self.log_path, encoding="utf-8") as handle:
            entries = [json.loads(line) for line in handle if line.strip()]
        return [e for e in entries if route is None or e["route"] == route]

    def logs_written(self) -> bool:
        import os

        return self.log_path is not None and os.path.exists(self.log_path)

    # -- serving -----------------------------------------------------------
    def serve(self, handler: BaseHTTPRequestHandler, turn: dict[str, Any]) -> None:
        if turn.get("hang"):
            # Hold the connection open without answering. A correct client-side
            # deadline or stall reaper must end this; a hung one fails the eval.
            while True:
                time.sleep(3600)

        if turn.get("delay_s"):
            time.sleep(float(turn["delay_s"]))

        if "error" in turn:
            spec = turn["error"] or {}
            status = int(spec.get("status", 500))
            if spec.get("retry_after"):
                handler.send_response(status)
                handler.send_header("Content-Type", "application/json")
                handler.send_header("retry-after", str(spec["retry_after"]))
                body = json.dumps(
                    {"error": {"message": spec.get("message", "mock: injected error")}}
                ).encode("utf-8")
                handler.send_header("Content-Length", str(len(body)))
                handler.end_headers()
                handler.wfile.write(body)
                return
            handler._json(
                status,
                {
                    "error": {
                        "type": spec.get("type", "invalid_request_error"),
                        "message": spec.get("message", "mock: injected error"),
                        "code": spec.get("code"),
                    }
                },
            )
            return

        chunks = self.sse(turn)
        corrupt_after = turn.get("corrupt_after")
        if corrupt_after:
            chunks = chunks[: max(1, int(corrupt_after) // 20)]
            handler.send_response(200)
            handler.send_header("Content-Type", "text/event-stream")
            handler.end_headers()
            try:
                for chunk in chunks:
                    handler.wfile.write(chunk.encode("utf-8"))
                    handler.wfile.flush()
                # A line that stops mid-JSON, the way a dropped connection
                # looks to a JSONL reader.
                handler.wfile.write(b'data: {"choices":[{"delta":{"content":"half')
                handler.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
            handler.close_connection = True
            return

        abort_after = turn.get("abort_after")
        if abort_after:
            chunks = chunks[: max(1, int(abort_after) // 20)]
            # No Content-Length: the client must see the socket close
            # mid-stream, which is what a provider dying mid-answer looks like.
            handler.send_response(200)
            handler.send_header("Content-Type", "text/event-stream")
            handler.end_headers()
            try:
                for chunk in chunks:
                    handler.wfile.write(chunk.encode("utf-8"))
                    handler.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
            handler.close_connection = True
            return

        handler.send_response(200)
        handler.send_header("Content-Type", "text/event-stream")
        handler.send_header("Cache-Control", "no-cache")
        handler.end_headers()
        try:
            for chunk in chunks:
                handler.wfile.write(chunk.encode("utf-8"))
                handler.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            # The client hung up mid-stream. That is a scenario, not an error.
            return

    def sse(self, turn: dict[str, Any]) -> list[str]:
        usage = turn.get("usage") or {"prompt_tokens": 1200, "completion_tokens": 40}
        out: list[str] = []

        def event(payload: dict[str, Any]) -> str:
            return f"data: {json.dumps(payload)}\n\n"

        out.append(event({"id": "mock", "object": "chat.completion.chunk", "model": "mock-model"}))
        if turn.get("reasoning"):
            out.append(
                event(
                    {
                        "choices": [
                            {"index": 0, "delta": {"reasoning_content": turn["reasoning"]}}
                        ]
                    }
                )
            )
        text = turn.get("text", "")
        if text:
            out.append(
                event({"choices": [{"index": 0, "delta": {"role": "assistant", "content": text}}]})
            )
        for index, call in enumerate(turn.get("tool_calls") or []):
            out.append(
                event(
                    {
                        "choices": [
                            {
                                "index": 0,
                                "delta": {
                                    "tool_calls": [
                                        {
                                            "index": index,
                                            "id": call.get("id", f"call_{index}"),
                                            "type": "function",
                                            "function": {
                                                "name": call["name"],
                                                "arguments": json.dumps(call.get("arguments", {})),
                                            },
                                        }
                                    ]
                                },
                            }
                        ]
                    }
                )
            )
        out.append(
            event(
                {
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"
                                 if turn.get("tool_calls") else "stop"}],
                    "usage": {
                        "prompt_tokens": usage.get("prompt_tokens", 0),
                        "completion_tokens": usage.get("completion_tokens", 0),
                        "total_tokens": usage.get("prompt_tokens", 0)
                        + usage.get("completion_tokens", 0),
                    },
                }
            )
        )
        out.append("data: [DONE]\n\n")
        return out


def last_user_text(body: dict[str, Any]) -> str:
    """The last user message, flattened to text. Chat-completions shape."""
    for message in reversed(body.get("messages") or []):
        if message.get("role") != "user":
            continue
        content = message.get("content")
        if isinstance(content, str):
            return content
        if isinstance(content, list):
            parts = [
                part.get("text", "")
                for part in content
                if isinstance(part, dict) and part.get("type") in ("text", "input_text")
            ]
            return "\n".join(parts)
    return ""


def _json(self, status: int, payload: dict[str, Any]) -> None:
    body = json.dumps(payload).encode("utf-8")
    self.send_response(status)
    self.send_header("Content-Type", "application/json")
    self.send_header("Content-Length", str(len(body)))
    self.end_headers()
    self.wfile.write(body)


BaseHTTPRequestHandler._json = _json  # type: ignore[attr-defined]


if __name__ == "__main__":  # pragma: no cover - manual smoke test
    import sys

    script = json.loads(sys.argv[1]) if len(sys.argv) > 1 else {}
    provider = MockProvider(
        {name: Route(name, turns) for name, turns in script.items()}, log_path=None
    )
    print(provider.start())
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        provider.stop()