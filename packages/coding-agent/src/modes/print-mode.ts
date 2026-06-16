/**
 * Print mode (single-shot): Send prompts, output result, exit.
 *
 * Used for:
 * - `hoocode -p "prompt"` - text output
 * - `hoocode --mode json "prompt"` - JSON event stream
 */

import type { AssistantMessage, ImageContent } from "@kolisachint/hoocode-ai";
import type { AgentSessionRuntime } from "../core/agent-session-runtime.js";
import { flushRawStdout, writeRawStdout } from "../core/output-guard.js";
import { buildSubagentResult, buildTaskForest, writeSubagentResult } from "../core/subagent-result.js";
import { taskStore } from "../core/task-store.js";
import { killTrackedDetachedChildren } from "../utils/shell.js";

/** Heartbeat cadence for spawned subagents (parent lifeguard stalls after 60s of silence). */
const SUBAGENT_HEARTBEAT_MS = 30000;

/**
 * Options for print mode.
 */
export interface PrintModeOptions {
	/** Output mode: "text" for final response only, "json" for all events */
	mode: "text" | "json";
	/** Array of additional prompts to send after initialMessage */
	messages?: string[];
	/** First message to send (may contain @file content) */
	initialMessage?: string;
	/** Images to attach to the initial message */
	initialImages?: ImageContent[];
	/** Internal: set when this process is a spawned subagent. Enables heartbeats and result.json. */
	taskId?: string;
	/** Hard cap on assistant turns. Near the cap the agent is asked to wrap up; at the cap it is aborted. */
	maxTurns?: number;
}

/**
 * Run in print (single-shot) mode.
 * Sends prompts to the agent and outputs the result.
 */
