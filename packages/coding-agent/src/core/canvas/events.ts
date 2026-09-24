/**
 * What the agent is doing, as session events a canvas can subscribe to.
 *
 * A canvas in front of a person needs to say, quietly, whether the agent is busy
 * and on what — otherwise the person cannot tell a slow turn from an idle agent,
 * or whether the thing they asked for has been picked up. The SDK already names
 * those signals, so hoocode emits GitHub's event types with GitHub's field names
 * rather than inventing a vocabulary (see `CANVAS_SESSION_EVENT_TYPES`).
 *
 * Deliberately little crosses: a tool's name and a one-line title, never its
 * arguments or output. A canvas is a separate process and a status line needs
 * no file contents.
 *
 * `assistant.intent` is the agent's own sentence in the Copilot CLI; hoocode's
 * models do not report one, so it is derived from the tool being run and sent
 * only when it changes.
 */

import { randomUUID } from "node:crypto";
import type { CanvasSessionEvent, CanvasSessionEventType, JsonValue } from "./protocol.js";

/** Longest tool title or intent sent, in characters. */
export const CANVAS_EVENT_TITLE_MAX = 80;

/** One todo, as `session.todos_changed` carries it. */
export interface CanvasTodo {
	id: number;
	title: string;
	status: string;
}

function clip(text: string, max = CANVAS_EVENT_TITLE_MAX): string {
	const line = text.replace(/\s+/g, " ").trim();
	return line.length <= max ? line : `${line.slice(0, max - 1)}…`;
}

function field(args: unknown, key: string): string | undefined {
	if (typeof args !== "object" || args === null) return undefined;
	const value = (args as Record<string, unknown>)[key];
	return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * A one-line title for a tool call, from the arguments a person would recognise:
 * the path, the command, the query, the canvas action. Unknown tools get their
 * name alone.
 */
export function toolTitle(toolName: string, args: unknown): string {
	const path = field(args, "path");
	switch (toolName) {
		case "read":
			return clip(path ? `read ${path}` : "read");
		case "edit":
		case "write":
			return clip(path ? `${toolName} ${path}` : toolName);
		case "bash": {
			const command = field(args, "command");
			return clip(command ? `$ ${command}` : "bash");
		}
		case "invoke_canvas_action": {
			const action = field(args, "action");
			return clip(action ? `canvas: ${action}` : "canvas");
		}
		default: {
			const query = field(args, "query") ?? field(args, "pattern") ?? field(args, "url") ?? path;
			return clip(query ? `${toolName} ${query}` : toolName);
		}
	}
}

/**
 * Turns hoocode's agent lifecycle into canvas session events.
 *
 * Stateful only where the SDK's events are: `tool.execution_complete` carries no
 * tool name upstream, so start titles are remembered by call id; intents and
 * todos are sent only when they change.
 */
export class CanvasEventSource {
	private readonly emit: (event: CanvasSessionEvent) => void;
	private readonly now: () => Date;
	private readonly titles = new Map<string, string>();
	private lastIntent: string | undefined;
	private lastTodos = "";
	private turnId: string | undefined;

	constructor(emit: (event: CanvasSessionEvent) => void, now: () => Date = () => new Date()) {
		this.emit = emit;
		this.now = now;
	}

	/** The agent started a run. */
	turnStart(): void {
		this.turnId = randomUUID();
		this.lastIntent = undefined;
		this.send("assistant.turn_start", { turnId: this.turnId }, false);
	}

	/** A tool started. Also moves the intent, if the title says something new. */
	toolStart(toolCallId: string, toolName: string, args: unknown): void {
		const title = toolTitle(toolName, args);
		this.titles.set(toolCallId, title);
		this.send("tool.execution_start", { toolCallId, toolName, toolTitle: title }, true);
		if (title !== this.lastIntent) {
			this.lastIntent = title;
			this.send("assistant.intent", { intent: title }, true);
		}
	}

	/** A tool finished. */
	toolEnd(toolCallId: string, isError: boolean): void {
		const title = this.titles.get(toolCallId);
		this.titles.delete(toolCallId);
		const data: { [key: string]: JsonValue } = { toolCallId, success: !isError };
		if (title) data.toolTitle = title;
		this.send("tool.execution_complete", data, true);
	}

	/** The agent is idle. */
	idle(aborted = false): void {
		this.titles.clear();
		this.lastIntent = undefined;
		this.turnId = undefined;
		this.send("session.idle", aborted ? { aborted } : {}, true);
	}

	/**
	 * The agent's todo list, sent when it differs from the last one sent.
	 *
	 * Upstream sends this event empty and expects a follow-up RPC read; hoocode has
	 * no such read, so the list rides in `data.todos`. A canvas written for upstream
	 * ignores the extra field.
	 */
	todos(todos: readonly CanvasTodo[]): void {
		const list = todos.map((todo) => ({ id: todo.id, title: clip(todo.title, 120), status: todo.status }));
		const key = JSON.stringify(list);
		if (key === this.lastTodos) return;
		this.lastTodos = key;
		this.send("session.todos_changed", { todos: list }, true);
	}

	private send(type: CanvasSessionEventType, data: { [key: string]: JsonValue }, ephemeral: boolean): void {
		this.emit({ id: randomUUID(), timestamp: this.now().toISOString(), parentId: null, ephemeral, type, data });
	}
}
