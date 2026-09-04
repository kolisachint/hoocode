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
	"app.thinking.cycleForward": true;
	"app.thinking.cycleBackward": true;
	"app.model.cycleForward": true;
	"app.model.cycleBackward": true;
	"app.model.select": true;
	"app.tools.expand": true;
	"app.view.cycleForward": true;
	"app.view.cycleBackward": true;
	"app.thinking.toggle": true;
	"app.tasks.cycleForward": true;
	"app.tasks.cycleBackward": true;
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
	"app.mode.cycleForward": true;
	"app.mode.cycleBackward": true;
	"app.options.next": true;
	"app.options.back": true;
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
 * The keyboard map.
 *
 * ## Why it is grouped the way it is
 *
 * There are around sixty bindings here. Nobody holds sixty of anything, and the
 * usual answer — sort them by mechanism, so everything that cycles sits together
 * — makes a list that is tidy on the page and useless at the keyboard. "It
 * cycles" is a fact about the widget. It is not what anyone is thinking when
 * they reach for the key.
 *
 * So the grouping is by **intention**: the five things a person is ever doing
 * here, in the order the loop runs.
 *
 *   1. **Flow** — get out, get back. esc, ctrl+c, ctrl+d, ctrl+z.
 *   2. **Compose** — the message in your hands. alt+e, alt+r, alt+enter, alt+↑.
 *   3. **Steer** — what the agent is before it runs. alt+a, alt+m, alt+t.
 *   4. **Read** — what you see of what it did. alt+o, alt+l, ctrl+o, ctrl+t.
 *   5. **Go** — sessions and places. alt+h, alt+w, alt+s, alt+k, alt+c.
 *
 * Five groups, none larger than five, which is the size a person can actually
 * hold. Two of them cost nothing to learn: **Flow** is the set every terminal
 * program already taught you, and the **overlays** (pickers, the tree, the
 * options pane) print their own keys on their own hint lines — recognised, never
 * recalled. That leaves three groups to genuinely know.
 *
 * The declaration order below *is* the grouping, and it is not cosmetic:
 * `orderKeybindingsConfig` writes `keybindings.json` in this order, so the file
 * a user opens to rebind something is grouped the same way this one is.
 *
 * ## What the chord tells you
 *
 * The modifier says what kind of thing will happen; the letter says to what.
 *
 * - **alt+<letter> sets a value.** Nothing takes the screen, nothing loses
 *   focus, you keep typing. Six of these are dials — an ordered set of stops
 *   with the current one painted where you can see it — and `shift+alt+<letter>`
 *   always steps back:
 *
 *       **a**gent mode · **m**odel · **t**hinking · tool **o**utput
 *       task **l**ist · session **c**olour
 *
 *   Reversibility is the point. A control you can undo invites you to try it; a
 *   one-way control makes you stop and think first, which is the wrong tax on a
 *   key you press all day.
 *
 * - **ctrl+<letter> acts on what is drawn right now**, and shares its letter
 *   with the alt key for the same subject. `alt+o` sets how much tool output
 *   there ever is, `ctrl+o` jumps to all of it and back. `alt+t` sets how much
 *   thinking there ever is, `ctrl+t` shows or hides what you have. Two subjects,
 *   two letters, four keys — half of what four unrelated chords would cost.
 *
 * - **A slash command chooses a stop outright.** `/mode`, `/model`, `/color`,
 *   `/tree`. The key steps, the command picks; that is why `app.model.select`
 *   and `app.session.tree` ship unbound, having given their letters to dials.
 *
 * One letter in the whole set names nothing: `alt+n` for the team roster. It
 * survives because the task panel prints it in its own header, so it is read off
 * the screen rather than remembered — which is the fallback for anything that
 * cannot earn a mnemonic.
 *
 * ## The constraints that shaped it
 *
 * - No binding takes a key the editor or the terminal already owns
 *   (ctrl+a/e/b/f/k/u/w/y/d/l/r/g, ctrl+s XOFF, ctrl+m == enter, ctrl+i == tab).
 * - Inside an overlay the query line is a text field, so every `ctrl+<letter>`
 *   there belongs to the *text*. Overlay verbs are all on `alt+<letter>`.
 * - Only `alt+<letter>` and `alt+<digit>` survive a terminal without the Kitty
 *   keyboard protocol; `alt+[` and `alt+]` arrive as the CSI and OSC introducers.
 * - `shift+<non-letter>` needs Kitty too, so a `shift+…` default is a
 *   convenience on top of a key that works everywhere. `shift+<letter>` is
 *   banned outright: without Kitty it *is* the uppercase letter, which no scope
 *   with a query line can tell from typing.
 * - Legacy `alt+p` and `alt+n` are also read as `alt+up` and `alt+down`, so no
 *   scope may bind both halves of either pair.
 *
 * `test/keybinding-layout.test.ts` holds all of it, including that every
 * binding belongs to exactly one family.
 */
