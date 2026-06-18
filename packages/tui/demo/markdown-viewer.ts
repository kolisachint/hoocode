/**
 * Demo: Markdown rendering + live theme switching
 * -----------------------------------------------
 * The Markdown component turns a markdown string into styled terminal lines:
 * headings, lists, blockquotes, fenced code blocks, inline code, links, and
 * bold/italic/strikethrough. Themes are just functions, so swapping the theme
 * re-styles the whole document — and the differential renderer only repaints
 * the cells whose styling actually changed.
 *
 * Press T to cycle the theme. Ctrl+C to exit.
 *
 * Run:  npx tsx packages/tui/demo/markdown-viewer.ts
 */

import chalk from "chalk";
import { Markdown, type MarkdownTheme } from "../src/components/markdown.js";
import { Text } from "../src/components/text.js";
import { matchesKey } from "../src/keys.js";
import { ProcessTerminal } from "../src/terminal.js";
import { TUI } from "../src/tui.js";
import { defaultMarkdownTheme } from "../test/test-themes.js";

const terminal = new ProcessTerminal();
const tui = new TUI(terminal);

// A second, monochrome theme to contrast with the default colorful one.
const monoTheme: MarkdownTheme = {
	heading: (t) => chalk.bold.underline(t),
	link: (t) => chalk.underline(t),
	linkUrl: (t) => chalk.dim(t),
	code: (t) => chalk.inverse(t),
	codeBlock: (t) => chalk.white(t),
	codeBlockBorder: (t) => chalk.dim(t),
	quote: (t) => chalk.italic.dim(t),
	quoteBorder: (t) => chalk.dim(t),
	hr: (t) => chalk.dim(t),
	listBullet: (t) => chalk.bold(t),
	bold: (t) => chalk.bold(t),
	italic: (t) => chalk.italic(t),
	strikethrough: (t) => chalk.strikethrough(t),
	underline: (t) => chalk.underline(t),
};

const themes: Array<{ name: string; theme: MarkdownTheme }> = [
	{ name: "default (color)", theme: defaultMarkdownTheme },
	{ name: "mono", theme: monoTheme },
];

const doc = `# hoocode-tui

A **minimal** terminal UI framework with _differential rendering_ and
flicker-free ~~tearing~~ updates.

## Why it's nice

- Only redraws the cells that **changed**
- Synchronized output via CSI 2026
- Components accept *theme* functions, e.g. [the docs](https://example.com)

## Quick start

\`\`\`ts
const tui = new TUI(new ProcessTerminal());
tui.addChild(new Markdown(doc, 1, 1, theme));
tui.start();
\`\`\`

> Tip: pass any \`MarkdownTheme\` to restyle the whole document at once.

---

Inline \`code\` renders too.`;

tui.addChild(new Text(chalk.bold.cyan("hoocode-tui · markdown viewer")));
const hint = new Text("");
tui.addChild(hint);

let themeIndex = 0;
let markdown = new Markdown(doc, 2, 1, themes[themeIndex].theme);
tui.addChild(markdown);

const updateHint = () => {
	hint.setText(chalk.dim(`Theme: ${chalk.white(themes[themeIndex].name)} · press T to toggle · Ctrl+C to exit`));
};
updateHint();

tui.addInputListener((data) => {
	if (matchesKey(data, "ctrl+c")) {
		tui.stop();
		process.exit(0);
	}
	if (matchesKey(data, "t") || matchesKey(data, "shift+t")) {
		themeIndex = (themeIndex + 1) % themes.length;
		// Markdown takes its theme at construction, so swap the child for the
		// new theme. It stays the last child, keeping the layout order.
		tui.removeChild(markdown);
		markdown = new Markdown(doc, 2, 1, themes[themeIndex].theme);
		tui.addChild(markdown);
		updateHint();
		tui.requestRender();
		return { consume: true };
	}
	return undefined;
});

tui.start();
