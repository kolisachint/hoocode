#!/usr/bin/env tsx
// Generates assets/subagents-demo.svg — a static preview of the `subagents`
// interactive demo (packages/coding-agent/demo/subagents.ts), rendered from the
// REAL interactive-mode components so the picture matches what the demo draws:
//   - AskOptionsComponent  (the options pane)
//   - TaskPanelComponent   (the tasks lens + the depth-2 subagents lens)
//   - taskStore            (the singleton both read from)
//
// We render each panel to its real ANSI lines, parse the SGR colours, and lay
// them out in a terminal-window SVG matching assets/demo.svg's chrome.
//
// Re-run after editing the demo: `npx tsx scripts/gen-subagents-svg.ts`
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setKeybindings, visibleWidth } from "@kolisachint/hoocode-tui";
import type { AskQuestion } from "../packages/coding-agent/src/core/extensions/types.js";
import { KeybindingsManager } from "../packages/coding-agent/src/core/keybindings.js";
import { taskStore } from "../packages/coding-agent/src/core/task-store.js";
import { AskOptionsComponent } from "../packages/coding-agent/src/modes/interactive/components/ask-options.js";
import { TaskPanelComponent } from "../packages/coding-agent/src/modes/interactive/components/task-panel.js";
import { initTheme } from "../packages/coding-agent/src/modes/interactive/theme/theme.js";

const COLS = 82;

// ── palette / layout (mirrors assets/demo.svg) ─────────────────────────────────
const C = {
	bg: "#13131a",
	chrome: "#1e1e24",
	border: "#2a2a33",
	text: "#c9cdd3",
	dim: "#666666",
	muted: "#808080",
	cyan: "#00d7ff",
};
const PADX = 18;
const HEAD = 38;
const LH = 15.5;
const FS = 11;
// Slightly generous advance so the right-aligned status column never clips,
// whatever monospace font the SVG viewer resolves.
// Natural monospace advance. Each glyph is placed on the cell grid via a per-character
// x list (below), so columns stay aligned without stretching any glyph.
const CW = FS * 0.6;
const W = Math.round(PADX * 2 + COLS * CW);

// ── xterm-256 → hex ────────────────────────────────────────────────────────────
const BASE16 = [
	"#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
	"#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
];
const hx = (v: number) => v.toString(16).padStart(2, "0");
function xterm256(n: number): string {
	if (n < 16) return BASE16[n];
	if (n >= 232) {
		const v = 8 + (n - 232) * 10;
		return `#${hx(v)}${hx(v)}${hx(v)}`;
	}
	const i = n - 16;
	const r = Math.floor(i / 36);
	const g = Math.floor((i % 36) / 6);
	const b = i % 6;
	const c = (x: number) => (x === 0 ? 0 : 55 + x * 40);
	return `#${hx(c(r))}${hx(c(g))}${hx(c(b))}`;
}

type Run = { text: string; fill: string; bold: boolean };

