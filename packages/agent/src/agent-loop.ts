/**
 * Agent loop that works with AgentMessage throughout.
 * Transforms to Message[] only at the LLM call boundary.
 */

import {
	type AssistantMessage,
	type Context,
	EventStream,
	streamSimple,
	type ToolResultMessage,
	type UserMessage,
	validateToolArguments,
} from "@kolisachint/hoocode-ai";
import type {
	AgentContext,
	AgentEvent,
	AgentLoopConfig,
	AgentMessage,
	AgentTool,
	AgentToolCall,
	AgentToolResult,
	StreamFn,
} from "./types.js";

export type AgentEventSink = (event: AgentEvent) => Promise<void> | void;

/**
 * Start an agent loop with a new prompt message.
 * The prompt is added to the context and events are emitted for it.
 */
export function agentLoop(
	prompts: AgentMessage[],
	context: AgentContext,
	config: AgentLoopConfig,
	signal?: AbortSignal,
	streamFn?: StreamFn,
): EventStream<AgentEvent, AgentMessage[]> {
	const stream = createAgentStream();

	void runAgentLoop(
		prompts,
		context,
		config,
		async (event) => {
			stream.push(event);
		},
		signal,
		streamFn,
	).then((messages) => {
		stream.end(messages);
	});

	return stream;
}

/**
 * Continue an agent loop from the current context without adding a new message.
 * Used for retries - context already has user message or tool results.
 *
 * **Important:** The last message in context must convert to a `user` or `toolResult` message
 * via `convertToLlm`. If it doesn't, the LLM provider will reject the request.
 * This cannot be validated here since `convertToLlm` is only called once per turn.
 */
export function agentLoopContinue(
	context: AgentContext,
	config: AgentLoopConfig,
	signal?: AbortSignal,
	streamFn?: StreamFn,
): EventStream<AgentEvent, AgentMessage[]> {
	if (context.messages.length === 0) {
		throw new Error("Cannot continue: no messages in context");
	}

	if (context.messages[context.messages.length - 1].role === "assistant") {
		throw new Error("Cannot continue from message role: assistant");
	}

	const stream = createAgentStream();

	void runAgentLoopContinue(
		context,
		config,
		async (event) => {
			stream.push(event);
		},
		signal,
		streamFn,
	).then((messages) => {
		stream.end(messages);
	});

	return stream;
}

export async function runAgentLoop(
	prompts: AgentMessage[],
	context: AgentContext,
	config: AgentLoopConfig,
	emit: AgentEventSink,
	signal?: AbortSignal,
	streamFn?: StreamFn,
): Promise<AgentMessage[]> {
	const newMessages: AgentMessage[] = [...prompts];
	const currentContext: AgentContext = {
		...context,
		messages: [...context.messages, ...prompts],
	};

	await emit({ type: "agent_start" });
	await emit({ type: "turn_start" });
	for (const prompt of prompts) {
		await emit({ type: "message_start", message: prompt });
		await emit({ type: "message_end", message: prompt });
	}

	await runLoop(currentContext, newMessages, config, signal, emit, streamFn);
	return newMessages;
}

export async function runAgentLoopContinue(
	context: AgentContext,
	config: AgentLoopConfig,
	emit: AgentEventSink,
	signal?: AbortSignal,
	streamFn?: StreamFn,
): Promise<AgentMessage[]> {
	if (context.messages.length === 0) {
		throw new Error("Cannot continue: no messages in context");
	}

	if (context.messages[context.messages.length - 1].role === "assistant") {
		throw new Error("Cannot continue from message role: assistant");
	}

	const newMessages: AgentMessage[] = [];
	const currentContext: AgentContext = { ...context };

	await emit({ type: "agent_start" });
	await emit({ type: "turn_start" });

	await runLoop(currentContext, newMessages, config, signal, emit, streamFn);
	return newMessages;
}

function createAgentStream(): EventStream<AgentEvent, AgentMessage[]> {
	return new EventStream<AgentEvent, AgentMessage[]>(
		(event: AgentEvent) => event.type === "agent_end",
		(event: AgentEvent) => (event.type === "agent_end" ? event.messages : []),
	);
}

