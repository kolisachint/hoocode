#!/usr/bin/env node
/**
 * A/B latency harness for hoocode.
 *
 * Runs the same prompt at different thinking levels (and/or models) in print
 * JSON mode, timestamping each streamed event as it arrives, and reports:
 *
 *   - startup_ms    process spawn -> first session event (Node + init cost)
 *   - ttft_ms       turn_start -> first model stream event (prefill / TTFT)
 *   - ttf_text_ms   turn_start -> first visible text delta (thinking tax shows here)
 *   - total_ms      process spawn -> process exit (whole turn, incl. tools)
 *   - tokens        input / output / cacheRead / cacheWrite (from final usage)
 *   - cache         HIT if cacheRead > 0 (warm prefix), else MISS (cold write)
 *
 * Because every `-p` invocation reuses the same system prompt + tool schemas,
 * the cached prefix is reused across runs within the cache TTL: run 1 is cold
 * (cacheWrite), later runs are warm (cacheRead). That cold/warm split is the
 * point — it isolates how much of the latency is prefill you can cache away.
 *
 * Usage:
 *   node scripts/bench-latency.mjs [options]
 *
 * Options:
 *   --levels off,medium        Comma list of thinking levels to compare (default: off,medium)
 *   --runs 3                   Runs per level (default: 3)
 *   --prompt "..."             Prompt to send (default: a no-tool prompt)
 *   --tool                     Use a built-in tool-triggering prompt instead
 *   --subagent                 Measure the spawned-subagent boot path: runs with
 *                              --task-id (trims themes/slash-commands/prompt-templates)
 *                              and an explore-style MCP-free allowlist + HOOCODE_SKIP_MCP,
 *                              so startup_ms reflects what a real dispatch pays.
 *   --model <pattern>          Pass --model to hoocode (default: configured model)
 *   --cli <path>              Path to cli.js (default: packages/coding-agent/dist/cli.js)
 *   --build                    Rebuild coding-agent before running
 *   --json                     Emit raw machine-readable JSON results
 *   --keep-warm                Don't sleep between levels (default sleeps 0; cache shared)
 *   --help
 */

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";
import { existsSync } from "node:fs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");

const NO_TOOL_PROMPT = "In one short sentence, what is a binary search tree? Do not use any tools.";
const TOOL_PROMPT =
	"List the files in the current directory using your tools, then in one sentence say what kind of project this is.";

function parseArgs(argv) {
	const opts = {
		levels: ["off", "medium"],
		runs: 3,
		prompt: undefined,
		useTool: false,
		subagent: false,
		model: undefined,
		cli: resolve(repoRoot, "packages/coding-agent/dist/cli.js"),
		build: false,
		json: false,
	};
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (a === "--help" || a === "-h") {
			printHelp();
			process.exit(0);
		} else if (a === "--levels") opts.levels = argv[++i].split(",").map((s) => s.trim()).filter(Boolean);
		else if (a === "--runs") opts.runs = Number.parseInt(argv[++i], 10);
		else if (a === "--prompt") opts.prompt = argv[++i];
		else if (a === "--tool") opts.useTool = true;
		else if (a === "--subagent") opts.subagent = true;
		else if (a === "--model") opts.model = argv[++i];
		else if (a === "--cli") opts.cli = resolve(process.cwd(), argv[++i]);
		else if (a === "--build") opts.build = true;
		else if (a === "--json") opts.json = true;
		else throw new Error(`Unknown argument: ${a}`);
	}
	if (!opts.prompt) opts.prompt = opts.useTool ? TOOL_PROMPT : NO_TOOL_PROMPT;
	if (!Number.isInteger(opts.runs) || opts.runs < 1) throw new Error(`Invalid --runs`);
	return opts;
}

function printHelp() {
	console.log(`A/B latency harness for hoocode.

Usage: node scripts/bench-latency.mjs [options]

  --levels off,medium   Thinking levels to compare (default: off,medium)
  --runs 3              Runs per level (default: 3)
  --prompt "..."        Custom prompt
  --tool                Use a tool-triggering prompt
  --subagent            Measure the spawned-subagent boot path (trimmed boot + MCP skip)
  --model <pattern>     Pass --model to hoocode
  --cli <path>          cli.js path (default: packages/coding-agent/dist/cli.js)
  --build               Rebuild coding-agent first
  --json                Raw JSON output
  --help`);
}