// Strip non-SGR escapes (APC cursor marker, OSC, other CSI), then tokenize SGR.
function parseAnsi(line: string): Run[] {
	const clean = line
		// strip APC (cursor marker) / OSC sequences, then non-SGR CSI (anything
		// ending in a letter other than `m`); SGR `…m` is left for the tokenizer.
		.replace(/\x1b[_\]][^\x07]*\x07/g, "")
		.replace(/\x1b\[[0-9;]*[A-Za-ln-z]/g, "");
	const runs: Run[] = [];
	let fill = C.text;
	let bold = false;
	const re = /\x1b\[([0-9;]*)m/g;
	let last = 0;
	let m: RegExpExecArray | null;
	const push = (text: string) => {
		if (text) runs.push({ text, fill, bold });
	};
	// biome-ignore lint/suspicious/noAssignInExpressions: standard regex walk
	while ((m = re.exec(clean)) !== null) {
		push(clean.slice(last, m.index));
		last = re.lastIndex;
		const params = m[1].split(";").map((p) => Number(p || 0));
		for (let i = 0; i < params.length; i++) {
			const p = params[i];
			if (p === 0) {
				fill = C.text;
				bold = false;
			} else if (p === 1) bold = true;
			else if (p === 22) bold = false;
			else if (p === 39) fill = C.text;
			else if (p === 38 && params[i + 1] === 5) {
				fill = xterm256(params[i + 2] ?? 7);
				i += 2;
			}
		}
	}
	push(clean.slice(last));
	return runs;
}

// ── capture the real panels ────────────────────────────────────────────────────
initTheme("dark");
setKeybindings(KeybindingsManager.create());

const question: AskQuestion = {
	question: "How thorough should this change be?",
	short: "Scope",
	options: [
		{ label: "Standard", description: "explore (+ nested scan) then review", recommended: true },
		{ label: "Deep", description: "explore, nested scan, and a longer review" },
		{ label: "Quick", description: "explore only — skip the review" },
	],
};
const ask = new AskOptionsComponent([question], () => {}, () => {});
ask.focused = true;
const optionLines = ask.render(COLS);

// Build the depth-2 scenario the demo ends on, including per-task token/cost
// usage so the header carries its `turn ↑… ↓… · $…` audit stamp and done rows
// show their token + elapsed column. createdAt is backdated so elapsed reads
// realistically instead of 0.0s.
const use = (input: number, output: number, cost: number) => ({ input, output, cacheRead: 0, cacheWrite: 0, cost });
const elapsed = (t: { id: number }, secs: number) => {
	const task = taskStore.list().find((x) => x.id === t.id);
	if (task) (task as { createdAt: number }).createdAt = Date.now() - secs * 1000;
};
const done = (t: { id: number }, secs: number, usage?: ReturnType<typeof use>) => {
	elapsed(t, secs);
	taskStore.update(t.id, usage ? { status: "done", usage } : { status: "done" });
};

taskStore.upsertAgent({ id: "main", name: "hoocode", role: "orchestrator", kind: "main", state: "running" });
const survey = taskStore.create("Survey the codebase");
const implement = taskStore.create("Implement provider handoff");
const test = taskStore.create("Add cross-provider tests");
taskStore.create("Update the docs");
done(survey, 4.1);
done(implement, 6.3);
taskStore.update(test.id, { status: "in_progress" });

taskStore.upsertAgent({ id: "explore", name: "explore", role: "subagent", kind: "subagent", state: "done", stats: { input: 12500, output: 2100, cost: 0.09 } });
const ec = taskStore.create("explore: map the ai providers", { source: "subagent", agent: "explore", subagentMode: "explore" });
const e1 = taskStore.create("list provider modules", { source: "subagent", agent: "explore", parentTaskId: ec.id });
taskStore.upsertAgent({ id: "scan", name: "scan", role: "subagent", kind: "subagent", state: "running" });
const sc = taskStore.create("scan: grep for stream() impls", { source: "subagent", agent: "scan", subagentMode: "scan", parentTaskId: ec.id });
const s1 = taskStore.create("read openai-responses.ts", { source: "subagent", agent: "scan", parentTaskId: sc.id });
const s2 = taskStore.create("read anthropic.ts", { source: "subagent", agent: "scan", parentTaskId: sc.id });
done(e1, 2.4, use(3100, 480, 0.02));
done(s1, 1.2, use(2200, 300, 0.01));
done(ec, 5.6, use(8200, 1400, 0.06));
taskStore.update(s2.id, { status: "in_progress" });
taskStore.upsertAgent({ id: "reviewer", name: "reviewer", role: "subagent", kind: "subagent", state: "running" });
const rc = taskStore.create("reviewer: audit the handoff diff", { source: "subagent", agent: "reviewer", subagentMode: "reviewer" });
taskStore.create("check message ordering", { source: "subagent", agent: "reviewer", parentTaskId: rc.id });

const panel = new TaskPanelComponent();
panel.setView("flat");
const tasksLines = panel.render(COLS);
panel.setView("subagents");
const subagentLines = panel.render(COLS);

// ── assemble rows (captions + captured panels) ─────────────────────────────────
type Row = { runs: Run[]; gap?: number };
const caption = (text: string): Row => ({ runs: [{ text, fill: C.cyan, bold: true }], gap: 14 });
const blank: Row = { runs: [] };

const rows: Row[] = [
	caption("1 · options pane — the agent asks before it acts"),
	...optionLines.map((l) => ({ runs: parseAnsi(l) })),
	caption("2 · tasks lens — the main agent's TodoWrite plan"),
	...tasksLines.map((l) => ({ runs: parseAnsi(l) })),
	caption("3 · subagents lens — depth-2 tree (Ctrl+N swaps lenses)"),
	...subagentLines.map((l) => ({ runs: parseAnsi(l) })),
	blank,
];

// ── emit SVG ───────────────────────────────────────────────────────────────────
const esc = (t: string) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const TOP = HEAD + 24;
let y = TOP;
const lineEls: string[] = [];
for (const row of rows) {
	if (row.gap) y += row.gap;
	// Place every character on the monospace cell grid with a per-character x list.
	// No textLength/lengthAdjust, so glyphs render at their natural width (no
	// stretching) while tree connectors and right-aligned columns stay aligned.
	let col = 0;
	let tspans = "";
	for (const r of row.runs) {
		if (r.text.length === 0) continue;
		const xs: string[] = [];
		for (const ch of r.text) {
			xs.push((PADX + col * CW).toFixed(1));
			col += visibleWidth(ch) || 1;
		}
		tspans += `<tspan x="${xs.join(" ")}" fill="${r.fill}"${r.bold ? ' font-weight="600"' : ""}>${esc(r.text)}</tspan>`;
	}
	lineEls.push(
		`<text y="${y.toFixed(1)}" font-size="${FS}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace" xml:space="preserve">${tspans}</text>`,
	);
	y += LH;
}
const H = Math.round(y + 12);

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="HooCode subagents demo: options pane, tasks lens, and a depth-2 subagents tree">
  <title>HooCode — subagents demo (options pane → task pane, depth-2)</title>
  <style>text{dominant-baseline:alphabetic}</style>
  <rect width="${W}" height="${H}" rx="12" fill="${C.bg}"/>
  <rect width="${W}" height="${HEAD}" rx="12" fill="${C.chrome}"/>
  <rect y="${HEAD - 12}" width="${W}" height="12" fill="${C.chrome}"/>
  <line x1="0" y1="${HEAD}" x2="${W}" y2="${HEAD}" stroke="${C.border}" stroke-width="1"/>
  <circle cx="24" cy="${HEAD / 2}" r="6" fill="#cc6666"/>
  <circle cx="44" cy="${HEAD / 2}" r="6" fill="#e6c547"/>
  <circle cx="64" cy="${HEAD / 2}" r="6" fill="#7fb069"/>
  <text x="${W / 2}" y="${(HEAD / 2 + 3.5).toFixed(1)}" text-anchor="middle" font-size="${FS}" fill="${C.muted}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">hoocode — subagents demo</text>
  ${lineEls.join("\n  ")}
</svg>
`;

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const out = join(root, "assets", "subagents-demo.svg");
writeFileSync(out, svg);
console.log(`wrote ${out} (${svg.length} bytes, ${rows.length} rows, viewBox 0 0 ${W} ${H})`);