/**
 * Main loop logic shared by agentLoop and agentLoopContinue.
 */
async function runLoop(
	initialContext: AgentContext,
	newMessages: AgentMessage[],
	initialConfig: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
	streamFn?: StreamFn,
): Promise<void> {
	let currentContext = initialContext;
	let config = initialConfig;
	let firstTurn = true;

	// Tracks tool calls that run detached (background tools). Their results are
	// injected into a later turn as follow-up messages instead of blocking the loop.
	const background = createBackgroundTaskManager();

	// Collect messages to inject before the next assistant turn: results from any
	// finished background tools take priority, then app-provided steering messages.
	const collectPendingMessages = async (): Promise<AgentMessage[]> => {
		const backgroundResults = background.drainResults();
		const steering = (await config.getSteeringMessages?.()) || [];
		return [...backgroundResults, ...steering];
	};

	// Check for steering messages at start (user may have typed while waiting)
	let pendingMessages: AgentMessage[] = await collectPendingMessages();

	// Outer loop: continues when queued follow-up messages arrive after agent would stop
	while (true) {
		let hasMoreToolCalls = true;

		// Inner loop: process tool calls, steering messages, and background tool results.
		// Background tools keep the loop alive: while one is still running we stay in the
		// inner loop so the agent can react to its result once it lands.
		while (hasMoreToolCalls || pendingMessages.length > 0 || background.pendingCount() > 0) {
			// Nothing new to act on yet, but background work is still in flight: wait for the
			// next background task to settle (rather than spinning an empty turn), then inject
			// whatever it produced. If it produced no message, re-evaluate the loop condition.
			if (!hasMoreToolCalls && pendingMessages.length === 0) {
				await background.waitForNext();
				pendingMessages = await collectPendingMessages();
				if (pendingMessages.length === 0) {
					continue;
				}
			}

			if (!firstTurn) {
				await emit({ type: "turn_start" });
			} else {
				firstTurn = false;
			}

			// Process pending messages (inject before next assistant response)
			if (pendingMessages.length > 0) {
				for (const message of pendingMessages) {
					await emit({ type: "message_start", message });
					await emit({ type: "message_end", message });
					currentContext.messages.push(message);
					newMessages.push(message);
				}
				pendingMessages = [];
			}

			// Stream assistant response
			const message = await streamAssistantResponse(currentContext, config, signal, emit, streamFn);
			newMessages.push(message);

			if (message.stopReason === "error" || message.stopReason === "aborted") {
				await emit({ type: "turn_end", message, toolResults: [] });
				await emit({ type: "agent_end", messages: newMessages });
				return;
			}

			// Check for tool calls
			const toolCalls = message.content.filter((c) => c.type === "toolCall");

			const toolResults: ToolResultMessage[] = [];
			hasMoreToolCalls = false;
			if (toolCalls.length > 0) {
				const executedToolBatch = await executeToolCalls(currentContext, message, config, signal, emit, background);
				toolResults.push(...executedToolBatch.messages);
				hasMoreToolCalls = !executedToolBatch.terminate;

				for (const result of toolResults) {
					currentContext.messages.push(result);
					newMessages.push(result);
				}
			}

			await emit({ type: "turn_end", message, toolResults });

			const nextTurnContext = {
				message,
				toolResults,
				context: currentContext,
				newMessages,
			};
			const nextTurnSnapshot = await config.prepareNextTurn?.(nextTurnContext);
			if (nextTurnSnapshot) {
				currentContext = nextTurnSnapshot.context ?? currentContext;
				config = {
					...config,
					model: nextTurnSnapshot.model ?? config.model,
					reasoning:
						nextTurnSnapshot.thinkingLevel === undefined
							? config.reasoning
							: nextTurnSnapshot.thinkingLevel === "off"
								? undefined
								: nextTurnSnapshot.thinkingLevel,
				};
			}

			if (
				await config.shouldStopAfterTurn?.({
					message,
					toolResults,
					context: currentContext,
					newMessages,
				})
			) {
				await emit({ type: "agent_end", messages: newMessages });
				return;
			}

			pendingMessages = await collectPendingMessages();
		}

		// Agent would stop here. Check for follow-up messages.
		const followUpMessages = (await config.getFollowUpMessages?.()) || [];
		if (followUpMessages.length > 0) {
			// Set as pending so inner loop processes them
			pendingMessages = followUpMessages;
			continue;
		}

		// No more messages, exit
		break;
	}

	await emit({ type: "agent_end", messages: newMessages });
}

