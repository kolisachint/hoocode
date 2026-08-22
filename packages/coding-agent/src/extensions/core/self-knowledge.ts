/**
 * SearchHooCode — retrieval over hoocode's own docs and the capabilities this
 * session actually has.
 *
 * The system prompt already lists every shipped doc with a one-line summary
 * (see `core/self-docs.ts`), which answers "which file covers extensions?".
 * What it cannot answer is anything *inside* a file: `extensions.md` alone is
 * over a thousand lines, and reading it whole to find one heading costs more
 * context than the rest of the prompt put together. This indexes docs at the
 * heading level so a question lands on a section and a line number.
 *
 * It also registers the capability kinds the registry has always declared but
 * nobody ever filled — `skill`, `command`, `agent`, `plugin-installed`
 * (registry.ts notes §6.4 left them eager). Those are eager in the prompt, so
 * indexing them is not about reachability; it is about "what can you do?"
 * having one place that answers it rather than the model reciting whichever
 * list happens to be in front of it.
 *
 * `mcp-tool` is deliberately out of scope: ResolveMcpTools owns that kind
 * because finding an MCP tool and making it callable are the same action
 * there, and a second searcher that returns un-resolvable names would be a
 * worse answer, not an extra one.
 */

import type { Static } from "typebox";
import { Type } from "typebox";
import { ensureDenseIndex } from "../../core/capabilities/dense.js";
import { type CapabilityDoc, getCapabilities, registerCapabilities } from "../../core/capabilities/registry.js";
import { resetCapabilitySearch, searchCapabilities } from "../../core/capabilities/search.js";
import type { BeforeAgentStartEvent, ExtensionAPI, ToolDefinition } from "../../core/extensions/types.js";
import { listSelfDocSections, sectionLabel } from "../../core/self-docs.js";

const SEARCH_HOOCODE_TOOL_NAME = "SearchHooCode";

/** Kinds this tool searches. `mcp-tool` belongs to ResolveMcpTools. */
const SEARCHABLE = ["doc", "skill", "command", "agent", "plugin-installed"] as const;

const DEFAULT_LIMIT = 8;
const MAX_LIMIT = 25;

/** Registered once per process; the docs are read-only install content. */
let docsRegistered = false;

/**
 * Index the shipped docs, the first time anyone asks.
 *
 * Lazy because it reads thirty files and produces roughly a thousand sections.
 * A session that never asks hoocode about itself should not pay for that at
 * startup, and the cost is invisible once paid — `listSelfDocSections` caches.
 */
function ensureDocsRegistered(): void {
	if (docsRegistered) return;
	docsRegistered = true;

	const sections = listSelfDocSections();
	if (sections.length === 0) return;

	registerCapabilities(
		"doc",
		sections.map(
			(section): CapabilityDoc => ({
				id: `doc:${section.id}`,
				kind: "doc",
				name: sectionLabel(section),
				description: section.excerpt,
				source: section.path,
				// The body is withheld until the model reads the file, which is
				// exactly the condition this flag records.
				deferred: true,
			}),
		),
	);
	resetCapabilitySearch();
}

/** Line-number lookup for rendering a hit, keyed by the id we registered under. */
function sectionIndex(): Map<string, { path: string; line: number }> {
	const index = new Map<string, { path: string; line: number }>();
	for (const section of listSelfDocSections()) {
		index.set(`doc:${section.id}`, { path: section.path, line: section.line });
	}
	return index;
}

/**
 * Mirror the session's skills, commands, and subagents into the registry.
 *
 * Driven from `before_agent_start` because that is where the assembled
 * `systemPromptOptions` carries the skills and agents the session actually
 * loaded — re-discovering them from disk here would risk disagreeing with what
 * the model was told. Guarded by a signature so a steady session registers once
 * instead of dropping the lexical index on every turn.
 */
let lastSignature = "";

