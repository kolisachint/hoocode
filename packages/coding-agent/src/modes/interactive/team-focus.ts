/**
 * hoocode team focus (--team): focus role rows, nudge, attach.
 *
 * Wires a hooteams connection into the TUI: team focus (app.team.focus),
 * nudging (n), the attach side panel (a) on the task panel's teams view, and
 * approval gates. task_paused events (and gates already pending on the server)
 * surface inline in the attach panel when it shows the paused role, otherwise
 * in the options pane; the answer goes back over POST /tasks/:id/resume.
 */

import type { Component, Container, OverlayHandle, TUI } from "@kolisachint/hoocode-tui";
import type { AskQuestion, ExtensionUIDialogOptions } from "../../core/extensions/index.js";
import { taskStore } from "../../core/task-store.js";
import { type TeamApproval, TeamApprovalCoordinator } from "../../core/team-approvals.js";
import type { TeamViewConnection } from "../../core/team-view.js";
import { ExtensionInputComponent } from "./components/extension-input.js";
import type { TaskPanelComponent } from "./components/task-panel.js";
import { TeamAttachPanelComponent } from "./components/team-attach-panel.js";

/** The slice of the interactive mode the team feature needs. */
export interface TeamFocusDeps {
	ui: TUI;
	taskPanel: TaskPanelComponent;
	editorContainer: Container;
	/** The prompt editor (read at call time; the editor can be swapped). */
	getEditor(): Component;
	showStatus(message: string): void;
	showWarning(message: string): void;
	showAskOptions(questions: AskQuestion[], opts?: ExtensionUIDialogOptions): Promise<string[] | undefined>;
	/** True while another ask-options pane is on screen. */
	isAskOptionsOpen(): boolean;
}

export class TeamFocusController {
	private client: TeamViewConnection | undefined = undefined;
	private attachPanel: TeamAttachPanelComponent | undefined = undefined;
	private attachHandle: OverlayHandle | undefined = undefined;

	constructor(private readonly deps: TeamFocusDeps) {}

	/** True once a team client has been attached (--team). */
	get connected(): boolean {
		return this.client !== undefined;
	}

	/**
	 * Wire a hooteams connection into the TUI. Called by main.ts when `--team`
	 * is set, before run().
	 */
	attachClient(client: TeamViewConnection): void {
		this.client = client;
		const approvals = new TeamApprovalCoordinator({
			present: (approval, signal) => this.presentApproval(approval, signal),
			resume: (taskId, option) => client.resume(taskId, option),
			info: (message) => this.deps.showStatus(message),
			warn: (message) => this.deps.showWarning(message),
		});
		client.subscribe((event) => approvals.handleEvent(event));
		// Gates that opened before we attached don't replay as task_paused.
		void client.pendingApprovals().then(
			(pending) => {
				for (const gate of pending) approvals.enqueuePending(gate);
			},
			() => {
				// Best-effort like the rest of the bridge; live gates still arrive via SSE.
			},
		);
	}

	/**
	 * Show one team approval gate and resolve with the chosen (or free-form)
	 * answer, undefined when skipped. When the attach side panel is open on the
	 * role that paused, the gate renders inline in the panel — right where its
	 * stream stopped; otherwise it goes to the options pane, waiting politely
	 * while another ask is on screen. Either way the signal (gate answered
	 * elsewhere) dismisses the prompt.
	 */
	private async presentApproval(approval: TeamApproval, signal: AbortSignal): Promise<string | undefined> {
		const panel = this.attachPanel;
		if (panel && approval.role === panel.role) {
			this.attachHandle?.focus();
			const answer = await panel.presentApproval(approval, signal);
			// Detaching mid-gate settles as skipped — fall through to the options
			// pane so the question isn't silently lost. A skip with the panel
			// still open is a real skip.
			if (answer !== undefined || signal.aborted || this.attachPanel === panel) return answer;
		}
		while (this.deps.isAskOptionsOpen() && !signal.aborted) {
			await new Promise((resolve) => setTimeout(resolve, 200));
		}
		if (signal.aborted) return undefined;
		const answers = await this.deps.showAskOptions(
			[
				{
					question: approval.question,
					short: approval.taskId,
					detail: `team task "${approval.taskId}"${approval.role ? ` (${approval.role})` : ""} is paused until answered`,
					options: approval.options.map((label) => ({ label })),
					allowCustom: true,
				},
			],
			{ signal },
		);
		return answers?.[0];
	}

	/** Move keyboard focus to the task panel's team roster. */
	enterFocus(): void {
		if (!this.client) {
			this.deps.showStatus("No team connected. Start with --team <url> to mirror a hooteams server.");
			return;
		}
		if (!taskStore.agents().some((a) => a.kind === "role")) {
			this.deps.showStatus("Team roster is empty — waiting for roles from the team server.");
			return;
		}
		this.deps.taskPanel.setView("teams");
		this.deps.ui.setFocus(this.deps.taskPanel);
		this.deps.ui.requestRender();
	}

	/** Leave team focus: detach any side panel and return focus to the editor. */
	exitFocus(): void {
		this.closeAttach();
		this.deps.ui.setFocus(this.deps.getEditor());
		this.deps.ui.requestRender();
	}

	/**
	 * Inline nudge editor for a role (n in team focus or while attached).
	 * Swaps the prompt editor for a one-line input; submit fires POST /steer in
	 * the background (the REPL and team focus are never blocked on the network).
	 */
	showNudgeInput(role: string): void {
		const client = this.client;
		if (!client) return;

		const restoreFocus = () => {
			this.deps.editorContainer.clear();
			this.deps.editorContainer.addChild(this.deps.getEditor());
			// Return focus where the nudge came from: the attach panel if one is
			// open, otherwise the team roster.
			if (this.attachHandle) this.attachHandle.focus();
			else this.deps.ui.setFocus(this.deps.taskPanel);
			this.deps.ui.requestRender();
		};

		const input = new ExtensionInputComponent(
			`Nudge ${role}`,
			undefined,
			(value) => {
				input.dispose();
				restoreFocus();
				const message = value.trim();
				if (!message) return;
				void client.steer(role, message).then(
					() => this.deps.showStatus(`Nudged ${role}`),
					(error) => this.deps.showWarning(`Failed to nudge ${role}: ${String(error)}`),
				);
			},
			() => {
				input.dispose();
				restoreFocus();
			},
			{ tui: this.deps.ui },
		);

		this.deps.editorContainer.clear();
		this.deps.editorContainer.addChild(input);
		this.deps.ui.setFocus(input);
		this.deps.ui.requestRender();
	}

	/** Open the attach side panel for a role (a in team focus). */
	showAttach(role: string): void {
		const client = this.client;
		if (!client) return;
		// One attached role at a time: re-attaching swaps the panel.
		this.closeAttach();
		const panel = new TeamAttachPanelComponent(
			role,
			client,
			{
				onDetach: () => this.closeAttach(),
				onNudge: (attachedRole) => this.showNudgeInput(attachedRole),
			},
			this.deps.ui,
		);
		this.attachPanel = panel;
		// preFocus is the task panel (attach is triggered from team focus), so
		// hiding the overlay drops the user back on the role roster.
		this.attachHandle = this.deps.ui.showOverlay(panel, {
			anchor: "top-right",
			width: "45%",
			minWidth: 36,
			margin: { top: 1, right: 1 },
		});
	}

	/** Detach the side panel; the role keeps running. Safe to call when closed. */
	closeAttach(): void {
		this.attachPanel?.dispose();
		this.attachHandle?.hide();
		this.attachPanel = undefined;
		this.attachHandle = undefined;
	}
}