/**
 * Stream an assistant response from the LLM.
 * This is where AgentMessage[] gets transformed to Message[] for the LLM.
 */
async function streamAssistantResponse(
	context: AgentContext,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
	streamFn?: StreamFn,
): Promise<AssistantMessage> {
	// Apply context transform if configured (AgentMessage[] → AgentMessage[])
	let messages = context.messages;
	if (config.transformContext) {
		messages = await config.transformContext(messages, signal);
	}

	// Convert to LLM-compatible messages (AgentMessage[] → Message[])
	const llmMessages = await config.convertToLlm(messages);

	// Build LLM context
	const llmContext: Context = {
		systemPrompt: context.systemPrompt,
		messages: llmMessages,
		tools: context.tools,
	};

	const streamFunction = streamFn || streamSimple;

	// Resolve API key (important for expiring tokens)
	const resolvedApiKey =
		(config.getApiKey ? await config.getApiKey(config.model.provider) : undefined) || config.apiKey;

	const response = await streamFunction(config.model, llmContext, {
		...config,
		apiKey: resolvedApiKey,
		signal,
	});

	let partialMessage: AssistantMessage | null = null;
	let addedPartial = false;

	for await (const event of response) {
		switch (event.type) {
			case "start":
				partialMessage = event.partial;
				context.messages.push(partialMessage);
				addedPartial = true;
				await emit({ type: "message_start", message: { ...partialMessage } });
				break;

			case "text_start":
			case "text_delta":
			case "text_end":
			case "thinking_start":
			case "thinking_delta":
			case "thinking_end":
			case "toolcall_start":
			case "toolcall_delta":
			case "toolcall_end":
				if (partialMessage) {
					partialMessage = event.partial;
					context.messages[context.messages.length - 1] = partialMessage;
					await emit({
						type: "message_update",
						assistantMessageEvent: event,
						message: { ...partialMessage },
					});
				}
				break;

			case "done":
			case "error": {
				const finalMessage = await response.result();
				if (addedPartial) {
					context.messages[context.messages.length - 1] = finalMessage;
				} else {
					context.messages.push(finalMessage);
				}
				if (!addedPartial) {
					await emit({ type: "message_start", message: { ...finalMessage } });
				}
				await emit({ type: "message_end", message: finalMessage });
				return finalMessage;
			}
		}
	}

	const finalMessage = await response.result();
	if (addedPartial) {
		context.messages[context.messages.length - 1] = finalMessage;
	} else {
		context.messages.push(finalMessage);
		await emit({ type: "message_start", message: { ...finalMessage } });
	}
	await emit({ type: "message_end", message: finalMessage });
	return finalMessage;
}

/**
 * Execute tool calls from an assistant message.
 *
 * Tool calls are partitioned into foreground and background. Foreground calls are
 * awaited (sequentially or in parallel). Background calls (`tool.background === true`)
 * return a placeholder tool result immediately and run detached via `background`;
 * the loop injects their real results into a later turn as follow-up messages.
 *
 * Tool result messages are returned in assistant source order so every tool call is
 * answered in place, regardless of which partition produced it.
 */
