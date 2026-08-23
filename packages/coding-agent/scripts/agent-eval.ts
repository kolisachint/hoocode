#!/usr/bin/env bun
/**
 * Agent-selection eval.
 *
 * Scores the built-in agent roster against the gold set in
 * `test/fixtures/agent-selection.json`: given each agent's summarized
 * description — exactly the text `<available_agents>` emits — which agent does a
 * model pick for each task, and does it correctly decline to delegate work the
 * parent should keep?
 *
 * The scoring lives in `src/core/agent-selection-eval.ts` and the judge in
 * `src/core/extensions/plugins/trigger-judge.ts`; this file is only the CLI.
 *
 * Usage (from packages/coding-agent):
 *   npm run agent-eval
 *   npm run agent-eval -- --model anthropic/claude-sonnet-5
 *   npm run agent-eval -- --gold path/to/cases.json --out runs/agents.json
 *   npm run agent-eval -- --dry-run     # print the corpus, call no model
 *
 * Runs from source through the root tsconfig's path aliases, so it needs no
 * build. That is why it is `tsx --tsconfig ../../tsconfig.json` rather than
 * `bun` like the search-eval scripts: bun resolves the workspace packages
 * through their `dist` entrypoints and fails until they are built.
 *
 * A run needs a model and credentials. Without them it exits non-zero saying so,
 * rather than reporting a green it did not measure.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { getAgentDir } from "../src/config.js";
import {
	agentCandidates,
	formatAgentSelectionReport,
	loadAgentCases,
	runAgentSelectionEval,
	validateAgentCases,
} from "../src/core/agent-selection-eval.js";
import { AuthStorage } from "../src/core/auth-storage.js";
import { createLlmTriggerJudge } from "../src/core/extensions/plugins/trigger-judge.js";
import { ModelRegistry } from "../src/core/model-registry.js";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(scriptDir, "..");

function flag(name: string): string | undefined {
	const index = process.argv.indexOf(`--${name}`);
	return index !== -1 ? process.argv[index + 1] : undefined;
}
const has = (name: string): boolean => process.argv.includes(`--${name}`);

const goldPath = flag("gold") ?? path.join(packageRoot, "test", "fixtures", "agent-selection.json");
const outPath = flag("out");
const modelRef = flag("model");
const dryRun = has("dry-run");

function die(message: string): never {
	console.error(message);
	process.exit(1);
}

const candidates = agentCandidates(packageRoot);
if (candidates.length === 0) die("No agents in the roster; nothing to evaluate.");

const cases = loadAgentCases(goldPath);
if (!cases) die(`No usable gold set at ${goldPath}.`);

// Checked before spending a model call: an expected agent that does not exist
// scores as a permanent miss and reads like a description problem, which is the
// most expensive way to be wrong about an eval.
const problems = validateAgentCases(candidates, cases);
if (problems.length > 0) die(`Gold set does not match the roster:\n  ${problems.join("\n  ")}`);

console.log(`roster (${candidates.length}):`);
for (const candidate of candidates) {
	console.log(`  ${candidate.name.padEnd(18)} ${candidate.description.slice(0, 100)}`);
}
console.log(`\ngold set: ${cases.length} case(s) from ${path.relative(packageRoot, goldPath)}`);

if (dryRun) {
	const delegating = cases.filter((c) => c.expect !== null).length;
	console.log(`  ${delegating} expect an agent, ${cases.length - delegating} expect no delegation`);
	console.log("\n--dry-run: no model called.");
	process.exit(0);
}

const authStorage = AuthStorage.create(path.join(getAgentDir(), "auth.json"));
const registry = ModelRegistry.create(authStorage, path.join(getAgentDir(), "models.json"));

const slash = modelRef?.indexOf("/") ?? -1;
const model =
	modelRef && slash > 0
		? registry.find(modelRef.slice(0, slash), modelRef.slice(slash + 1))
		: registry.getAvailable()[0];
if (!model) {
	die(
		modelRef
			? `Model "${modelRef}" not found or not available.`
			: "No model available. Authenticate one, or pass --model <provider>/<id>.",
	);
}

const auth = await registry.getApiKeyAndHeaders(model);
if (!auth.ok) die(`Could not authenticate ${model.provider}/${model.id}: ${auth.error}`);

console.log(`judge: ${model.provider}/${model.id}\n`);

const outcome = await runAgentSelectionEval(
	candidates,
	cases,
	createLlmTriggerJudge({ model, apiKey: auth.apiKey, headers: auth.headers }),
);

if (outcome.status === "not-run") die(`Eval did not run: ${outcome.reason}`);

console.log(formatAgentSelectionReport(outcome.report));

if (outPath) {
	const resolved = path.resolve(outPath);
	mkdirSync(path.dirname(resolved), { recursive: true });
	writeFileSync(
		resolved,
		`${JSON.stringify({ report: outcome.report, results: outcome.outcome.record.results }, null, 2)}\n`,
	);
	console.log(`\nwrote ${resolved}`);
}