function run(cmd, args, opts = {}) {
	return new Promise((resolveP, reject) => {
		const child = spawn(cmd, args, { stdio: "inherit", ...opts });
		child.on("error", reject);
		child.on("exit", (code) => (code === 0 ? resolveP() : reject(new Error(`${cmd} exited ${code}`))));
	});
}

/** Run one invocation and collect timing + usage. */
function measureRun(cli, level, prompt, model, subagent = false) {
	return new Promise((resolveP, reject) => {
		const args = [cli, "-p", "--mode", "json", "--thinking", level];
		if (model) args.push("--model", model);
		// Reproduce a spawned subagent's boot path: a task id flips on the trimmed
		// boot profile (no themes/slash-commands/prompt-templates) and an MCP-free
		// allowlist mirrors an explore dispatch, with HOOCODE_SKIP_MCP set the way
		// the pool sets it for such a child.
		const childEnv = subagent
			? { ...process.env, HOOCODE_SKIP_MCP: "1", HOOCODE_SUBAGENT_DEPTH: "1" }
			: process.env;
		if (subagent) {
			args.push("--task-id", `bench-${Date.now()}`, "--tools", "read,grep,find,ls");
		}
		args.push(prompt);

		const t0 = performance.now();
		const m = {
			level,
			startupMs: null,
			ttftMs: null,
			ttfTextMs: null,
			totalMs: null,
			turns: 0,
			usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
			error: null,
		};
		let turnStartAt = null;
		let firstStreamAt = null;
		let firstTextAt = null;

		const child = spawn("node", args, { cwd: repoRoot, stdio: ["ignore", "pipe", "pipe"], env: childEnv });
		let stderr = "";
		child.stderr.on("data", (d) => (stderr += d.toString()));

		const rl = createInterface({ input: child.stdout });
		rl.on("line", (line) => {
			const now = performance.now();
			if (!line.trim()) return;
			let ev;
			try {
				ev = JSON.parse(line);
			} catch {
				return; // ping or non-JSON noise
			}
			if (m.startupMs === null && ev.type === "session") m.startupMs = now - t0;

			if (ev.type === "turn_start") {
				if (turnStartAt === null) turnStartAt = now; // first turn anchors TTFT
			} else if (ev.type === "message_update") {
				const inner = ev.assistantMessageEvent;
				// First model stream activity of the first turn = prefill done (TTFT).
				if (firstStreamAt === null && turnStartAt !== null) firstStreamAt = now;
				// First user-visible answer text. With thinking on, this lags TTFT.
				if (firstTextAt === null && inner && inner.type === "text_delta") firstTextAt = now;
			} else if (ev.type === "turn_end") {
				m.turns++;
				const u = ev.message?.usage;
				if (u) {
					m.usage.input += u.input ?? 0;
					m.usage.output += u.output ?? 0;
					m.usage.cacheRead += u.cacheRead ?? 0;
					m.usage.cacheWrite += u.cacheWrite ?? 0;
				}
				if (ev.message?.stopReason === "error") m.error = ev.message.errorMessage || "model error";
			}
		});

		child.on("error", reject);
		child.on("exit", (code) => {
			m.totalMs = performance.now() - t0;
			if (turnStartAt !== null && firstStreamAt !== null) m.ttftMs = firstStreamAt - turnStartAt;
			if (turnStartAt !== null && firstTextAt !== null) m.ttfTextMs = firstTextAt - turnStartAt;
			if (code !== 0 && !m.error) m.error = `exit ${code}: ${stderr.trim().split("\n").slice(-2).join(" ")}`;
			resolveP(m);
		});
	});
}

function median(xs) {
	const v = xs.filter((x) => x != null).sort((a, b) => a - b);
	if (v.length === 0) return null;
	const mid = Math.floor(v.length / 2);
	return v.length % 2 ? v[mid] : (v[mid - 1] + v[mid]) / 2;
}
const fmt = (x) => (x == null ? "  —  " : `${Math.round(x)}`.padStart(5));