async function executeToolCalls(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
	background: BackgroundTaskManager,
): Promise<ExecutedToolCallBatch> {
	const toolCalls = assistantMessage.content.filter((c) => c.type === "toolCall");
	// Single pass so a per-call `background` predicate is evaluated exactly once.
	const backgroundCalls: AgentToolCall[] = [];
	const foregroundCalls: AgentToolCall[] = [];
	for (const toolCall of toolCalls) {
		(isBackgroundTool(currentContext, toolCall) ? backgroundCalls : foregroundCalls).push(toolCall);
	}

	// Dispatch background tool calls first so their placeholder results are ready
	// before any (potentially slow) foreground tools start executing.
	const backgroundMessages = await dispatchBackgroundToolCalls(
		currentContext,
		assistantMessage,
		backgroundCalls,
		config,
		signal,
		emit,
		background,
	);

	let foregroundBatch: ExecutedToolCallBatch = { messages: [], terminate: false };
	if (foregroundCalls.length > 0) {
		const hasSequentialToolCall = foregroundCalls.some(
			(tc) => currentContext.tools?.find((t) => t.name === tc.name)?.executionMode === "sequential",
		);
		foregroundBatch =
			config.toolExecution === "sequential" || hasSequentialToolCall
				? await executeToolCallsSequential(currentContext, assistantMessage, foregroundCalls, config, signal, emit)
				: await executeToolCallsParallel(currentContext, assistantMessage, foregroundCalls, config, signal, emit);
	}

	const messages = orderToolResultsBySource(toolCalls, [...backgroundMessages, ...foregroundBatch.messages]);
	return {
		messages,
		// Only foreground results can request early termination; a batch made up
		// entirely of background dispatches never terminates the loop.
		terminate: foregroundBatch.terminate,
	};
}

function isBackgroundTool(currentContext: AgentContext, toolCall: AgentToolCall): boolean {
	const background = currentContext.tools?.find((t) => t.name === toolCall.name)?.background;
	if (typeof background === "function") {
		try {
			return background(toolCall) === true;
		} catch {
			// A throwing predicate must not break tool dispatch; treat as foreground.
			return false;
		}
	}
	return background === true;
}

/** Reorder finalized tool result messages to match the assistant's tool-call order. */
function orderToolResultsBySource(toolCalls: AgentToolCall[], messages: ToolResultMessage[]): ToolResultMessage[] {
	const byId = new Map(messages.map((message) => [message.toolCallId, message]));
	const ordered: ToolResultMessage[] = [];
	for (const toolCall of toolCalls) {
		const message = byId.get(toolCall.id);
		if (message) {
			ordered.push(message);
		}
	}
	return ordered;
}

/**
 * Dispatch background tool calls without blocking the loop.
 *
 * For each background call we emit a placeholder tool result immediately (satisfying
 * the assistant's tool call) and kick off the real execution detached. When that work
 * finishes, its result is queued on the {@link BackgroundTaskManager} as a follow-up
 * user message for a later turn. Preparation failures (unknown tool, invalid args,
 * blocked by `beforeToolCall`) resolve synchronously, exactly like foreground calls.
 */
async function dispatchBackgroundToolCalls(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	toolCalls: AgentToolCall[],
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
	background: BackgroundTaskManager,
): Promise<ToolResultMessage[]> {
	const messages: ToolResultMessage[] = [];

	for (const toolCall of toolCalls) {
		await emit({
			type: "tool_execution_start",
			toolCallId: toolCall.id,
			toolName: toolCall.name,
			args: toolCall.arguments,
		});

		const preparation = await prepareToolCall(currentContext, assistantMessage, toolCall, config, signal);
		if (preparation.kind === "immediate") {
			const finalized: FinalizedToolCallOutcome = {
				toolCall,
				result: preparation.result,
				isError: preparation.isError,
			};
			await emitToolExecutionEnd(finalized, emit);
			const message = createToolResultMessage(finalized);
			await emitToolResultMessage(message, emit);
			messages.push(message);
			continue;
		}

		// The tool was prepared successfully: answer the tool call with a placeholder now,
		// run the real work detached, and surface its result on a later turn.
		const placeholder = createBackgroundPlaceholderOutcome(toolCall);
		await emitToolExecutionEnd(placeholder, emit);
		const placeholderMessage = createToolResultMessage(placeholder);
		await emitToolResultMessage(placeholderMessage, emit);
		messages.push(placeholderMessage);

		background.spawn(async () => {
			// Background tools own their lifecycle; suppress streaming update events to
			// avoid emitting updates for a tool whose execution already "ended" above.
			const executed = await executePreparedToolCall(preparation, signal, NOOP_EMIT);
			const finalized = await finalizeExecutedToolCall(
				currentContext,
				assistantMessage,
				preparation,
				executed,
				config,
				signal,
			);
			return createBackgroundResultMessage(finalized);
		});
	}

	return messages;
}

