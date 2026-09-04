import {
	type Keybinding,
	type KeybindingDefinitions,
	type KeybindingsConfig,
	type KeyId,
	TUI_KEYBINDINGS,
	KeybindingsManager as TuiKeybindingsManager,
} from "@kolisachint/hoocode-tui";
import { existsSync, readFileSync } from "fs";
import { join } from "path";
import { getAgentDir } from "../config.js";

interface AppKeybindings {
	"app.interrupt": true;
	"app.clear": true;
	"app.exit": true;
	"app.suspend": true;
	"app.thinking.cycle": true;
	"app.model.cycleForward": true;
	"app.model.cycleBackward": true;
	"app.model.select": true;
	"app.tools.expand": true;
	"app.view.cycleForward": true;
	"app.view.cycleBackward": true;
	"app.tools.unfoldOne": true;
	"app.tools.foldOne": true;
	"app.thinking.toggle": true;
	"app.tasks.cycleView": true;
	"app.team.focus": true;
	"app.team.nudge": true;
	"app.team.attach": true;
	"app.session.toggleNamedFilter": true;
	"app.editor.external": true;
	"app.message.followUp": true;
	"app.message.dequeue": true;
	"app.clipboard.pasteImage": true;
	"app.input.voiceTranscribe": true;
	"app.session.new": true;
	"app.session.tree": true;
	"app.session.fork": true;
	"app.session.resume": true;
	"app.session.changeDirectory": true;
	"app.session.color.cycleForward": true;
	"app.session.color.cycleBackward": true;
	"app.settings.open": true;
	"app.hotkeys.open": true;
	"app.mode.cycle": true;
	"app.tree.foldOrUp": true;
	"app.tree.unfoldOrDown": true;
	"app.tree.editLabel": true;
	"app.tree.toggleLabelTimestamp": true;
	"app.session.togglePath": true;
	"app.session.toggleSort": true;
	"app.session.rename": true;
	"app.session.delete": true;
	"app.session.deleteNoninvasive": true;
	"app.models.save": true;
	"app.models.enableAll": true;
	"app.models.clearAll": true;
	"app.models.toggleProvider": true;
	"app.models.reorderUp": true;
	"app.models.reorderDown": true;
	"app.tree.filter.default": true;
	"app.tree.filter.noTools": true;
	"app.tree.filter.userOnly": true;
	"app.tree.filter.labeledOnly": true;
	"app.tree.filter.all": true;
	"app.tree.filter.cycleForward": true;
	"app.tree.filter.cycleBackward": true;
}

export type AppKeybinding = keyof AppKeybindings;

declare module "@kolisachint/hoocode-tui" {
	interface Keybindings extends AppKeybindings {}
}

/**
 * The cockpit layout.
 *
 * Three rings, and which ring a key belongs to is decided by its modifier:
 *
 * - **ctrl — the view.** What is on screen right now: expand, thinking blocks,
 *   task panel, model cycling. Pressed many times a minute, so they sit under
 *   the hand and never require a second modifier.
 * - **alt — the cockpit.** What the agent *is* and where it works: model,
 *   mode, working directory, settings, sessions. Pressed a few times a session.
 * - **overlays.** Inside a picker, the query line is a text field, so every
 *   `ctrl+<letter>` there belongs to the *text* (ctrl+a start of line, ctrl+u
 *   kill line, ctrl+w kill word). A picker's own verbs are therefore all on
 *   `alt+<letter>`, mnemonic to that picker; the picker captures keys while it
 *   is open, so reusing a global letter there is unambiguous.
 *
 * Two rules keep the set learnable: shift reverses whatever the unshifted key
 * does, and no binding takes a key the editor or the terminal already owns
 * (ctrl+a/e/b/f/k/u/w/y/d/l/r/g, ctrl+s XOFF, ctrl+m == enter, ctrl+i == tab).
 *
 * Two constraints from the key parser shape which alt keys are usable at all,
 * and `test/keybinding-layout.test.ts` holds both:
 *
 * - Only `alt+<letter>` and `alt+<digit>` survive a terminal without the Kitty
 *   keyboard protocol. `alt+[` and `alt+]` arrive as the CSI and OSC
 *   introducers and are not parsed as keys; nor is any shift-modified key, so a
 *   `shift+…` default is a Kitty-only convenience on top of an unshifted key
 *   that works everywhere (as `shift+ctrl+p` has always been).
 * - Legacy `alt+p` and `alt+n` are also accepted as `alt+up` and `alt+down`
 *   (the emacs previous/next aliases), so no scope may bind both halves of
 *   either pair.
 */