export const KEYBINDINGS = {
	...TUI_KEYBINDINGS,

	// ── Flow — getting out, getting back ────────────────────────────────────
	// Every terminal program you already use binds these, so they cost nothing to
	// learn and must never move: whatever else is misconfigured, you can still
	// stop the agent, clear the line, and leave.
	"app.interrupt": { defaultKeys: "escape", description: "Cancel or abort" },
	"app.clear": { defaultKeys: "ctrl+c", description: "Clear editor" },
	"app.exit": { defaultKeys: "ctrl+d", description: "Exit when editor is empty" },
	"app.suspend": {
		defaultKeys: process.platform === "win32" ? [] : "ctrl+z",
		description: "Suspend to background",
	},

	// ── Compose — the message in your hands ─────────────────────────────────
	// Everything here acts on the text you are writing. Highest-frequency group,
	// and the one your hands are already on.
	// alt+e, not ctrl+g: ctrl+g is emacs/readline "abort" — the key you hit to
	// get out of something. Having it launch $EDITOR inverts that reflex.
	"app.editor.external": {
		defaultKeys: "alt+e",
		description: "Open external editor",
	},
	// alt+r, not ctrl+r: ctrl+r is reverse history search in every shell, and it
	// was also the session picker's rename key — one chord, two meanings.
	"app.input.voiceTranscribe": {
		defaultKeys: "alt+r",
		description: "Record voice and transcribe into the editor",
	},
	"app.clipboard.pasteImage": {
		defaultKeys: process.platform === "win32" ? "alt+v" : "ctrl+v",
		description: "Paste image from clipboard",
	},
	"app.message.followUp": {
		defaultKeys: "alt+enter",
		description: "Queue follow-up message",
	},
	"app.message.dequeue": {
		defaultKeys: "alt+up",
		description: "Restore queued messages",
	},

	// ── Steer — what the agent is before it runs ────────────────────────────
	// The only three keys that change what happens next, and the only three that
	// cost anything: latency, money, behaviour. The footer shows all three, which
	// is why they are worth one deliberate chunk of memory.
	// alt+a for "agent mode", and it is the dial the footer leads with. Not alt+m:
	// mode and model are one letter apart and the model has the better claim on
	// m, so this one takes the letter of what it selects — the agent's stance.
	"app.mode.cycleForward": {
		defaultKeys: "alt+a",
		description: "Cycle agent mode (ask → plan → build → debug)",
	},
	"app.mode.cycleBackward": {
		defaultKeys: "shift+alt+a",
		description: "Cycle agent mode backward",
	},
	// alt+m, not ctrl+p: the model is a cockpit dial — it is what the agent *is*,
	// not what is on screen — and p named nothing. The letter names the dial now,
	// which is the whole rule for the six of them. `/model` opens the picker that
	// used to be on this key, exactly as `/color` backs alt+c.
	"app.model.cycleForward": {
		defaultKeys: "alt+m",
		description: "Cycle to next model",
	},
	"app.model.cycleBackward": {
		defaultKeys: "shift+alt+m",
		description: "Cycle to previous model",
	},
	// Unbound by default: a dial's key steps it, and its slash command chooses.
	// alt+m stepping the model is worth more than alt+m opening a list of them,
	// and `/model` is one keystroke further with completion on the name.
	"app.model.select": { defaultKeys: [], description: "Open model selector" },
	// alt+t pairs with ctrl+t the way alt+o pairs with ctrl+o: same letter, same
	// subject, ctrl acting on what is drawn right now and alt on how much there
	// ever is. shift+tab stays as a second key rather than the first — it is the
	// cycle key every terminal agent has taught, and with no slash command for
	// the thinking level it is the only way to reach this dial on a terminal
	// that eats alt. Hints and /hotkeys name alt+t, which is the taught key.
	"app.thinking.cycleForward": {
		defaultKeys: ["alt+t", "shift+tab"],
		description: "Cycle thinking level (off → … → high)",
	},
	"app.thinking.cycleBackward": {
		defaultKeys: "shift+alt+t",
		description: "Cycle thinking level backward",
	},

	// ── Read — what you see of what it did ──────────────────────────────────
	// Free and reversible, every one of them: nothing here touches the work, only
	// the window onto it. Press again or add shift and you are back where you
	// were, which is what makes poking at them safe.
	// The view dial pairs with ctrl+o on purpose: same letter, different ring.
	// alt+o walks the dial a stop at a time and saves where it lands; ctrl+o
	// jumps to the far end and back without moving your home stop. One value
	// between them, so there is no second "expanded" state to keep in sync.
	"app.view.cycleForward": {
		defaultKeys: "alt+o",
		description: "Cycle tool output view (radar → peek → full)",
	},
	"app.view.cycleBackward": {
		defaultKeys: "shift+alt+o",
		description: "Cycle tool output view backward",
	},
	"app.tools.expand": {
		defaultKeys: "ctrl+o",
		description: "Jump to the full view from wherever you are, and back again",
	},
	"app.thinking.toggle": {
		defaultKeys: "ctrl+t",
		description: "Toggle thinking blocks",
	},
	// alt+l for the task *ledger* — the pane's own name for itself. It was
	// ctrl+n, which named nothing and was the last dial off the alt ring; moving
	// it also gives the lens the reverse it could never have on ctrl, where
	// shift+ctrl+n is Windows Terminal's "new window". ctrl+n is free now, so an
	// emacs config can take it for cursorDown without colliding.
	"app.tasks.cycleForward": {
		defaultKeys: "alt+l",
		description: "Cycle task panel view (tasks → subagents → teams, skips empty lenses)",
	},
	"app.tasks.cycleBackward": {
		defaultKeys: "shift+alt+l",
		description: "Cycle task panel view backward",
	},
	// The one letter in the set that names nothing. It survives on the fallback
	// every unmemorable key needs: the task panel prints it in its own header
	// when the teams lens is up, so it is read off the screen rather than
	// remembered. alt+n steps INTO that lens and focuses the roster (--team only).
	"app.team.focus": {
		defaultKeys: "alt+n",
		description: "Focus the team roster (navigate roles, n nudge, a attach)",
	},

	// ── Go — sessions and places ────────────────────────────────────────────
	// These take the screen and hand it back on escape. Each has a slash command
	// that does the same thing, so none of them has to be remembered as a key.
	"app.session.resume": { defaultKeys: "alt+h", description: "Resume a session from history" },
	// Unbound since alt+t became the thinking dial: the letter is worth more to a
	// dial pressed through the day than to a surface `/tree` opens with
	// completion, the same trade `app.model.select` makes for `/model`.
	"app.session.tree": { defaultKeys: [], description: "Open session tree" },
	// The two destructive-ish session moves stay unbound by default. /new
	// replaces the transcript and /fork needs a message picked out of it, so
	// neither wants to be one stray chord away; both remain bindable by hand.
	"app.session.new": { defaultKeys: [], description: "Start a new session" },
	"app.session.fork": { defaultKeys: [], description: "Fork current session" },
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

	// ── Overlays — live only while their surface is open ────────────────────
	// Never a memory burden: each surface prints its own keys on its own hint
	// line, so these are recognised, not recalled. They may reuse a global
	// letter, because the surface captures keys while it is up.
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
	// The options pane reads as a horizontal wizard, so the arrows point the way
	// the steps run: → commits the highlighted answer and moves on, ← goes back.
	// On the free-text row they only mean that while the field is empty —
	// otherwise they move the text cursor, and enter is what commits.
	"app.options.next": { defaultKeys: "right", description: "Confirm and advance to the next question" },
	"app.options.back": { defaultKeys: "left", description: "Go back to the previous question" },
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
	"app.session.toggleNamedFilter": {
		defaultKeys: "alt+n",
		description: "Toggle named session filter",
	},
	"app.session.rename": {
		defaultKeys: "alt+r",
		description: "Rename session",
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
	"app.tree.foldOrUp": {
		defaultKeys: ["ctrl+left", "alt+left"],
		description: "Fold tree branch or move up",
	},
	"app.tree.unfoldOrDown": {
		defaultKeys: ["ctrl+right", "alt+right"],
		description: "Unfold tree branch or move down",
	},
	// alt+l ("label") and alt+t ("time"), not shift+l / shift+t. The tree has a
	// search query that takes every printable key, and outside the Kitty protocol
	// shift+<letter> *is* the plain uppercase letter — so typing "TODO" or
	// "Logger" into the query opened the label editor instead of searching. Same
	// rule as every other picker verb: the query owns the letters, the verbs take
	// alt. Global alt+t opens the tree; inside the tree it toggles timestamps,
	// which is unambiguous because the tree captures keys while it is open.
	"app.tree.editLabel": {
		defaultKeys: "alt+l",
		description: "Edit tree label",
	},
	"app.tree.toggleLabelTimestamp": {
		defaultKeys: "alt+t",
		description: "Toggle tree label timestamps",
	},
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
	// Renamed when each gained a backward half, so that every two-direction dial
	// reads <dial>.cycleForward / <dial>.cycleBackward.
	"app.thinking.cycle": "app.thinking.cycleForward",
	"app.mode.cycle": "app.mode.cycleForward",
	"app.tasks.cycleView": "app.tasks.cycleForward",
	interrupt: "app.interrupt",
	clear: "app.clear",
	exit: "app.exit",
	suspend: "app.suspend",
	cycleThinkingLevel: "app.thinking.cycleForward",
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