const NOOP_EMIT: AgentEventSink = () => {};

/** Placeholder result returned immediately for a dispatched background tool call. */
function createBackgroundPlaceholderOutcome(toolCall: AgentToolCall): FinalizedToolCallOutcome {
	return {
		toolCall,
		result: {
			content: [
				{
					type: "text",
					text: `Started "${toolCall.name}" in the background. Its result will arrive as a follow-up message once it finishes — keep working in the meantime.`,
				},
			],
			details: { background: true, status: "running" },
		},
		isError: false,
	};
}

/**
 * Build the follow-up user message that carries a finished background tool's result.
 *
 * The tool call itself was already answered by a placeholder, so the real result is
 * delivered as a fresh user message (rather than a second tool result for the same id)
 * and injected like a steering message on the next loop iteration.
 */
function createBackgroundResultMessage(finalized: FinalizedToolCallOutcome): UserMessage {
	const header = finalized.isError
		? `Background tool "${finalized.toolCall.name}" (id ${finalized.toolCall.id}) failed:`
		: `Background tool "${finalized.toolCall.name}" (id ${finalized.toolCall.id}) finished:`;
	return {
		role: "user",
		content: [{ type: "text", text: header }, ...finalized.result.content],
		timestamp: Date.now(),
	};
}

/**
 * Tracks background tool calls that run detached from the main loop.
 *
 * The loop polls {@link BackgroundTaskManager.pendingCount} to stay alive while work
 * is in flight, awaits {@link BackgroundTaskManager.waitForNext} when it has nothing
 * else to do, and drains finished results via {@link BackgroundTaskManager.drainResults}.
 */
interface BackgroundTaskManager {
	/** Number of background tool calls still executing. */
	pendingCount(): number;
	/** Run a background tool call detached; its resolved message is queued for the loop. */
	spawn(run: () => Promise<AgentMessage>): void;
	/** Remove and return all finished background result messages. */
	drainResults(): AgentMessage[];
	/** Resolve once another in-flight task settles, or immediately if none are pending. */
	waitForNext(): Promise<void>;
}

function createBackgroundTaskManager(): BackgroundTaskManager {
	const results: AgentMessage[] = [];
	const inflight = new Set<Promise<void>>();
	let waiter: { promise: Promise<void>; resolve: () => void } | undefined;

	function signalSettled(): void {
		if (waiter) {
			const current = waiter;
			waiter = undefined;
			current.resolve();
		}
	}

	return {
		pendingCount: () => inflight.size,
		drainResults: () => results.splice(0, results.length),
		waitForNext: () => {
			if (results.length > 0 || inflight.size === 0) {
				return Promise.resolve();
			}
			if (!waiter) {
				let resolve!: () => void;
				const promise = new Promise<void>((r) => {
					resolve = r;
				});
				waiter = { promise, resolve };
			}
			return waiter.promise;
		},
		spawn: (run) => {
			const task = (async () => {
				try {
					results.push(await run());
				} catch {
					// `run` is expected to encode failures into its result message, so a throw
					// here is unexpected; drop it rather than crash the detached task.
				}
			})();
			inflight.add(task);
			void task.finally(() => {
				inflight.delete(task);
				signalSettled();
			});
		},
	};
}

type ExecutedToolCallBatch = {
	messages: ToolResultMessage[];
	terminate: boolean;
};

