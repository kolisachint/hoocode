# AgentHarness lifecycle

`AgentHarness` is the orchestration layer above the low-level `Agent`. It owns session persistence, runtime configuration, resource resolution, operation locking, and extension-facing mutation semantics.

This document describes the current direction and implemented behavior. Some extension/session-facade details are planned and called out explicitly.

## Ultimate lifecycle goal

Harness listeners and hooks should be able to close over the `AgentHarness` instance and call public harness APIs from any event where those APIs are documented as allowed. Those calls must not corrupt in-flight turn snapshots, reorder persisted transcript entries, lose pending writes, deadlock settlement, or leave the harness in the wrong phase.

The intended rule is:

- structural operations remain rejected while busy
- queue operations are accepted at documented turn-safe points
- runtime config setters update future snapshots without mutating the current provider request
- session writes made while busy are durably queued and flushed in deterministic order
- getters return latest harness config, not in-flight snapshots

A final lifecycle hardening pass should prove these guarantees with a broad listener/hook reentrancy test suite.

## State model

The harness separates state into four categories.

### Harness config

Harness config is the latest runtime configuration set by the application or extensions:

- model
- thinking level
- tools
- active tool names
- resources
- system prompt or system prompt provider

Getters return harness config. They do not return the snapshot used by an in-flight provider request.

Setters update harness config immediately, including while a turn is in flight. Changes affect the next turn snapshot, not the currently running provider request.

`setResources()` accepts concrete resources and emits `resources_update` on every call with shallow-copied current and previous resources. Applications own loading/reloading resources from disk or other sources and should call `setResources()` with new values.

`getResources()` returns shallow-copied current resources. It is a live config read, not the last turn snapshot.

### Turn snapshot

A turn snapshot is the concrete state used for one LLM turn. It is created by `createTurnState()` and contains:

- persisted session messages
- resolved resources
- resolved system prompt
- model
- thinking level
- all tools
- active tools

Static option values are used directly. System-prompt provider callbacks are invoked once per `createTurnState()` call. All logic for that turn uses the same snapshot.

Resource arrays are shallow-copied when a snapshot is created. Individual skill and prompt-template objects are not deep-copied.

### Session

The session contains persisted entries only. Session reads return persisted state and do not include queued writes.

### Pending session writes

Session writes requested while an operation is active are queued as pending session writes. Pending writes are based on session-entry shapes without generated fields (`id`, `parentId`, `timestamp`).

Pending session writes are always persisted. They are flushed at save points, at operation settlement, and in failure cleanup.

A public pending-writes/session-facade API is planned but not implemented yet.

## Operation phases

The harness has an explicit phase:

```ts
type AgentHarnessPhase = "idle" | "turn" | "compaction" | "branch_summary" | "retry";
```

Structural operations require `phase === "idle"` and synchronously set the phase before the first `await`:

- `prompt`
- `skill`
- `promptFromTemplate`
- `compact`
- `navigateTree`

Starting another structural operation while the harness is not idle throws.

The following operations are allowed during a turn where appropriate:

- `steer`
- `followUp`
- `nextTurn`
- `abort`
- runtime config setters

Phase/settlement semantics are still provisional and need a full lifecycle pass.

## Turn execution

`prompt`, `skill`, and `promptFromTemplate` follow the same flow:

1. Assert idle and set phase to `"turn"`.
2. Create a turn snapshot with `createTurnState()`.
3. Derive invocation text from that snapshot.
4. Execute the turn with `executeTurn()`.

`skill` and `promptFromTemplate` resolve their resource from the same snapshot that is passed to the turn. They do not resolve resources separately.

`steer`, `followUp`, and `nextTurn` accept text plus optional images and create user messages internally. `nextTurn` messages are inserted before the new user message on the next user-initiated turn.

Queue modes are live, not turn-snapshotted:

- `steeringMode`
- `followUpMode`

Changing a queue mode during a run affects the next queue drain. Queue drains happen at safe points.

## Save points

A save point occurs after an assistant turn and its tool-result messages have completed.

At a save point the harness:

1. flushes pending session writes after the agent-emitted messages for that turn
2. creates a fresh turn snapshot if the low-level loop may continue
3. applies the fresh context/model/thinking-level state before the next provider request

This lets model, thinking level, tool, resource, and system prompt changes made during a turn affect the next turn in the same run, while never mutating an in-flight provider request. The loop callbacks are not recreated at save points.

The low-level loop converts harness `ThinkingLevel` to provider `reasoning` at the provider boundary:

- `"off"` -> `undefined`
- all other thinking levels pass through

No state refresh is needed on `agent_end` except flushing leftover pending session writes and clearing the operation phase. The exact `settled` event timing is still under review.

If the system-prompt callback throws while starting `prompt`, `skill`, or `promptFromTemplate`, the operation throws and the harness returns to idle. If it throws from the save-point snapshot created by `prepareNextTurn`, the low-level agent run records an assistant error message.

## Hooks and events

Current hooks receive only the event payload. There is no extension context object yet.

Event payloads describe what is happening. Harness getters describe latest config for future snapshots.

The split between harness-specific events (`AgentHarnessOwnEvent`) and the union of low-level plus harness events (`AgentHarnessEvent`) is provisional but useful for distinguishing hookable harness events from public subscription events.

A future extension context may expose the harness and a queued-write session facade.

## Planned session facade

Extensions should eventually interact with a harness-scoped session facade rather than the raw session.

Planned read semantics:

- reads delegate to persisted session state
- reads do not include queued pending writes

Planned write semantics:

- idle: persist immediately
- busy: enqueue as pending session writes

A planned diagnostics API may expose pending writes explicitly:

```ts
getPendingWrites(): readonly PendingSessionWrite[]
```

Agent-emitted messages are persisted on `message_end` to preserve transcript ordering. Pending extension/session writes flush after those messages at save points.

## Abort

Abort is allowed during a turn. It aborts the low-level run and clears low-level steering/follow-up queues.

Abort does not discard pending session writes. Pending writes flush at the next save point if reached, at `agent_end`, or in operation failure cleanup.

Abort barrier semantics still need an audit.

## Compaction and tree navigation

Compaction and tree navigation are structural session mutations.

They are allowed only while idle and are not queued. They operate on persisted session state. The next prompt creates a fresh turn snapshot.

Branch summary generation is part of the tree navigation operation.

Auto-compaction and retry decision points are not implemented in `AgentHarness` yet.

## Final lifecycle hardening todo

Before treating `AgentHarness` as migration-ready, add a broad test suite that exercises listeners and hooks closing over the harness and calling public APIs during every relevant event:

- runtime config setters from low-level lifecycle events and harness events
- resource/tool/model/thinking updates during active turns and save points
- session writes from listeners and hooks, including writes from `settled`
- queue operations from turn events, tool events, and provider hooks
- rejected structural operations while busy
- abort from listeners/hooks
- getter behavior during active operations
- deterministic ordering of agent-emitted messages and pending listener writes
- no deadlocks when async listeners call harness APIs and await them
- phase cleanup through success, provider error, hook error, abort, compaction, and tree navigation
