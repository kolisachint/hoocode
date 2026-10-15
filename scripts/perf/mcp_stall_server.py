#!/usr/bin/env python3
"""A stdio MCP server that accepts a tool call and never answers it.

Used by `scripts/perf/load_scenario.py` (the load run) to check that a stalled MCP
request stays inside its own deadline (`DEFAULT_REQUEST_TIMEOUT`, 60 s) and never
holds up the UI. The handshake (`initialize`, `tools/list`, `ping`) is answered
normally, so the server connects and its one tool, `wait`, is listed. A
`tools/call` is recorded and then left without a reply.

Messages are newline-delimited JSON-RPC on stdin and stdout, as the MCP stdio
transport uses. Nothing is written to stderr, so a full stderr pipe cannot stall
this process.

    scripts/perf/mcp_stall_server.py [accepted-log]

If a path is given, each accepted `tools/call` appends one line to it.
"""

from __future__ import annotations

import json
import sys

PROTOCOL = "2025-06-18"
TOOL = {
    "name": "wait",
    "description": "Accepts the call and never answers (load-scenario fixture).",
    "inputSchema": {"type": "object", "properties": {}},
}


def send(message: dict) -> None:
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def reply(request_id, result=None, error=None) -> None:
    message = {"jsonrpc": "2.0", "id": request_id}
    if error is not None:
        message["error"] = error
    else:
        message["result"] = result if result is not None else {}
    send(message)


def main() -> int:
    accepted = sys.argv[1] if len(sys.argv) > 1 else None
    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = message.get("method")
        request_id = message.get("id")
        if request_id is None:
            # A notification (for example notifications/initialized): no reply.
            continue
        if method == "initialize":
            requested = (message.get("params") or {}).get("protocolVersion", PROTOCOL)
            reply(
                request_id,
                {
                    "protocolVersion": requested,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "stall", "version": "0.1.0"},
                },
            )
        elif method == "tools/list":
            reply(request_id, {"tools": [TOOL]})
        elif method == "ping":
            reply(request_id, {})
        elif method == "tools/call":
            if accepted:
                with open(accepted, "a", encoding="utf-8") as handle:
                    handle.write(json.dumps(message.get("params") or {}) + "\n")
            # Never reply. The client's request deadline has to end this call.
        else:
            reply(request_id, error={"code": -32601, "message": f"method not found: {method}"})
    return 0


if __name__ == "__main__":
    sys.exit(main())