async function executeToolCallsSequential(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	toolCalls: AgentToolCall[],
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
): Promise<ExecutedToolCallBatch> {
	const finalizedCalls: FinalizedToolCallOutcome[] = [];
	const messages: ToolResultMessage[] = [];

	for (const toolCall of toolCalls) {
		await emit({
			type: "tool_execution_start",
			toolCallId: toolCall.id,
			toolName: toolCall.name,
			args: toolCall.arguments,
		});

		const preparation = await prepareToolCall(currentContext, assistantMessage, toolCall, config, signal);
		let finalized: FinalizedToolCallOutcome;
		if (preparation.kind === "immediate") {
			finalized = {
				toolCall,
				result: preparation.result,
				isError: preparation.isError,
			};
		} else {
			const executed = await executePreparedToolCall(preparation, signal, emit);
			finalized = await finalizeExecutedToolCall(
				currentContext,
				assistantMessage,
				preparation,
				executed,
				config,
				signal,
			);
		}

		await emitToolExecutionEnd(finalized, emit);
		const toolResultMessage = createToolResultMessage(finalized);
		await emitToolResultMessage(toolResultMessage, emit);
		finalizedCalls.push(finalized);
		messages.push(toolResultMessage);
	}

	return {
		messages,
		terminate: shouldTerminateToolBatch(finalizedCalls),
	};
}

async function executeToolCallsParallel(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	toolCalls: AgentToolCall[],
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
): Promise<ExecutedToolCallBatch> {
	const finalizedCalls: FinalizedToolCallEntry[] = [];

	for (const toolCall of toolCalls) {
		await emit({
			type: "tool_execution_start",
			toolCallId: toolCall.id,
			toolName: toolCall.name,
			args: toolCall.arguments,
		});

		const preparation = await prepareToolCall(currentContext, assistantMessage, toolCall, config, signal);
		if (preparation.kind === "immediate") {
			const finalized = {
				toolCall,
				result: preparation.result,
				isError: preparation.isError,
			} satisfies FinalizedToolCallOutcome;
			await emitToolExecutionEnd(finalized, emit);
			finalizedCalls.push(finalized);
			continue;
		}

		finalizedCalls.push(async () => {
			const executed = await executePreparedToolCall(preparation, signal, emit);
			const finalized = await finalizeExecutedToolCall(
				currentContext,
				assistantMessage,
				preparation,
				executed,
				config,
				signal,
			);
			await emitToolExecutionEnd(finalized, emit);
			return finalized;
		});
	}

	const orderedFinalizedCalls = await Promise.all(
		finalizedCalls.map((entry) => (typeof entry === "function" ? entry() : Promise.resolve(entry))),
	);
	const messages: ToolResultMessage[] = [];
	for (const finalized of orderedFinalizedCalls) {
		const toolResultMessage = createToolResultMessage(finalized);
		await emitToolResultMessage(toolResultMessage, emit);
		messages.push(toolResultMessage);
	}

	return {
		messages,
		terminate: shouldTerminateToolBatch(orderedFinalizedCalls),
	};
}

type PreparedToolCall = {
	kind: "prepared";
	toolCall: AgentToolCall;
	tool: AgentTool<any>;
	args: unknown;
};

type ImmediateToolCallOutcome = {
	kind: "immediate";
	result: AgentToolResult<any>;
	isError: boolean;
};

type ExecutedToolCallOutcome = {
	result: AgentToolResult<any>;
	isError: boolean;
};

type FinalizedToolCallOutcome = {
	toolCall: AgentToolCall;
	result: AgentToolResult<any>;
	isError: boolean;
};

type FinalizedToolCallEntry = FinalizedToolCallOutcome | (() => Promise<FinalizedToolCallOutcome>);

function shouldTerminateToolBatch(finalizedCalls: FinalizedToolCallOutcome[]): boolean {
	return finalizedCalls.length > 0 && finalizedCalls.every((finalized) => finalized.result.terminate === true);
}

function prepareToolCallArguments(tool: AgentTool<any>, toolCall: AgentToolCall): AgentToolCall {
	if (!tool.prepareArguments) {
		return toolCall;
	}
	const preparedArguments = tool.prepareArguments(toolCall.arguments);
	if (preparedArguments === toolCall.arguments) {
		return toolCall;
	}
	return {
		...toolCall,
		arguments: preparedArguments as Record<string, any>,
	};
}