export const KEYBINDINGS = {
	...TUI_KEYBINDINGS,
	"app.interrupt": { defaultKeys: "escape", description: "Cancel or abort" },
	"app.clear": { defaultKeys: "ctrl+c", description: "Clear editor" },
	"app.exit": { defaultKeys: "ctrl+d", description: "Exit when editor is empty" },
	"app.suspend": {
		defaultKeys: process.platform === "win32" ? [] : "ctrl+z",
		description: "Suspend to background",
	},
	"app.thinking.cycle": {
		defaultKeys: "shift+tab",
		description: "Cycle thinking level",
	},
	"app.model.cycleForward": {
		defaultKeys: "ctrl+p",
		description: "Cycle to next model",
	},
	"app.model.cycleBackward": {
		defaultKeys: "shift+ctrl+p",
		description: "Cycle to previous model",
	},
	// alt+m, not ctrl+l: ctrl+l is "clear the screen" in every shell and every
	// full-screen terminal app, and users press it reflexively to tidy up. It
	// opening a model dialog is the single most surprising key in the old set.
	"app.model.select": { defaultKeys: "alt+m", description: "Open model selector" },
	"app.tools.expand": {
		defaultKeys: "ctrl+o",
		description: "Expand or collapse what is in front of you (tool bodies, header, summaries)",
	},
	// The view dial pairs with ctrl+o on purpose: same letter, different ring.
	// ctrl+o opens what is already there; alt+o decides how much is ever there.
	"app.view.cycleForward": {
		defaultKeys: "alt+o",
		description: "Cycle tool output view (radar → glance → full)",
	},
	"app.view.cycleBackward": {
		defaultKeys: "shift+alt+o",
		description: "Cycle tool output view backward",
	},
	// The one-at-a-time counterpart to ctrl+o's all-or-nothing. The transcript is
	// bottom-anchored with no app-level scrolling — anything far enough up is in
	// the terminal's own scrollback, which this process cannot address — so "open
	// one" can only ever mean "open one at the tail". Repeating the key peels
	// backwards from there.
	"app.tools.unfoldOne": {
		defaultKeys: "alt+u",
		description: "Open the newest folded thing — a chain in radar, a tool body otherwise",
	},
	"app.tools.foldOne": {
		defaultKeys: "shift+alt+u",
		description: "Re-fold the most recently opened chain or tool block",
	},
	"app.thinking.toggle": {
		defaultKeys: "ctrl+t",
		description: "Toggle thinking blocks",
	},
	"app.tasks.cycleView": {
		// ctrl+n is free in the main editor (no emacs next-line binding here).
		defaultKeys: "ctrl+n",
		description: "Cycle task panel view (tasks → subagents → teams, skips empty lenses)",
	},
	"app.team.focus": {
		// Pairs with ctrl+n (cycle task panel view): alt+n steps INTO the teams
		// lens and focuses the role roster (--team only). Not shift+ctrl+n —
		// Windows Terminal intercepts that as its "new window" shortcut, the
		// same trap that moved app.tasks.cycleView off ctrl+shift+t.
		defaultKeys: "alt+n",
		description: "Focus the team roster (navigate roles, n nudge, a attach)",
	},
	"app.team.nudge": {
		// Plain letters are safe here: these fire only while the task panel holds
		// focus (team-focus mode), never while typing in the editor.
		defaultKeys: "n",
		description: "Nudge the selected team role (team focus mode)",
	},
	"app.team.attach": {
		defaultKeys: "a",
		description: "Attach to the selected team role (team focus mode)",
	},
	// alt+e, not ctrl+g: ctrl+g is emacs/readline "abort" — the key you hit to
	// get out of something. Having it launch $EDITOR inverts that reflex.
	"app.editor.external": {
		defaultKeys: "alt+e",
		description: "Open external editor",
	},
	"app.message.followUp": {
		defaultKeys: "alt+enter",
		description: "Queue follow-up message",
	},
	"app.message.dequeue": {
		defaultKeys: "alt+up",
		description: "Restore queued messages",
	},
	"app.clipboard.pasteImage": {
		defaultKeys: process.platform === "win32" ? "alt+v" : "ctrl+v",
		description: "Paste image from clipboard",
	},
	// alt+r, not ctrl+r: ctrl+r is reverse history search in every shell, and it
	// was also the session picker's rename key — one chord, two meanings.
	"app.input.voiceTranscribe": {
		defaultKeys: "alt+r",
		description: "Record voice and transcribe into the editor",
	},
	// The two destructive-ish session moves stay unbound by default. /new
	// replaces the transcript and /fork needs a message picked out of it, so
	// neither wants to be one stray chord away; both remain bindable by hand.
	"app.session.new": { defaultKeys: [], description: "Start a new session" },
	"app.session.fork": { defaultKeys: [], description: "Fork current session" },
	// Navigation is non-destructive, so it gets keys: t for tree, h for history.
	"app.session.tree": { defaultKeys: "alt+t", description: "Open session tree" },
	"app.session.resume": { defaultKeys: "alt+h", description: "Resume a session from history" },
	"app.session.changeDirectory": {
		defaultKeys: "alt+w",
		description: "Change working directory (move to another repo without quitting)",
	},
	// alt+c ("colour"), and the tree's filter cycle has the same letter on
	// purpose: the tree captures keys while it is open, so the two are never live
	// at once, and both mean "step to the next one" wherever you press them. The
	// backward half needs Kitty, like every other shift-modified default here.
	"app.session.color.cycleForward": {
		defaultKeys: "alt+c",
		description: "Cycle the session chip's color",
	},
	"app.session.color.cycleBackward": {
		defaultKeys: "shift+alt+c",
		description: "Cycle the session chip's color backward",
	},
	"app.settings.open": { defaultKeys: "alt+s", description: "Open settings" },
	"app.hotkeys.open": { defaultKeys: "alt+k", description: "Show keyboard shortcuts" },
	"app.mode.cycle": {
		defaultKeys: "alt+g",
		description: "Cycle agent mode (ask → plan → build → debug)",
	},
	"app.tree.foldOrUp": {
		defaultKeys: ["ctrl+left", "alt+left"],
		description: "Fold tree branch or move up",
	},
	"app.tree.unfoldOrDown": {
		defaultKeys: ["ctrl+right", "alt+right"],
		description: "Unfold tree branch or move down",
	},
	"app.tree.editLabel": {
		defaultKeys: "shift+l",
		description: "Edit tree label",
	},
	"app.tree.toggleLabelTimestamp": {
		defaultKeys: "shift+t",
		description: "Toggle tree label timestamps",
	},
	// Session picker verbs. All on alt so the picker's query line keeps the
	// emacs editing keys it used to lose: ctrl+a jumped to the start of the
	// query *and* enabled every model, ctrl+d deleted a character *and* deleted
	// the highlighted session.
	"app.session.togglePath": {
		defaultKeys: "alt+p",
		description: "Toggle session path display",
	},
	"app.session.toggleSort": {
		defaultKeys: "alt+o",
		description: "Toggle session sort order",
	},
	"app.session.rename": {
		defaultKeys: "alt+r",
		description: "Rename session",
	},
	"app.session.toggleNamedFilter": {
		defaultKeys: "alt+n",
		description: "Toggle named session filter",
	},
	"app.session.delete": {
		defaultKeys: "alt+x",
		description: "Delete session",
	},
	"app.session.deleteNoninvasive": {
		defaultKeys: "ctrl+backspace",
		description: "Delete session when query is empty",
	},
	// Model picker verbs, same rule. ctrl+s in particular was XOFF: on a terminal
	// with flow control still on, "save" froze the session instead.
	"app.models.save": {
		defaultKeys: "alt+s",
		description: "Save model selection",
	},
	"app.models.enableAll": {
		defaultKeys: "alt+a",
		description: "Enable all models",
	},
	"app.models.clearAll": {
		defaultKeys: "alt+x",
		description: "Clear all models",
	},
	// alt+g ("group"), not alt+p: on a terminal without the Kitty protocol, alt+p
	// arrives as ESC-p, which the key parser also accepts as alt+up — and alt+up
	// is this same picker's reorder key.
	"app.models.toggleProvider": {
		defaultKeys: "alt+g",
		description: "Toggle all models for provider",
	},
	"app.models.reorderUp": {
		defaultKeys: "alt+up",
		description: "Move model up in order",
	},
	"app.models.reorderDown": {
		defaultKeys: "alt+down",
		description: "Move model down in order",
	},
	"app.options.next": { defaultKeys: "right", description: "Confirm and advance to the next question" },
	"app.options.back": { defaultKeys: "left", description: "Go back to the previous question" },
	// Five lenses in a fixed order, so they are numbered rather than lettered:
	// alt+1..alt+5 needs no mnemonic and collides with nothing. The old set
	// (ctrl+d/t/u/l/a) collided with delete-char, thinking, kill-to-start,
	// clear-screen and start-of-line respectively.
	"app.tree.filter.default": {
		defaultKeys: "alt+1",
		description: "Tree filter: default view",
	},
	"app.tree.filter.noTools": {
		defaultKeys: "alt+2",
		description: "Tree filter: hide tool results",
	},
	"app.tree.filter.userOnly": {
		defaultKeys: "alt+3",
		description: "Tree filter: user messages only",
	},
	"app.tree.filter.labeledOnly": {
		defaultKeys: "alt+4",
		description: "Tree filter: labeled entries only",
	},
	"app.tree.filter.all": {
		defaultKeys: "alt+5",
		description: "Tree filter: show all entries",
	},
	// alt+c for "cycle". Not a bracket pair, tempting as `alt+[` / `alt+]` are:
	// without the Kitty protocol those arrive as ESC-[ and ESC-], which are the
	// CSI and OSC introducers and so are not parsed as keys at all. The backward
	// half needs Kitty, like every other shift-modified default here.
	"app.tree.filter.cycleForward": {
		defaultKeys: "alt+c",
		description: "Tree filter: cycle forward",
	},
	"app.tree.filter.cycleBackward": {
		defaultKeys: "shift+alt+c",
		description: "Tree filter: cycle backward",
	},
} as const satisfies KeybindingDefinitions;

