/**
 * Demo: Overlays + SelectList + focus management
 * ----------------------------------------------
 * A VS Code / Slack-style command palette. Press Ctrl+K to pop a centered
 * overlay; arrow-navigate the SelectList, Enter runs the command, Esc closes.
 *
 * Demonstrates three real features at once:
 *   - tui.showOverlay()  — render on top of existing content with positioning
 *   - SelectList         — a focusable, keyboard-driven chooser
 *   - focus capture/restore — the overlay grabs focus on open and the TUI
 *                             returns focus to where it was on close.
 *
 * Run:  npx tsx packages/tui/demo/command-palette.ts
 */

import chalk from "chalk";
import { type SelectItem, SelectList } from "../src/components/select-list.js";
import { Text } from "../src/components/text.js";
import { matchesKey } from "../src/keys.js";
import { ProcessTerminal } from "../src/terminal.js";
import { TUI } from "../src/tui.js";
import { defaultSelectListTheme } from "../test/test-themes.js";

const terminal = new ProcessTerminal();
const tui = new TUI(terminal);

tui.addChild(new Text(chalk.bold.cyan("hoocode-tui · command palette")));
const hint = new Text(chalk.dim("Press Ctrl+K to open the palette · Ctrl+C to exit"));
tui.addChild(hint);

const status = new Text(chalk.dim("No command run yet."));
tui.addChild(status);

const commands: SelectItem[] = [
	{ value: "new-file", label: "New File", description: "Create an empty file in the workspace" },
	{ value: "open-folder", label: "Open Folder…", description: "Pick a directory to work in" },
	{ value: "toggle-theme", label: "Toggle Theme", description: "Switch between light and dark" },
	{ value: "git-commit", label: "Git: Commit", description: "Stage all changes and commit" },
	{ value: "git-push", label: "Git: Push", description: "Push the current branch to origin" },
	{ value: "format-doc", label: "Format Document", description: "Run the formatter on this file" },
	{ value: "split-editor", label: "Split Editor Right", description: "Open a second pane" },
	{ value: "reload-window", label: "Reload Window", description: "Restart the UI" },
];

let paletteOpen = false;

const openPalette = () => {
	if (paletteOpen) return;
	paletteOpen = true;

	const list = new SelectList(commands, 6, defaultSelectListTheme);

	// showOverlay centers by default and returns a handle we use to close it.
	const handle = tui.showOverlay(list, { width: 60, anchor: "center" });

	const close = () => {
		paletteOpen = false;
		handle.hide(); // removes the overlay and restores prior focus for us
		tui.requestRender();
	};

	list.onSelect = (item) => {
		status.setText(chalk.green("✔ ran ") + chalk.bold(item.label) + chalk.dim(`  (${item.value})`));
		close();
	};
	list.onCancel = close;
};

tui.addInputListener((data) => {
	if (matchesKey(data, "ctrl+c")) {
		tui.stop();
		process.exit(0);
	}
	// Only the main view handles Ctrl+K; while the overlay is open it owns input.
	if (!paletteOpen && matchesKey(data, "ctrl+k")) {
		openPalette();
		return { consume: true };
	}
	return undefined;
});

tui.start();