async function prepareToolCall(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	toolCall: AgentToolCall,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
): Promise<PreparedToolCall | ImmediateToolCallOutcome> {
	const tool = currentContext.tools?.find((t) => t.name === toolCall.name);
	if (!tool) {
		return {
			kind: "immediate",
			result: createErrorToolResult(`Tool ${toolCall.name} not found`),
			isError: true,
		};
	}

	try {
		const preparedToolCall = prepareToolCallArguments(tool, toolCall);
		const validatedArgs = validateToolArguments(tool, preparedToolCall);
		if (config.beforeToolCall) {
			const beforeResult = await config.beforeToolCall(
				{
					assistantMessage,
					toolCall,
					args: validatedArgs,
					context: currentContext,
				},
				signal,
			);
			if (beforeResult?.block) {
				return {
					kind: "immediate",
					result: createErrorToolResult(beforeResult.reason || "Tool execution was blocked"),
					isError: true,
				};
			}
		}
		return {
			kind: "prepared",
			toolCall,
			tool,
			args: validatedArgs,
		};
	} catch (error) {
		return {
			kind: "immediate",
			result: createErrorToolResult(error instanceof Error ? error.message : String(error)),
			isError: true,
		};
	}
}

async function executePreparedToolCall(
	prepared: PreparedToolCall,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
): Promise<ExecutedToolCallOutcome> {
	const updateEvents: Promise<void>[] = [];

	try {
		const result = await prepared.tool.execute(
			prepared.toolCall.id,
			prepared.args as never,
			signal,
			(partialResult) => {
				updateEvents.push(
					Promise.resolve(
						emit({
							type: "tool_execution_update",
							toolCallId: prepared.toolCall.id,
							toolName: prepared.toolCall.name,
							args: prepared.toolCall.arguments,
							partialResult,
						}),
					),
				);
			},
		);
		await Promise.all(updateEvents);
		return { result, isError: false };
	} catch (error) {
		await Promise.all(updateEvents);
		return {
			result: createErrorToolResult(error instanceof Error ? error.message : String(error)),
			isError: true,
		};
	}
}

async function finalizeExecutedToolCall(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	prepared: PreparedToolCall,
	executed: ExecutedToolCallOutcome,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
): Promise<FinalizedToolCallOutcome> {
	let result = executed.result;
	let isError = executed.isError;

	if (config.afterToolCall) {
		try {
			const afterResult = await config.afterToolCall(
				{
					assistantMessage,
					toolCall: prepared.toolCall,
					args: prepared.args,
					result,
					isError,
					context: currentContext,
				},
				signal,
			);
			if (afterResult) {
				result = {
					content: afterResult.content ?? result.content,
					details: afterResult.details ?? result.details,
					terminate: afterResult.terminate ?? result.terminate,
				};
				isError = afterResult.isError ?? isError;
			}
		} catch (error) {
			result = createErrorToolResult(error instanceof Error ? error.message : String(error));
			isError = true;
		}
	}

	return {
		toolCall: prepared.toolCall,
		result,
		isError,
	};
}

function createErrorToolResult(message: string): AgentToolResult<any> {
	return {
		content: [{ type: "text", text: message }],
		details: {},
	};
}

async function emitToolExecutionEnd(finalized: FinalizedToolCallOutcome, emit: AgentEventSink): Promise<void> {
	await emit({
		type: "tool_execution_end",
		toolCallId: finalized.toolCall.id,
		toolName: finalized.toolCall.name,
		result: finalized.result,
		isError: finalized.isError,
	});
}

function createToolResultMessage(finalized: FinalizedToolCallOutcome): ToolResultMessage {
	return {
		role: "toolResult",
		toolCallId: finalized.toolCall.id,
		toolName: finalized.toolCall.name,
		content: finalized.result.content,
		details: finalized.result.details,
		isError: finalized.isError,
		timestamp: Date.now(),
	};
}

async function emitToolResultMessage(toolResultMessage: ToolResultMessage, emit: AgentEventSink): Promise<void> {
	await emit({ type: "message_start", message: toolResultMessage });
	await emit({ type: "message_end", message: toolResultMessage });
}