async function main() {
	const opts = parseArgs(process.argv.slice(2));

	if (opts.build) {
		console.error("Building coding-agent…");
		await run("npm", ["run", "build"], { cwd: resolve(repoRoot, "packages/coding-agent") });
	}
	if (!existsSync(opts.cli)) {
		throw new Error(`CLI not found at ${opts.cli} — run with --build or pass --cli <path>`);
	}

	console.error(`\nhoocode latency A/B`);
	console.error(`  cli:     ${opts.cli}`);
	console.error(`  model:   ${opts.model ?? "(configured default)"}`);
	console.error(`  levels:  ${opts.levels.join(", ")}`);
	console.error(`  runs:    ${opts.runs} per level`);
	if (opts.subagent) console.error(`  mode:    subagent boot (trimmed boot + MCP skip)`);
	console.error(`  prompt:  ${JSON.stringify(opts.prompt)}\n`);

	const all = [];
	for (const level of opts.levels) {
		for (let i = 0; i < opts.runs; i++) {
			process.stderr.write(`  [${level}] run ${i + 1}/${opts.runs}… `);
			const m = await measureRun(opts.cli, level, opts.prompt, opts.model, opts.subagent);
			all.push(m);
			const cache = m.usage.cacheRead > 0 ? "warm" : "cold";
			process.stderr.write(
				m.error
					? `ERROR: ${m.error}\n`
					: `ttft=${fmt(m.ttftMs)}ms  ttfText=${fmt(m.ttfTextMs)}ms  total=${fmt(m.totalMs)}ms  ${cache}\n`,
			);
		}
	}

	if (opts.json) {
		console.log(JSON.stringify({ opts: { ...opts }, runs: all }, null, 2));
		return;
	}

	// Summary table (medians per level).
	console.log(`\n=== medians per level (ms) ===`);
	console.log(
		`level    startup   ttft  ttfText  total   turns  in/out tok   cacheRead  cacheWrite`,
	);
	console.log(`${"-".repeat(86)}`);
	for (const level of opts.levels) {
		const runs = all.filter((r) => r.level === level && !r.error);
		if (runs.length === 0) {
			console.log(`${level.padEnd(8)} (all runs errored)`);
			continue;
		}
		const startup = median(runs.map((r) => r.startupMs));
		const ttft = median(runs.map((r) => r.ttftMs));
		const ttfText = median(runs.map((r) => r.ttfTextMs));
		const total = median(runs.map((r) => r.totalMs));
		const turns = median(runs.map((r) => r.turns));
		const inTok = Math.round(median(runs.map((r) => r.usage.input)) ?? 0);
		const outTok = Math.round(median(runs.map((r) => r.usage.output)) ?? 0);
		const cr = Math.round(median(runs.map((r) => r.usage.cacheRead)) ?? 0);
		const cw = Math.round(median(runs.map((r) => r.usage.cacheWrite)) ?? 0);
		console.log(
			`${level.padEnd(8)} ${fmt(startup)}  ${fmt(ttft)}   ${fmt(ttfText)}  ${fmt(total)}    ${String(turns).padStart(2)}   ${`${inTok}/${outTok}`.padStart(11)}   ${String(cr).padStart(9)}   ${String(cw).padStart(9)}`,
		);
	}

	// Delta read-out: thinking tax + cache benefit.
	const base = opts.levels.includes("off") ? "off" : opts.levels[0];
	const baseRuns = all.filter((r) => r.level === base && !r.error);
	if (baseRuns.length) {
		console.log(`\n=== read-out ===`);
		const baseText = median(baseRuns.map((r) => r.ttfTextMs ?? r.ttftMs));
		for (const level of opts.levels) {
			if (level === base) continue;
			const runs = all.filter((r) => r.level === level && !r.error);
			if (!runs.length) continue;
			const lvlText = median(runs.map((r) => r.ttfTextMs ?? r.ttftMs));
			if (baseText != null && lvlText != null) {
				console.log(
					`  thinking="${level}" adds ~${Math.round(lvlText - baseText)}ms to first visible text vs "${base}" (${Math.round(baseText)} -> ${Math.round(lvlText)}ms)`,
				);
			}
		}
		const cold = baseRuns.find((r) => r.usage.cacheRead === 0);
		const warm = baseRuns.find((r) => r.usage.cacheRead > 0);
		if (cold && warm && cold.ttftMs != null && warm.ttftMs != null) {
			console.log(
				`  cache warm vs cold TTFT: ${Math.round(cold.ttftMs)}ms (cold) -> ${Math.round(warm.ttftMs)}ms (warm), saved ~${Math.round(cold.ttftMs - warm.ttftMs)}ms`,
			);
		}
	}
	console.log();
}

main().catch((err) => {
	console.error(err instanceof Error ? err.message : String(err));
	process.exit(1);
});