const KEYBINDING_NAME_MIGRATIONS = {
	cursorUp: "tui.editor.cursorUp",
	cursorDown: "tui.editor.cursorDown",
	cursorLeft: "tui.editor.cursorLeft",
	cursorRight: "tui.editor.cursorRight",
	cursorWordLeft: "tui.editor.cursorWordLeft",
	cursorWordRight: "tui.editor.cursorWordRight",
	cursorLineStart: "tui.editor.cursorLineStart",
	cursorLineEnd: "tui.editor.cursorLineEnd",
	jumpForward: "tui.editor.jumpForward",
	jumpBackward: "tui.editor.jumpBackward",
	pageUp: "tui.editor.pageUp",
	pageDown: "tui.editor.pageDown",
	deleteCharBackward: "tui.editor.deleteCharBackward",
	deleteCharForward: "tui.editor.deleteCharForward",
	deleteWordBackward: "tui.editor.deleteWordBackward",
	deleteWordForward: "tui.editor.deleteWordForward",
	deleteToLineStart: "tui.editor.deleteToLineStart",
	deleteToLineEnd: "tui.editor.deleteToLineEnd",
	yank: "tui.editor.yank",
	yankPop: "tui.editor.yankPop",
	undo: "tui.editor.undo",
	newLine: "tui.input.newLine",
	submit: "tui.input.submit",
	tab: "tui.input.tab",
	copy: "tui.input.copy",
	selectUp: "tui.select.up",
	selectDown: "tui.select.down",
	selectPageUp: "tui.select.pageUp",
	selectPageDown: "tui.select.pageDown",
	selectConfirm: "tui.select.confirm",
	selectCancel: "tui.select.cancel",
	interrupt: "app.interrupt",
	clear: "app.clear",
	exit: "app.exit",
	suspend: "app.suspend",
	cycleThinkingLevel: "app.thinking.cycle",
	cycleModelForward: "app.model.cycleForward",
	cycleModelBackward: "app.model.cycleBackward",
	selectModel: "app.model.select",
	expandTools: "app.tools.expand",
	toggleThinking: "app.thinking.toggle",
	toggleSessionNamedFilter: "app.session.toggleNamedFilter",
	externalEditor: "app.editor.external",
	followUp: "app.message.followUp",
	dequeue: "app.message.dequeue",
	pasteImage: "app.clipboard.pasteImage",
	newSession: "app.session.new",
	tree: "app.session.tree",
	fork: "app.session.fork",
	resume: "app.session.resume",
	treeFoldOrUp: "app.tree.foldOrUp",
	treeUnfoldOrDown: "app.tree.unfoldOrDown",
	treeEditLabel: "app.tree.editLabel",
	treeToggleLabelTimestamp: "app.tree.toggleLabelTimestamp",
	toggleSessionPath: "app.session.togglePath",
	toggleSessionSort: "app.session.toggleSort",
	renameSession: "app.session.rename",
	deleteSession: "app.session.delete",
	deleteSessionNoninvasive: "app.session.deleteNoninvasive",
} as const satisfies Record<string, Keybinding>;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isLegacyKeybindingName(key: string): key is keyof typeof KEYBINDING_NAME_MIGRATIONS {
	return key in KEYBINDING_NAME_MIGRATIONS;
}

