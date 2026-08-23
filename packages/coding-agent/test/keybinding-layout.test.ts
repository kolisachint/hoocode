import { describe, expect, it } from "vitest";
import { KEYBINDINGS, KeybindingsManager } from "../src/core/keybindings.js";

/**
 * Guards the cockpit layout described in `core/keybindings.ts`.
 *
 * Keybindings are grouped by the scope they are live in. Two ids may share a
 * key only when their scopes never accept input at the same time — a picker
 * captures keys while it is open, so its verbs may reuse a global letter. Two
 * ids in the *same* scope sharing a key is a bug: whichever handler is
 * registered first wins silently, and `/hotkeys` then documents a lie.
 *
 * Update these lists when a binding is added, moved between scopes, or removed.
 */

/** Live while the prompt editor has focus — the main editor plus every global action. */
const GLOBAL_SCOPE = [
	// tui editor + input, all live in the prompt
	"tui.editor.cursorUp",
	"tui.editor.cursorDown",
	"tui.editor.cursorLeft",
	"tui.editor.cursorRight",
	"tui.editor.cursorWordLeft",
	"tui.editor.cursorWordRight",
	"tui.editor.cursorLineStart",
	"tui.editor.cursorLineEnd",
	"tui.editor.jumpForward",
	"tui.editor.jumpBackward",
	"tui.editor.pageUp",
	"tui.editor.pageDown",
	"tui.editor.deleteCharBackward",
	"tui.editor.deleteWordBackward",
	"tui.editor.deleteWordForward",
	"tui.editor.deleteToLineStart",
	"tui.editor.deleteToLineEnd",
	"tui.editor.yank",
	"tui.editor.yankPop",
	"tui.editor.undo",
	"tui.input.newLine",
	"tui.input.submit",
	"tui.input.tab",
	// Deliberately absent, and not collisions: `tui.editor.deleteCharForward`
	// and `app.exit` share ctrl+d, and `tui.input.copy`, `tui.select.cancel` and
	// `app.clear` share ctrl+c. Both pairs are one key with a state-dependent
	// meaning (exit only on an empty editor, cancel only with a selection), not
	// two handlers racing for the same keystroke.
	//
	// app actions wired in InteractiveMode.setupEditorActions
	"app.interrupt",
	"app.suspend",
	"app.thinking.cycle",
	"app.model.cycleForward",
	"app.model.cycleBackward",
	"app.model.select",
	"app.tools.expand",
	"app.view.cycleForward",
	"app.view.cycleBackward",
	"app.tools.unfoldOne",
	"app.tools.foldOne",
	"app.thinking.toggle",
	"app.tasks.cycleView",
	"app.team.focus",
	"app.editor.external",
	"app.input.voiceTranscribe",
	"app.message.followUp",
	"app.message.dequeue",
	"app.clipboard.pasteImage",
	"app.session.new",
	"app.session.tree",
	"app.session.fork",
	"app.session.resume",
	"app.session.changeDirectory",
	"app.settings.open",
	"app.hotkeys.open",
	"app.mode.cycle",
] as const;

/** The session picker: a query line plus its own verbs. */
const SESSION_PICKER_SCOPE = [
	"tui.select.up",
	"tui.select.down",
	"tui.select.pageUp",
	"tui.select.pageDown",
	"tui.select.confirm",
	"app.session.togglePath",
	"app.session.toggleSort",
	"app.session.rename",
	"app.session.delete",
	"app.session.deleteNoninvasive",
	"app.session.toggleNamedFilter",
] as const;

/** The scoped-models picker. */
const MODELS_PICKER_SCOPE = [
	"tui.select.up",
	"tui.select.down",
	"tui.select.confirm",
	"app.models.save",
	"app.models.enableAll",
	"app.models.clearAll",
	"app.models.toggleProvider",
	"app.models.reorderUp",
	"app.models.reorderDown",
] as const;

/** The session tree, including its five filter lenses. */
const TREE_SCOPE = [
	"tui.select.up",
	"tui.select.down",
	"app.tree.foldOrUp",
	"app.tree.unfoldOrDown",
	"app.tree.filter.default",
	"app.tree.filter.noTools",
	"app.tree.filter.userOnly",
	"app.tree.filter.labeledOnly",
	"app.tree.filter.all",
	"app.tree.filter.cycleForward",
	"app.tree.filter.cycleBackward",
] as const;

const SCOPES: Array<[string, readonly string[]]> = [
	["global", GLOBAL_SCOPE],
	["session picker", SESSION_PICKER_SCOPE],
	["models picker", MODELS_PICKER_SCOPE],
	["session tree", TREE_SCOPE],
];

