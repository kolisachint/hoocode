/**
 * Demo (vs Claude Code #1): Provider freedom — cross-provider handoff
 * -------------------------------------------------------------------
 * Claude Code is single-vendor by design: every turn goes to Anthropic.
 * HooCode's unified `ai` layer lets ONE conversation span multiple providers —
 * the message history (including tool results) from provider A is handed to
 * provider B, which continues seamlessly.
 *
 * This drives the REAL agent runtime (`Agent` from hoocode-agent-core). The only
 * thing swapped out is the network: two faux providers stand in for "anthropic"
 * and "openai" so the demo runs offline and deterministically. Nothing about the
 * agent loop, tool execution, or message handling is mocked.
 *
 * Run:  npx tsx packages/coding-agent/demo/provider-handoff.ts
 */

import { Agent, type AgentTool } from "@kolisachint/hoocode-agent-core";
import { fauxAssistantMessage, fauxText, fauxToolCall, registerFauxProvider } from "@kolisachint/hoocode-ai";
import chalk from "chalk";
import { type Static, Type } from "typebox";

// A real AgentTool — the agent actually executes this between provider turns.
const searchSchema = Type.Object({ query: Type.String() });
const webSearch: AgentTool<typeof searchSchema, undefined> = {
	name: "web_search",
	label: "Web Search",
	description: "Search the web for a query",
	parameters: searchSchema,
	execute: async (_id: string, args: Static<typeof searchSchema>) => ({
		content: [
			{
				type: "text",
				text: `Top result for "${args.query}": HooCode gates every edit/command behind Yes/No/Always.`,
			},
		],
		details: undefined,
	}),
};

// Two providers, distinct vendors. `api` auto-generates a unique id, so neither
// collides with the real built-in registrations.
const anthropic = registerFauxProvider({ provider: "anthropic", models: [{ id: "claude-sonnet-4-6" }] });
const openai = registerFauxProvider({ provider: "openai", models: [{ id: "gpt-5" }] });

// Turn 1 runs on Anthropic: it calls the search tool, then drafts an answer.
anthropic.setResponses([
	fauxAssistantMessage(
		[
			fauxText("Let me look that up."),
			fauxToolCall("web_search", { query: "HooCode permission model" }, { id: "s1" }),
		],
		{ stopReason: "toolUse" },
	),
	fauxAssistantMessage("Draft: HooCode asks before every edit and command."),
]);

// Turn 2 runs on OpenAI: it sees the full prior history (incl. the tool result)
// and refines the draft.
openai.setResponses([
	fauxAssistantMessage(
		"Refined: Every edit and shell command in HooCode passes a Yes/No/Always gate you control — deterministic by design.",
	),
]);

const agent = new Agent({
	initialState: {
		systemPrompt: "You are a research assistant. Use web_search when helpful.",
		model: anthropic.getModel(),
		thinkingLevel: "off",
		tools: [webSearch],
	},
});

const label = (provider: string, model: string) => chalk.dim(`[${chalk.bold(provider)}/${model}]`);

async function main() {
	console.log(chalk.bold.cyan("\nHooCode · cross-provider handoff (one conversation, two vendors)\n"));

	// Turn 1 — Anthropic
	await agent.prompt("Research how HooCode handles permissions, then draft a one-liner.");

	// Hand off mid-conversation: same agent, same history, different provider.
	agent.state.model = openai.getModel();
	console.log(chalk.yellow("\n— handing the conversation to a different provider —\n"));

	// Turn 2 — OpenAI, continuing from Anthropic's messages + tool result
	await agent.prompt("Now sharpen that into a crisp marketing one-liner.");

	// Print the transcript, labeling which provider produced each assistant turn.
	const textOf = (content: unknown): string =>
		typeof content === "string"
			? content
			: Array.isArray(content)
				? content
						.filter((c) => c?.type === "text")
						.map((c) => c.text)
						.join(" ")
				: "";
	console.log(chalk.bold("Transcript:"));
	for (const m of agent.state.messages) {
		if (m.role === "user") {
			console.log(`  ${chalk.green("user")}  ${textOf(m.content)}`);
		} else if (m.role === "assistant") {
			const text = m.content
				.filter((c) => c.type === "text")
				.map((c) => (c as { text: string }).text)
				.join(" ");
			const calls = m.content.filter((c) => c.type === "toolCall").map((c) => (c as { name: string }).name);
			const suffix = calls.length ? chalk.magenta(`  ⟶ calls: ${calls.join(", ")}`) : "";
			if (text || suffix) console.log(`  ${label(m.provider, m.model)} ${text}${suffix}`);
		} else if (m.role === "toolResult") {
			console.log(`  ${chalk.blue("tool")}  ${m.content.map((c) => (c.type === "text" ? c.text : "")).join("")}`);
		}
	}

	const providers = new Set(agent.state.messages.filter((m) => m.role === "assistant").map((m) => m.provider));
	console.log(
		chalk.bold.cyan(`\n→ ${providers.size} providers in a single conversation: ${[...providers].join(" + ")}.`),
	);
	console.log(chalk.dim("  Claude Code can't leave Anthropic; HooCode treats the provider as a swappable detail.\n"));

	anthropic.unregister();
	openai.unregister();
	process.exit(0);
}

main();
