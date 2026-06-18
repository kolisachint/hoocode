#!/usr/bin/env tsx
// Generates assets/subagents-demo.svg — an ANIMATED preview of the `subagents`
// interactive demo (packages/coding-agent/demo/subagents.ts). It captures the
// REAL interactive-mode components at successive stages of the run and cross-fades
// between them on a pure-CSS loop, so it plays inline on the GitHub README (no JS,
// no download) and always matches what the demo actually draws:
//   - AskOptionsComponent  (the options pane)
//   - TaskPanelComponent   (the tasks lens + the depth-2 subagents lens)
//   - taskStore            (the singleton both read from)
//
// Re-run after editing the demo: `npx tsx scripts/gen-subagents-svg.ts`
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setKeybindings, visibleWidth } from "@kolisachint/hoocode-tui";
import type { AskQuestion } from "../packages/coding-agent/src/core/extensions/types.js";
import { KeybindingsManager } from "../packages/coding-agent/src/core/keybindings.js";
import { taskStore } from "../packages/coding-agent/src/core/task-store.js";
import type { TaskPanelView } from "../packages/coding-agent/src/modes/interactive/components/task-panel.js";
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

// ── capture real frames of the run as it progresses ────────────────────────────
initTheme("dark");
setKeybindings(KeybindingsManager.create());

type Frame = { caption: string; lines: string[] };
const frames: Frame[] = [];

const use = (input: number, output: number, cost: number) => ({ input, output, cacheRead: 0, cacheWrite: 0, cost });
const backdate = (id: number, secs: number) => {
	const t = taskStore.list().find((x) => x.id === id);
	if (t) (t as { createdAt: number }).createdAt = Date.now() - secs * 1000;
};
const finish = (id: number, secs: number, usage?: ReturnType<typeof use>) => {
	backdate(id, secs);
	taskStore.update(id, usage ? { status: "done", usage } : { status: "done" });
};

const panel = new TaskPanelComponent();
const snap = (caption: string, view: TaskPanelView) => {
	panel.setView(view);
	frames.push({ caption, lines: panel.render(COLS) });
};

// Frame 1 — the options pane (the agent asks before it acts).
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
frames.push({ caption: "the agent asks before it acts — options pane", lines: ask.render(COLS) });

// Frame 2 — the TodoWrite plan (tasks lens).
taskStore.upsertAgent({ id: "main", name: "hoocode", role: "orchestrator", kind: "main", state: "running" });
const survey = taskStore.create("Survey the codebase");
const implement = taskStore.create("Implement provider handoff");
const test = taskStore.create("Add cross-provider tests");
taskStore.create("Update the docs");
taskStore.update(survey.id, { status: "in_progress" });
snap("TodoWrite plan — the tasks lens", "flat");

// Frame 3 — work progresses through the plan.
finish(survey.id, 4.1);
taskStore.update(implement.id, { status: "in_progress" });
snap("working through the plan", "flat");

// Frame 4 — dispatch subagents; one dispatches a subagent of its own (depth 2).
finish(implement.id, 6.3);
taskStore.update(test.id, { status: "in_progress" });
taskStore.upsertAgent({ id: "explore", name: "explore", role: "subagent", kind: "subagent", state: "running" });
const ec = taskStore.create("explore: map the ai providers", { source: "subagent", agent: "explore", subagentMode: "explore" });
const e1 = taskStore.create("list provider modules", { source: "subagent", agent: "explore", parentTaskId: ec.id });
taskStore.upsertAgent({ id: "scan", name: "scan", role: "subagent", kind: "subagent", state: "running" });
const sc = taskStore.create("scan: grep for stream() impls", { source: "subagent", agent: "scan", subagentMode: "scan", parentTaskId: ec.id });
const s1 = taskStore.create("read openai-responses.ts", { source: "subagent", agent: "scan", parentTaskId: sc.id });
const s2 = taskStore.create("read anthropic.ts", { source: "subagent", agent: "scan", parentTaskId: sc.id });
finish(e1.id, 2.4, use(3100, 480, 0.02));
finish(s1.id, 1.2, use(2200, 300, 0.01));
taskStore.update(s2.id, { status: "in_progress" });
snap("subagents — explore dispatches scan (depth-2 tree)", "subagents");

// Frame 5 — complete, with the token + cost audit stamp on the header.
finish(s2.id, 1.0, use(1900, 260, 0.01));
finish(sc.id, 2.3, use(4300, 700, 0.03));
finish(ec.id, 5.6, use(8200, 1400, 0.06));
taskStore.upsertAgent({ id: "explore", name: "explore", kind: "subagent", state: "done", stats: { input: 12500, output: 2100, cost: 0.09 } });
taskStore.upsertAgent({ id: "scan", name: "scan", kind: "subagent", state: "done", stats: { input: 4300, output: 700, cost: 0.03 } });
taskStore.upsertAgent({ id: "reviewer", name: "reviewer", role: "subagent", kind: "subagent", state: "running" });
const rc = taskStore.create("reviewer: audit the handoff diff", { source: "subagent", agent: "reviewer", subagentMode: "reviewer" });
const r1 = taskStore.create("check message ordering", { source: "subagent", agent: "reviewer", parentTaskId: rc.id });
finish(r1.id, 1.5, use(2100, 360, 0.02));
finish(rc.id, 3.1, use(5100, 900, 0.04));
taskStore.upsertAgent({ id: "reviewer", name: "reviewer", kind: "subagent", state: "done", stats: { input: 5100, output: 900, cost: 0.04 } });
snap("complete — token + cost audit on the header", "subagents");

