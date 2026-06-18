/**
 * Demo: Differential rendering & flicker-free live updates
 * --------------------------------------------------------
 * Headlines hoocode-tui's core selling point: a render loop that diffs the
 * previous frame against the next and only rewrites the cells that changed,
 * wrapped in synchronized-output markers (CSI 2026) so the screen never tears.
 *
 * Everything below is composed from the *real* library exports — no component
 * is reimplemented. We just mutate Text/Box children on a timer and call
 * `tui.requestRender()`; the framework does the efficient redraw.
 *
 * Run:  npx tsx packages/tui/demo/live-dashboard.ts
 */

import chalk from "chalk";
import { Box } from "../src/components/box.js";
import { Loader } from "../src/components/loader.js";
import { Text } from "../src/components/text.js";
import { matchesKey } from "../src/keys.js";
import { ProcessTerminal } from "../src/terminal.js";
import { TUI } from "../src/tui.js";

const terminal = new ProcessTerminal();
const tui = new TUI(terminal);

tui.addChild(new Text(chalk.bold.cyan("hoocode-tui · live dashboard")));
tui.addChild(
	new Text(chalk.dim("Updating 10×/sec with zero flicker — only changed cells are redrawn. Press Ctrl+C to exit.")),
);

// A Box gives us padding + an optional background so the panel reads as a unit.
const panel = new Box(2, 1, (s) => chalk.bgHex("#11141b")(s));
tui.addChild(panel);

// Live rows we mutate in place. Because these are the same component instances
// across frames, the renderer can diff them instead of clearing the screen.
const clockRow = new Text("", 0, 0);
const cpuRow = new Text("", 0, 0);
const memRow = new Text("", 0, 0);
const netRow = new Text("", 0, 0);
const framesRow = new Text("", 0, 0);
panel.addChild(clockRow);
panel.addChild(cpuRow);
panel.addChild(memRow);
panel.addChild(netRow);
panel.addChild(framesRow);

// A real Loader spinner, animating on its own interval.
const loader = new Loader(
	tui,
	(s) => chalk.cyan(s),
	(s) => chalk.dim(s),
	"streaming telemetry…",
);
tui.addChild(loader);

const bar = (value: number, width = 24, color: (s: string) => string = chalk.green): string => {
	const filled = Math.round((value / 100) * width);
	const meter = color("█".repeat(filled)) + chalk.dim("░".repeat(width - filled));
	return `${meter} ${value.toString().padStart(3)}%`;
};

// Smoothly-wandering fake metrics so the bars visibly move.
let cpu = 32;
let mem = 58;
let net = 12;
let frames = 0;
const wander = (v: number, lo = 2, hi = 98) => Math.max(lo, Math.min(hi, v + Math.round((Math.random() - 0.5) * 14)));

const timer = setInterval(() => {
	cpu = wander(cpu);
	mem = wander(mem);
	net = wander(net);
	frames++;

	clockRow.setText(chalk.bold("  time  ") + chalk.white(new Date().toLocaleTimeString()));
	cpuRow.setText(chalk.bold("  cpu   ") + bar(cpu, 24, cpu > 80 ? chalk.red : chalk.green));
	memRow.setText(chalk.bold("  mem   ") + bar(mem, 24, mem > 80 ? chalk.red : chalk.yellow));
	netRow.setText(chalk.bold("  net   ") + bar(net, 24, chalk.cyan));
	framesRow.setText(chalk.dim(`  rendered frames: ${frames}`));

	tui.requestRender();
}, 100);

tui.addInputListener((data) => {
	if (matchesKey(data, "ctrl+c")) {
		clearInterval(timer);
		loader.stop();
		tui.stop();
		process.exit(0);
	}
	return undefined;
});

tui.start();