export async function runPrintMode(runtimeHost: AgentSessionRuntime, options: PrintModeOptions): Promise<number> {
	const { mode, messages = [], initialMessage, initialImages, taskId, maxTurns } = options;
	let exitCode = 0;
	let session = runtimeHost.session;
	let unsubscribe: (() => void) | undefined;
	let disposed = false;

	// Spawned subagents (json mode + task id) emit a periodic heartbeat so the
	// parent lifeguard does not treat a long-thinking turn as a stall.
	const isSubagent = mode === "json" && typeof taskId === "string" && taskId.length > 0;
	let heartbeat: NodeJS.Timeout | undefined;
	if (isSubagent) {
		// Emit an immediate ping so a child that crashes during startup surfaces its
		// real error instead of stalling silently until the first delayed heartbeat.
		writeRawStdout(`${JSON.stringify({ ping: true })}\n`);
		heartbeat = setInterval(() => {
			writeRawStdout(`${JSON.stringify({ ping: true })}\n`);
		}, SUBAGENT_HEARTBEAT_MS);
		heartbeat.unref();
	}
	// Turn-limit enforcement for spawned subagents: ask the agent to wrap up near
	// the cap, then hard-stop at the cap so a runaway agent always terminates.
	let reachedMaxTurns = false;
	let turnLimitUnsub: (() => void) | undefined;
	const signalCleanupHandlers: Array<() => void> = [];

	const disposeRuntime = async (): Promise<void> => {
		if (disposed) return;
		disposed = true;
		unsubscribe?.();
		await runtimeHost.dispose();
	};

	const registerSignalHandlers = (): void => {
		const signals: NodeJS.Signals[] = ["SIGTERM"];
		if (process.platform !== "win32") {
			signals.push("SIGHUP");
		}

		for (const signal of signals) {
			const handler = () => {
				killTrackedDetachedChildren();
				void disposeRuntime().finally(() => {
					process.exit(signal === "SIGHUP" ? 129 : 143);
				});
			};
			process.on(signal, handler);
			signalCleanupHandlers.push(() => process.off(signal, handler));
		}
	};

	registerSignalHandlers();

	runtimeHost.setRebindSession(async () => {
		await rebindSession();
	});

	const rebindSession = async (): Promise<void> => {
		session = runtimeHost.session;
		await session.bindExtensions({
			commandContextActions: {
				waitForIdle: () => session.agent.waitForIdle(),
				newSession: async (newSessionOptions) => runtimeHost.newSession(newSessionOptions),
				fork: async (entryId, forkOptions) => {
					const result = await runtimeHost.fork(entryId, forkOptions);
					return { cancelled: result.cancelled };
				},
				navigateTree: async (targetId, navigateOptions) => {
					const result = await session.navigateTree(targetId, {
						summarize: navigateOptions?.summarize,
						customInstructions: navigateOptions?.customInstructions,
						replaceInstructions: navigateOptions?.replaceInstructions,
						label: navigateOptions?.label,
					});
					return { cancelled: result.cancelled };
				},
				switchSession: async (sessionPath, switchOptions) => {
					return runtimeHost.switchSession(sessionPath, switchOptions);
				},
				reload: async () => {
					await session.reload();
				},
			},
			onError: (err) => {
				console.error(`Extension error (${err.extensionPath}): ${err.error}`);
			},
		});

		unsubscribe?.();
		unsubscribe = session.subscribe((event) => {
			if (mode === "json") {
				writeRawStdout(`${JSON.stringify(event)}\n`);
			}
		});
	};

	try {
		if (mode === "json") {
			const header = session.sessionManager.getHeader();
			if (header) {
				writeRawStdout(`${JSON.stringify(header)}\n`);
			}
		}

		await rebindSession();

		if (isSubagent && typeof maxTurns === "number" && maxTurns > 0) {
			const cap = maxTurns;
			const wrapUpAt = Math.floor(cap * 0.9);
			let turns = 0;
			let warned = false;
			turnLimitUnsub = session.subscribe((event) => {
				if (event.type !== "turn_end") return;
				turns += 1;
				if (turns >= cap) {
					if (!reachedMaxTurns) {
						reachedMaxTurns = true;
						void session.abort();
					}
					return;
				}
				if (!warned && wrapUpAt >= 1 && wrapUpAt < cap && turns >= wrapUpAt) {
					warned = true;
					void session.steer(
						`You are at turn ${turns} of your ${cap}-turn limit. Stop investigating or making changes now and write your final summary of findings and results in your next message.`,
					);
				}
			});
		}

		if (initialMessage) {
			await session.prompt(initialMessage, { images: initialImages });
		}

		for (const message of messages) {
			await session.prompt(message);
		}

		// Spawned subagents write the audit file the parent pool verifies.
		if (isSubagent && taskId) {
			const stats = session.getSessionStats();
			const result = buildSubagentResult(
				session.state.messages,
				{
					input: stats.tokens.input,
					output: stats.tokens.output,
					cacheRead: stats.tokens.cacheRead,
					cacheWrite: stats.tokens.cacheWrite,
					cost: stats.cost,
				},
				{ reachedMaxTurns },
			);
			// Propagate this subagent's own task subtree (its TodoWrite plan plus any
			// nested delegations/MCP calls) so the parent can render work deeper than
			// one level under the dispatching task.
			const tree = buildTaskForest(taskStore.list());
			if (tree.length > 0) result.task_tree = tree;
			writeSubagentResult(session.sessionManager.getCwd(), taskId, result);
			if (result.status === "failed") exitCode = 1;
		}

		if (mode === "text") {
			const state = session.state;
			const lastMessage = state.messages[state.messages.length - 1];

			if (lastMessage?.role === "assistant") {
				const assistantMsg = lastMessage as AssistantMessage;
				if (assistantMsg.stopReason === "error" || assistantMsg.stopReason === "aborted") {
					console.error(assistantMsg.errorMessage || `Request ${assistantMsg.stopReason}`);
					exitCode = 1;
				} else {
					for (const content of assistantMsg.content) {
						if (content.type === "text") {
							writeRawStdout(`${content.text}\n`);
						}
					}
				}
			}
		}

		return exitCode;
	} catch (error: unknown) {
		console.error(error instanceof Error ? error.message : String(error));
		return 1;
	} finally {
		if (heartbeat) clearInterval(heartbeat);
		turnLimitUnsub?.();
		for (const cleanup of signalCleanupHandlers) {
			cleanup();
		}
		await disposeRuntime();
		await flushRawStdout();
	}
}