// ── layout ─────────────────────────────────────────────────────────────────────
const esc = (t: string) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const CAP_Y = HEAD + 20;
const BODY_Y = CAP_Y + LH + 6;
const maxLines = frames.reduce((mx, f) => Math.max(mx, f.lines.length), 0);
const H = Math.round(BODY_Y + maxLines * LH + 8);

// Place every character on the monospace cell grid via a per-character x list, so
// columns stay aligned and no glyph is stretched.
function textLine(runs: Run[], y: number, baseCol = 0): string {
	let col = baseCol;
	let tspans = "";
	for (const r of runs) {
		if (r.text.length === 0) continue;
		const xs: string[] = [];
		for (const ch of r.text) {
			xs.push((PADX + col * CW).toFixed(1));
			col += visibleWidth(ch) || 1;
		}
		tspans += `<tspan x="${xs.join(" ")}" fill="${r.fill}"${r.bold ? ' font-weight="600"' : ""}>${esc(r.text)}</tspan>`;
	}
	return `<text y="${y.toFixed(1)}" font-size="${FS}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace" xml:space="preserve">${tspans}</text>`;
}

// ── animation timing (pure-CSS, looping) ───────────────────────────────────────
const DUR = [2.8, 2.2, 2.2, 3.0, 4.0]; // seconds each frame holds (frame 5 lingers)
const T = DUR.reduce((a, b) => a + b, 0);
const starts: number[] = [];
let acc = 0;
for (const d of DUR) {
	starts.push(acc);
	acc += d;
}
const fadePct = (0.5 / T) * 100;

const groups: string[] = [];
const keyframes: string[] = [];
const classes: string[] = [];
frames.forEach((frame, i) => {
	// Each frame repaints the whole body so it fully occludes earlier frames; the
	// latest visible frame is last in document order, so it draws on top.
	let body = `<rect x="0" y="${HEAD}" width="${W}" height="${H - HEAD}" fill="${C.bg}"/>`;
	body += textLine([{ text: `▸ ${frame.caption}`, fill: C.cyan, bold: true }], CAP_Y);
	frame.lines.forEach((line, j) => {
		body += textLine(parseAnsi(line), BODY_Y + j * LH);
	});
	groups.push(`<g class="f${i}">${body}</g>`);

	const inFull = (starts[i] / T) * 100;
	const inHidden = Math.max(0, inFull - fadePct);
	keyframes.push(
		i === 0
			? `@keyframes f0{0%,96%{opacity:1}100%{opacity:0}}`
			: `@keyframes f${i}{0%,${inHidden.toFixed(2)}%{opacity:0}${inFull.toFixed(2)}%,96%{opacity:1}100%{opacity:0}}`,
	);
	classes.push(`.f${i}{opacity:0;animation:f${i} ${T}s linear infinite}`);
});

const css = `text{dominant-baseline:alphabetic}\n${classes.join("\n")}\n${keyframes.join("\n")}`;

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="HooCode subagents demo: options pane, TodoWrite tasks lens, and a depth-2 subagents tree, animated">
  <title>HooCode — subagents demo (options pane → task pane, depth-2)</title>
  <style>${css}</style>
  <rect width="${W}" height="${H}" rx="12" fill="${C.bg}"/>
  <rect width="${W}" height="${HEAD}" rx="12" fill="${C.chrome}"/>
  <rect y="${HEAD - 12}" width="${W}" height="12" fill="${C.chrome}"/>
  <line x1="0" y1="${HEAD}" x2="${W}" y2="${HEAD}" stroke="${C.border}" stroke-width="1"/>
  <circle cx="24" cy="${HEAD / 2}" r="6" fill="#cc6666"/>
  <circle cx="44" cy="${HEAD / 2}" r="6" fill="#e6c547"/>
  <circle cx="64" cy="${HEAD / 2}" r="6" fill="#7fb069"/>
  <text x="${W / 2}" y="${(HEAD / 2 + 3.5).toFixed(1)}" text-anchor="middle" font-size="${FS}" fill="${C.muted}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">hoocode — subagents demo</text>
  ${groups.join("\n  ")}
</svg>
`;

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const out = join(root, "assets", "subagents-demo.svg");
writeFileSync(out, svg);
console.log(`wrote ${out} (${svg.length} bytes, ${frames.length} frames, ${T}s loop, viewBox 0 0 ${W} ${H})`);