function toKeybindingsConfig(value: unknown): KeybindingsConfig {
	if (!isRecord(value)) return {};

	const config: KeybindingsConfig = {};
	for (const [key, binding] of Object.entries(value)) {
		if (typeof binding === "string") {
			config[key] = binding as KeyId;
			continue;
		}
		if (Array.isArray(binding) && binding.every((entry) => typeof entry === "string")) {
			config[key] = binding as KeyId[];
		}
	}
	return config;
}

export function migrateKeybindingsConfig(rawConfig: Record<string, unknown>): {
	config: Record<string, unknown>;
	migrated: boolean;
} {
	const config: Record<string, unknown> = {};
	let migrated = false;

	for (const [key, value] of Object.entries(rawConfig)) {
		const nextKey = isLegacyKeybindingName(key) ? KEYBINDING_NAME_MIGRATIONS[key] : key;
		if (nextKey !== key) {
			migrated = true;
		}
		if (key !== nextKey && Object.hasOwn(rawConfig, nextKey)) {
			migrated = true;
			continue;
		}
		config[nextKey] = value;
	}

	return { config: orderKeybindingsConfig(config), migrated };
}

function orderKeybindingsConfig(config: Record<string, unknown>): Record<string, unknown> {
	const ordered: Record<string, unknown> = {};
	for (const keybinding of Object.keys(KEYBINDINGS)) {
		if (Object.hasOwn(config, keybinding)) {
			ordered[keybinding] = config[keybinding];
		}
	}

	const extras = Object.keys(config)
		.filter((key) => !Object.hasOwn(ordered, key))
		.sort();
	for (const key of extras) {
		ordered[key] = config[key];
	}

	return ordered;
}