function collisionsWithin(ids: readonly string[]): string[] {
	const manager = new KeybindingsManager();
	const byKey = new Map<string, string[]>();
	for (const id of ids) {
		for (const key of manager.getKeys(id as never)) {
			byKey.set(key, [...(byKey.get(key) ?? []), id]);
		}
	}
	return [...byKey.entries()]
		.filter(([, owners]) => owners.length > 1)
		.map(([key, owners]) => `${key}: ${owners.join(" + ")}`);
}

/**
 * The raw bytes a terminal without the Kitty protocol sends for a key, or
 * undefined when it sends nothing this parser recognises.
 */
function legacyInput(key: string): string | undefined {
	if (/^alt\+[a-z0-9]$/.test(key)) return `\x1b${key.slice(4)}`;
	if (/^ctrl\+[a-z]$/.test(key)) {
		return String.fromCharCode(key.charCodeAt(5) - 96);
	}
	return undefined;
}

/**
 * Ids in one scope that a single legacy keystroke would fire together.
 *
 * `getKeys` comparison alone misses this: `matchesKey` accepts ESC-p as both
 * alt+p and alt+up (the emacs previous/next aliases), so two ids on those two
 * key names collide on one keypress even though their KeyIds differ.
 */
function legacyCollisionsWithin(ids: readonly string[]): string[] {
	const manager = new KeybindingsManager();
	const inputs = new Set<string>();
	for (const id of ids) {
		for (const key of manager.getKeys(id as never)) {
			const input = legacyInput(key);
			if (input) inputs.add(input);
		}
	}

	const collisions: string[] = [];
	for (const input of inputs) {
		const owners = ids.filter((id) => manager.matches(input, id as never));
		if (owners.length > 1) collisions.push(`${JSON.stringify(input)}: ${owners.join(" + ")}`);
	}
	return collisions;
}

describe("keybinding layout", () => {
	it.each(SCOPES)("has no two %s bindings on the same key", (_name, ids) => {
		expect(collisionsWithin(ids)).toEqual([]);
	});

	it.each(SCOPES)("has no two %s bindings a single legacy keystroke fires", (_name, ids) => {
		expect(legacyCollisionsWithin(ids)).toEqual([]);
	});

	it("gives every action at least one key a non-Kitty terminal can send", () => {
		const manager = new KeybindingsManager();
		const unreachable = Object.keys(KEYBINDINGS).filter((id) => {
			const keys = manager.getKeys(id as never);
			// Unbound by default is a choice, not a gap; a shift-modified key is a
			// Kitty-only convenience and only counts as a gap when it is the only key.
			if (keys.length === 0) return false;
			return !keys.some((key) => !key.startsWith("shift+") || key === "shift+tab");
		});
		// The reverse halves of the cycles: each pairs with an unshifted key that
		// does work everywhere, so losing them costs an extra press, not an action.
		// `tui.input.newLine` is the long-standing exception the startup banner
		// already warns about ("extended-keys is off"), not a new gap.
		expect(unreachable).toEqual([
			"tui.input.newLine",
			"app.model.cycleBackward",
			"app.view.cycleBackward",
			"app.tools.foldOne",
			"app.tree.editLabel",
			"app.tree.toggleLabelTimestamp",
			"app.tree.filter.cycleBackward",
		]);
	});

	it("binds nothing to an alt key the parser cannot read", () => {
		const manager = new KeybindingsManager();
		const unparseable: string[] = [];
		for (const id of Object.keys(KEYBINDINGS)) {
			for (const key of manager.getKeys(id as never)) {
				// alt+<symbol> arrives as ESC + that symbol, which collides with the
				// CSI/OSC introducers; only letters and digits survive.
				if (/^alt\+[^a-z0-9]$/.test(key)) unparseable.push(`${id}: ${key}`);
			}
		}
		expect(unparseable).toEqual([]);
	});

	it("keeps every scope's ids real", () => {
		for (const [, ids] of SCOPES) {
			for (const id of ids) {
				expect(Object.hasOwn(KEYBINDINGS, id)).toBe(true);
			}
		}
	});

	it("leaves the emacs editing keys to the pickers' query lines", () => {
		// Every picker verb is on alt so ctrl+a/u/w/d/e still edit the query.
		const pickerVerbs = [...SESSION_PICKER_SCOPE, ...MODELS_PICKER_SCOPE, ...TREE_SCOPE].filter((id) =>
			id.startsWith("app."),
		);
		const manager = new KeybindingsManager();
		const ctrlLetterVerbs = pickerVerbs.filter((id) =>
			manager.getKeys(id as never).some((key) => /^ctrl\+[a-z]$/.test(key)),
		);
		expect(ctrlLetterVerbs).toEqual([]);
	});

	it("gives every action a description for /hotkeys", () => {
		const undocumented = Object.entries(KEYBINDINGS)
			.filter(([, definition]) => !definition.description)
			.map(([id]) => id);
		expect(undocumented).toEqual([]);
	});
});
