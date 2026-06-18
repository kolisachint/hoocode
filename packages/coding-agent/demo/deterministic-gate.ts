/**
 * Demo (vs Claude Code #2): Deterministic, mode-scoped execution + permission gate
 * --------------------------------------------------------------------------------
 * Most agents act first and tell you later. HooCode is deterministic: every tool
 * call passes a permission gate driven by an explicit, merged mode policy, and a
 * destructive command is BLOCKED rather than run-then-reported.
 *
 * This wires three REAL pieces together:
 *   - `mergeConfigs`     — resolves the build-mode tool policy (global ⊕ project)
 *   - `buildSystemPrompt`— loads the mode's scoped system prompt
 *   - `Agent.beforeToolCall` — the genuine runtime hook the gate plugs into;
 *     returning `{ block: true }` stops the tool and the loop emits an error result.
 *
 * The provider is faux (offline/deterministic) and scripts an assistant turn that
 * tries three tools: a safe read, an edit, and `rm -rf`. The gate auto-allows the
 * read, prompts→approves the edit, and denies the destructive bash.
 *
 * Run:  npx tsx packages/coding-agent/demo/deterministic-gate.ts
 */

import {
	Agent,
	type AgentTool,
	type BeforeToolCallContext,
	type BeforeToolCallResult,
} from "@kolisachint/hoocode-agent-core";
import { fauxAssistantMessage, fauxText, fauxToolCall, registerFauxProvider } from "@kolisachint/hoocode-ai";
import chalk from "chalk";
import { type TSchema, Type } from "typebox";
import { buildSystemPrompt, type HooConfig, mergeConfigs } from "../src/extensions/core/hoo-core.js";

// Minimal stand-in tools (the real read/edit/bash live in the full runtime).
// Their execute bodies are inert — bash never actually runs anything.
const mk = (name: string, schema: TSchema): AgentTool => ({
	name,
	label: name,
	description: `${name} tool`,
	parameters: schema,
	execute: async (_id, args) => ({
		content: [{ type: "text", text: `${name}(${JSON.stringify(args)}) ran` }],
		details: undefined,
	}),
});
const tools = [
	mk("read", Type.Object({ path: Type.String() })),
	mk("edit", Type.Object({ path: Type.String(), change: Type.String() })),
	mk("bash", Type.Object({ command: Type.String() })),
];

// REAL config resolution: build-mode policy = global ⊕ project (project wins on
// scalars, arrays union). This is the exact merge the agent uses at runtime.
const global: HooConfig = {
	modes: { build: { auto_allow: ["read", "ls", "grep"], denied_bash_commands: ["rm\\s+-rf"] } },
};
const project: HooConfig = { modes: { build: { allowed_write_paths: ["src/**"] } } };
const policy = mergeConfigs(global, project).modes?.build ?? {};

const remembered = new Set<string>();

// The permission gate, plugged into the real Agent.beforeToolCall hook.
const gate = async (ctx: BeforeToolCallContext): Promise<BeforeToolCallResult | undefined> => {
	const name = ctx.toolCall.name;
	const args = ctx.args as Record<string, unknown>;

	if (policy.auto_allow?.includes(name) || remembered.has(name)) {
		console.log(`  ${chalk.green("ALLOW")}  ${name}  ${chalk.dim("(auto-allow policy)")}`);
		return undefined;
	}
	if (name === "bash") {
		const cmd = String(args.command ?? "");
		if (policy.denied_bash_commands?.some((re) => new RegExp(re).test(cmd))) {
			console.log(`  ${chalk.red("DENY")}   ${name}  ${chalk.dim(`"${cmd}" matches denied_bash_commands`)}`);
			return { block: true, reason: "Blocked by permission gate: matches denied_bash_commands." };
		}
	}
	// Anything else → prompt the user. We simulate "Always" (Yes + remember).
	remembered.add(name);
	console.log(`  ${chalk.yellow("PROMPT")} ${name}  ${chalk.dim("→ user chose Always (remembered)")}`);
	return undefined;
};

const faux = registerFauxProvider({ provider: "anthropic", models: [{ id: "claude-sonnet-4-6" }] });
faux.setResponses([
	fauxAssistantMessage(
		[
			fauxText("I'll inspect, edit, then clean up."),
			fauxToolCall("read", { path: "README.md" }, { id: "t1" }),
			fauxToolCall("edit", { path: "src/app.ts", change: "fix bug" }, { id: "t2" }),
			fauxToolCall("bash", { command: "rm -rf ~/project" }, { id: "t3" }),
		],
		{ stopReason: "toolUse" },
	),
	fauxAssistantMessage("Done — the destructive command was blocked by the gate."),
]);

async function main() {
	console.log(chalk.bold.cyan("\nHooCode · deterministic, mode-scoped permission gate\n"));

	const prompt = buildSystemPrompt("build", process.cwd());
	console.log(chalk.bold("Active mode:"), "build");
	console.log(chalk.dim("  scoped prompt: ") + (prompt?.split("\n")[0] ?? "(default)"));
	console.log(chalk.bold("Resolved policy (global ⊕ project):"));
	console.log(
		chalk.dim(
			`  auto_allow=${JSON.stringify(policy.auto_allow)} denied_bash=${JSON.stringify(policy.denied_bash_commands)}\n`,
		),
	);

	console.log(chalk.bold("Gate decisions:"));
	const agent = new Agent({
		initialState: { systemPrompt: prompt ?? "", model: faux.getModel(), thinkingLevel: "off", tools },
		beforeToolCall: gate,
	});
	await agent.prompt("Inspect, apply a fix, then clean up the workspace.");

	const blocked = agent.state.messages.some(
		(m) =>
			m.role === "toolResult" &&
			m.isError &&
			/gate/i.test(m.content.map((c) => (c.type === "text" ? c.text : "")).join("")),
	);
	console.log(
		chalk.bold.cyan(
			`\n→ Same script, same decisions, every run. Destructive bash ${blocked ? "blocked" : "NOT blocked"}.`,
		),
	);
	console.log(chalk.dim("  The agent kept going after the block — it didn't act first and apologize later.\n"));

	faux.unregister();
	process.exit(0);
}

main();
