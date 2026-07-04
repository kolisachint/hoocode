/**
 * Utilities for formatting keybinding hints in the UI.
 */

import { getKeybindings, type Keybinding, type KeyId, matchesKey } from "@kolisachint/hoocode-tui";
import { type AppKeybinding, KEYBINDINGS } from "../../../core/keybindings.js";
import { theme } from "../theme/theme.js";

export interface KeyTextFormatOptions {
	capitalize?: boolean;
}

function formatKeyPart(part: string, options: KeyTextFormatOptions): string {
	const displayPart = process.platform === "darwin" && part.toLowerCase() === "alt" ? "option" : part;
	return options.capitalize ? displayPart.charAt(0).toUpperCase() + displayPart.slice(1) : displayPart;
}

export function formatKeyText(key: string, options: KeyTextFormatOptions = {}): string {
	return key
		.split("/")
		.map((k) =>
			k
				.split("+")
				.map((part) => formatKeyPart(part, options))
				.join("+"),
		)
		.join("/");
}

function formatKeys(keys: KeyId[], options: KeyTextFormatOptions = {}): string {
	if (keys.length === 0) return "";
	return formatKeyText(keys.join("/"), options);
}

export function keyText(keybinding: Keybinding): string {
	return formatKeys(getKeybindings().getKeys(keybinding));
}

export function keyDisplayText(keybinding: Keybinding): string {
	return formatKeys(getKeybindings().getKeys(keybinding), { capitalize: true });
}

export function keyHint(keybinding: Keybinding, description: string): string {
	return theme.fg("dim", keyText(keybinding)) + theme.fg("muted", ` ${description}`);
}

export function rawKeyHint(key: string, description: string): string {
	return theme.fg("dim", formatKeyText(key)) + theme.fg("muted", ` ${description}`);
}

/** Default keys of an app-level binding, from its definition. */
function appDefaultKeys(id: AppKeybinding): KeyId[] {
	const defaults = KEYBINDINGS[id].defaultKeys;
	return Array.isArray(defaults) ? defaults : [defaults];
}

/**
 * Match input against an app-level keybinding. The global manager is the
 * app-aware one in normal runs (user overrides apply); when it is the bare TUI
 * default (tests, early boot) the app definition's default keys are used so
 * the component never goes dead.
 */
export function matchesAppKey(data: string, id: AppKeybinding): boolean {
	const keybindings = getKeybindings();
	if (keybindings.getDefinition(id)) return keybindings.matches(data, id);
	return appDefaultKeys(id).some((key) => matchesKey(data, key));
}

/** First configured key for an app-level binding, for hint lines. */
export function appKeyLabel(id: AppKeybinding): string {
	const keybindings = getKeybindings();
	const keys = keybindings.getDefinition(id) ? keybindings.getKeys(id) : undefined;
	if (keys && keys.length > 0) return keys[0] as string;
	return (appDefaultKeys(id)[0] ?? "") as string;
}