function loadRawConfig(path: string): Record<string, unknown> | undefined {
	if (!existsSync(path)) return undefined;
	try {
		const parsed = JSON.parse(readFileSync(path, "utf-8")) as unknown;
		return isRecord(parsed) ? parsed : undefined;
	} catch {
		return undefined;
	}
}

export class KeybindingsManager extends TuiKeybindingsManager {
	private configPath: string | undefined;

	constructor(userBindings: KeybindingsConfig = {}, configPath?: string) {
		super(KEYBINDINGS, userBindings);
		this.configPath = configPath;
	}

	static create(agentDir: string = getAgentDir()): KeybindingsManager {
		const configPath = join(agentDir, "keybindings.json");
		const userBindings = KeybindingsManager.loadFromFile(configPath);
		return new KeybindingsManager(userBindings, configPath);
	}

	reload(): void {
		if (!this.configPath) return;
		this.setUserBindings(KeybindingsManager.loadFromFile(this.configPath));
	}

	getEffectiveConfig(): KeybindingsConfig {
		return this.getResolvedBindings();
	}

	private static loadFromFile(path: string): KeybindingsConfig {
		const rawConfig = loadRawConfig(path);
		if (!rawConfig) return {};
		return toKeybindingsConfig(migrateKeybindingsConfig(rawConfig).config);
	}
}

export type { KeyId, KeybindingsConfig };
