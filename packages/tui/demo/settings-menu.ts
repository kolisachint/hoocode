/**
 * Demo: SettingsList — an interactive, searchable settings menu
 * -------------------------------------------------------------
 * SettingsList renders a label/value table where Enter/Space cycles a value
 * through a fixed set of options, optional fuzzy search filters the rows, and
 * an item can open a submenu. Arrow keys move the cursor; Esc exits.
 *
 * Run:  npx tsx packages/tui/demo/settings-menu.ts
 */

import chalk from "chalk";
import { type SettingItem, SettingsList, type SettingsListTheme } from "../src/components/settings-list.js";
import { Text } from "../src/components/text.js";
import { matchesKey } from "../src/keys.js";
import { ProcessTerminal } from "../src/terminal.js";
import { TUI } from "../src/tui.js";

const terminal = new ProcessTerminal();
const tui = new TUI(terminal);

const theme: SettingsListTheme = {
	label: (text, selected) => (selected ? chalk.bold.white(text) : chalk.white(text)),
	value: (text, selected) => (selected ? chalk.bold.cyan(text) : chalk.cyan(text)),
	description: (text) => chalk.dim(text),
	cursor: chalk.cyan("→"),
	hint: (text) => chalk.dim(text),
};

tui.addChild(new Text(chalk.bold.cyan("hoocode-tui · settings")));
tui.addChild(new Text(chalk.dim("↑/↓ move · Enter/Space cycles value · type to search · Esc or Ctrl+C to exit")));

const status = new Text(chalk.dim("Adjust a setting to see the change event fire."));
tui.addChild(status);

const items: SettingItem[] = [
	{
		id: "theme",
		label: "Color theme",
		description: "Palette used to render the UI",
		currentValue: "dark",
		values: ["dark", "light", "high-contrast", "solarized"],
	},
	{
		id: "editor.keymap",
		label: "Editor keymap",
		description: "Keybinding preset for the editor",
		currentValue: "default",
		values: ["default", "vim", "emacs"],
	},
	{
		id: "editor.wrap",
		label: "Soft wrap",
		description: "Wrap long lines at the viewport edge",
		currentValue: "on",
		values: ["on", "off"],
	},
	{
		id: "telemetry",
		label: "Telemetry",
		description: "Send anonymous usage data",
		currentValue: "off",
		values: ["off", "on"],
	},
	{
		id: "font.size",
		label: "Font size",
		description: "Cell font size in points",
		currentValue: "14",
		values: ["12", "13", "14", "16", "18"],
	},
];

const settings = new SettingsList(
	items,
	8,
	theme,
	(id, newValue) => {
		status.setText(chalk.green("✔ ") + chalk.bold(id) + chalk.dim(" → ") + chalk.cyan(newValue));
		tui.requestRender();
	},
	() => {
		tui.stop();
		process.exit(0);
	},
	{ enableSearch: true },
);

tui.addChild(settings);
tui.setFocus(settings);

tui.addInputListener((data) => {
	if (matchesKey(data, "ctrl+c")) {
		tui.stop();
		process.exit(0);
	}
	return undefined;
});

tui.start();
