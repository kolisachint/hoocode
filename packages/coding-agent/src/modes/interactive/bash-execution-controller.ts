/**
 * Bash command execution for the interactive mode (the `!cmd` prompt mode).
 *
 * Runs a bash command through the session (letting extensions intercept via
 * the user_bash event), renders a BashExecutionComponent for it, and streams
 * output into that component. Commands started while the agent is streaming are
 * parked in the pending area and moved into the chat transcript once the turn
 * finishes. Extracted from interactive-mode.ts behind a narrow
 * BashExecutionControllerDeps interface.
 */

import type { Container, TUI } from "@kolisachint/hoocode-tui";
import type { AgentSession } from "../../core/agent-session.js";
import type { TruncationResult } from "../../core/tools/truncate.js";
import { BashExecutionComponent } from "./components/bash-execution.js";

/** The slice of the interactive mode the bash execution needs. */
export interface BashExecutionControllerDeps {
	/** The active session (read at call time; the session can be swapped). */
	get session(): AgentSession;
	ui: TUI;
	/** Holds bash rows started while the agent is streaming. */
	pendingMessagesContainer: Container;
	/** The chat transcript. */
	chatContainer: Container;
	showError(errorMessage: string): void;
}

export class BashExecutionController {
	// Current bash execution component
	private bashComponent: BashExecutionComponent | undefined = undefined;

	// Pending bash components (shown in pending area, moved to chat on submit)
	private pendingBashComponents: BashExecutionComponent[] = [];

	constructor(private readonly deps: BashExecutionControllerDeps) {}

	private get session(): AgentSession {
		return this.deps.session;
	}

	/** Move pending bash components from pending area to chat */
	flushPendingBashComponents(): void {
		for (const component of this.pendingBashComponents) {
			this.deps.pendingMessagesContainer.removeChild(component);
			this.deps.chatContainer.addChild(component);
		}
		this.pendingBashComponents = [];
	}

	async handleBashCommand(command: string, excludeFromContext = false): Promise<void> {
		const extensionRunner = this.session.extensionRunner;

		// Emit user_bash event to let extensions intercept
		const eventResult = await extensionRunner.emitUserBash({
			type: "user_bash",
			command,
			excludeFromContext,
			cwd: this.session.sessionManager.getCwd(),
		});

		// If extension returned a full result, use it directly
		if (eventResult?.result) {
			const result = eventResult.result;

			// Create UI component for display
			this.bashComponent = new BashExecutionComponent(command, this.deps.ui, excludeFromContext);
			if (this.session.isStreaming) {
				this.deps.pendingMessagesContainer.addChild(this.bashComponent);
				this.pendingBashComponents.push(this.bashComponent);
			} else {
				this.deps.chatContainer.addChild(this.bashComponent);
			}

			// Show output and complete
			if (result.output) {
				this.bashComponent.appendOutput(result.output);
			}
			this.bashComponent.setComplete(
				result.exitCode,
				result.cancelled,
				result.truncated ? ({ truncated: true, content: result.output } as TruncationResult) : undefined,
				result.fullOutputPath,
			);

			// Record the result in session
			this.session.recordBashResult(command, result, { excludeFromContext });
			this.bashComponent = undefined;
			this.deps.ui.requestRender();
			return;
		}

		// Normal execution path (possibly with custom operations)
		const isDeferred = this.session.isStreaming;
		this.bashComponent = new BashExecutionComponent(command, this.deps.ui, excludeFromContext);

		if (isDeferred) {
			// Show in pending area when agent is streaming
			this.deps.pendingMessagesContainer.addChild(this.bashComponent);
			this.pendingBashComponents.push(this.bashComponent);
		} else {
			// Show in chat immediately when agent is idle
			this.deps.chatContainer.addChild(this.bashComponent);
		}
		this.deps.ui.requestRender();

		try {
			const result = await this.session.executeBash(
				command,
				(chunk) => {
					if (this.bashComponent) {
						this.bashComponent.appendOutput(chunk);
						this.deps.ui.requestRender();
					}
				},
				{ excludeFromContext, operations: eventResult?.operations },
			);

			if (this.bashComponent) {
				this.bashComponent.setComplete(
					result.exitCode,
					result.cancelled,
					result.truncated ? ({ truncated: true, content: result.output } as TruncationResult) : undefined,
					result.fullOutputPath,
				);
			}
		} catch (error) {
			if (this.bashComponent) {
				this.bashComponent.setComplete(undefined, false);
			}
			this.deps.showError(`Bash command failed: ${error instanceof Error ? error.message : "Unknown error"}`);
		}

		this.bashComponent = undefined;
		this.deps.ui.requestRender();
	}
}