function registerSessionCapabilities(event: BeforeAgentStartEvent, pi: ExtensionAPI): void {
	const skills = event.systemPromptOptions.skills ?? [];
	const agents = event.systemPromptOptions.agents ?? [];
	let commands: Array<{ name: string; description?: string }> = [];
	try {
		commands = pi.getCommands();
	} catch {
		// Commands are an interactive-mode concern; a headless session may not
		// have them. Missing commands is not a reason to skip skills and agents.
		commands = [];
	}

	const signature = [
		skills.map((s) => s.name).join(","),
		agents.map((a) => a.name).join(","),
		commands.map((c) => c.name).join(","),
	].join("|");
	if (signature === lastSignature) return;
	lastSignature = signature;

	registerCapabilities(
		"skill",
		skills.map(
			(skill): CapabilityDoc => ({
				id: `skill:${skill.name}`,
				kind: "skill",
				name: skill.name,
				description: skill.description ?? "",
				source: skill.filePath,
				// Eager: the prompt already lists these, so a hit is a convenience.
				deferred: false,
			}),
		),
	);
	registerCapabilities(
		"agent",
		agents.map(
			(agent): CapabilityDoc => ({
				id: `agent:${agent.name}`,
				kind: "agent",
				name: agent.name,
				description: agent.description ?? "",
				deferred: false,
			}),
		),
	);
	registerCapabilities(
		"command",
		commands.map(
			(command): CapabilityDoc => ({
				id: `command:${command.name}`,
				kind: "command",
				name: `/${command.name}`,
				description: command.description ?? "",
				deferred: false,
			}),
		),
	);
	resetCapabilitySearch();
}

function describeHit(doc: CapabilityDoc, sections: Map<string, { path: string; line: number }>): string {
	if (doc.kind === "doc") {
		const where = sections.get(doc.id);
		const location = where ? `${where.path}:${where.line}` : (doc.source ?? "");
		return `- [doc] ${doc.name}\n  read ${location}\n  ${doc.description}`;
	}
	const summary = doc.description ? ` — ${doc.description}` : "";
	return `- [${doc.kind}] ${doc.name}${summary}`;
}

export function setupSelfKnowledge(pi: ExtensionAPI): void {
	pi.on("before_agent_start", (event: BeforeAgentStartEvent) => {
		registerSessionCapabilities(event, pi);
	});

	const params = Type.Object(
		{
			query: Type.String({
				description:
					"What you want to know about hoocode, in your own words — 'how do I write an extension', " +
					"'where are sessions stored', 'can it run subagents'.",
			}),
			limit: Type.Optional(Type.Number({ description: `Maximum results. Default ${DEFAULT_LIMIT}.` })),
		},
		{ additionalProperties: false },
	);

	pi.registerTool({
		name: SEARCH_HOOCODE_TOOL_NAME,
		label: SEARCH_HOOCODE_TOOL_NAME,
		description:
			"Search hoocode's own documentation and the capabilities loaded in this session (skills, slash " +
			"commands, subagents, installed plugins). Use it when the user asks what hoocode can do, how one of " +
			"its features works, or how to configure or extend it. Doc results come back as a file path and line " +
			"number — read that range for the answer rather than replying from the excerpt alone. " +
			"MCP tools are not covered here; find those with ResolveMcpTools.",
		promptSnippet: "Search hoocode's own docs and this session's capabilities by describing what you need.",
		promptGuidelines: [
			"For questions about hoocode itself — its features, configuration, or how to extend it — use SearchHooCode and read the section it points at instead of answering from memory.",
		],
		parameters: params,
		executionMode: "parallel",
		async execute(_toolCallId: string, args: Static<typeof params>) {
			const query = args.query?.trim();
			if (!query) {
				return { content: [{ type: "text" as const, text: "Provide a query." }], details: undefined };
			}

			ensureDocsRegistered();
			const limit = Math.min(Math.max(1, Math.trunc(args.limit ?? DEFAULT_LIMIT)), MAX_LIMIT);

			// Fire-and-forget over everything registered, not just the docs: the
			// dense store is shared, so indexing a subset here would evict whatever
			// the MCP resolver indexed. The lexical leg answers this call either way.
			void ensureDenseIndex(getCapabilities()).catch(() => {});

			const { hits, legs } = await searchCapabilities(query, { kinds: [...SEARCHABLE], limit });
			if (hits.length === 0) {
				return {
					content: [
						{
							type: "text" as const,
							text: `Nothing matched "${query}". The docs are listed in the system prompt under "About hoocode itself" — read the most likely file directly.`,
						},
					],
					details: undefined,
				};
			}

			const sections = sectionIndex();
			const body = hits.map((hit) => describeHit(hit.doc, sections)).join("\n");
			// Say which legs answered: lexical-only on a conceptual query is a
			// weaker result, and the caller should be able to see that.
			const note = legs.includes("dense") ? "" : "\n(Lexical match only — try naming the feature if this missed.)";
			return {
				content: [{ type: "text" as const, text: `Results for "${query}":\n${body}${note}` }],
				details: undefined,
			};
		},
	} as ToolDefinition);
}

/** Tests, and anything that relocates the package root mid-process. */
export function resetSelfKnowledge(): void {
	docsRegistered = false;
	lastSignature = "";
}
