/**
 * Assembles an A2A {@link AgentCard} from a HooCode instance's *active*
 * capabilities.
 *
 * The card is built dynamically so it always reflects what the running instance
 * can actually do: the built-in tool bundle that is enabled (bash, edit,
 * browser, …) plus every SKILL.md discovered for the current working directory.
 * Enabling browser tools or dropping a project skill changes the published card
 * with no extra wiring.
 */

import {
	A2A_PROTOCOL_VERSION,
	type AgentCard,
	type AgentProvider,
	type AgentSkill,
	type SecurityScheme,
} from "./types.js";

/** HooCode's built-in tool names, as used by the tool registry. */
export type BuiltinToolName =
	| "read"
	| "bash"
	| "edit"
	| "write"
	| "grep"
	| "find"
	| "ls"
	| "webfetch"
	| "websearch"
	| "browser_run"
	| "browser_continue"
	| "DocRead"
	| "DocEdit"
	| "DocWrite"
	| "DocScan"
	| "DocGrep"
	| "DocPeek";

/**
 * A group of built-in tools that maps to a single advertised A2A skill. A group
 * is advertised when *any* of its tools is active, and the card records which of
 * the group's tools are present so peers can see the concrete surface.
 */
interface ToolSkillGroup {
	id: string;
	name: string;
	description: string;
	tags: string[];
	examples: string[];
	tools: BuiltinToolName[];
}

/**
 * Canonical mapping of HooCode's built-in tools to A2A skills. Ordered so the
 * card reads from the most fundamental capability (shell) outward.
 */
const TOOL_SKILL_GROUPS: ToolSkillGroup[] = [
	{
		id: "shell-execution",
		name: "Shell execution",
		description: "Run shell commands in the workspace through a permission-gated bash tool.",
		tags: ["bash", "shell", "exec"],
		examples: ["Run the test suite", "Show the git status of this repo"],
		tools: ["bash"],
	},
	{
		id: "file-editing",
		name: "File editing",
		description: "Create and modify files with surgical, diff-based edits and full-file writes.",
		tags: ["edit", "write", "files"],
		examples: ["Rename this function across the file", "Add a license header to every source file"],
		tools: ["edit", "write"],
	},
	{
		id: "code-navigation",
		name: "Code search & navigation",
		description: "Read files and search the codebase by name or content to locate and understand code.",
		tags: ["read", "grep", "find", "search"],
		examples: ["Find where this symbol is defined", "Summarize how the auth flow works"],
		tools: ["read", "grep", "find", "ls"],
	},
	{
		id: "web-retrieval",
		name: "Web retrieval",
		description: "Fetch pages and search the web to gather up-to-date context.",
		tags: ["web", "fetch", "search"],
		examples: ["Fetch the changelog from this URL", "Search the web for the latest API docs"],
		tools: ["webfetch", "websearch"],
	},
	{
		id: "browser-automation",
		name: "Browser automation",
		description: "Drive a headless browser to navigate pages and interact with web UIs.",
		tags: ["browser", "automation", "playwright"],
		examples: ["Open the app and check the login page renders", "Click through the checkout flow"],
		tools: ["browser_run", "browser_continue"],
	},
	{
		id: "document-editing",
		name: "Document editing",
		description: "Read and patch structured documents (Office, PDF) with id-based extract-and-patch edits.",
		tags: ["documents", "office", "pdf"],
		examples: ["Update the summary section of this .docx", "Extract the table from this spreadsheet"],
		tools: ["DocRead", "DocEdit", "DocWrite", "DocScan", "DocGrep", "DocPeek"],
	},
];

/** Minimal shape of a discovered SKILL.md needed to advertise it. */
export interface DiscoverableSkill {
	name: string;
	description: string;
}

export interface ResolveActiveToolsOptions {
	/** Web tools (webfetch + websearch) are enabled for this instance. */
	enableWebTools?: boolean;
	/** Browser tools (browser_run + browser_continue) are enabled. */
	enableBrowserTools?: boolean;
	/** Document tools (DocRead/DocEdit/DocWrite + discovery loop) are enabled. */
	enableFileTools?: boolean;
}

/** The core coding bundle every non-restricted HooCode session ships with. */
const DEFAULT_ACTIVE_TOOLS: BuiltinToolName[] = ["read", "bash", "edit", "write", "grep", "find", "ls"];

/**
 * Resolve the set of built-in tools active for an instance from its settings,
 * mirroring how the session's tool factory decides what to register (see
 * `buildSessionOptions` in main.ts). Opt-in bundles are added when enabled.
 */
export function resolveActiveTools(options: ResolveActiveToolsOptions = {}): BuiltinToolName[] {
	const active = [...DEFAULT_ACTIVE_TOOLS];
	if (options.enableWebTools) active.push("webfetch", "websearch");
	if (options.enableBrowserTools) active.push("browser_run", "browser_continue");
	if (options.enableFileTools) active.push("DocRead", "DocEdit", "DocWrite", "DocScan", "DocGrep", "DocPeek");
	return active;
}

export interface BuildAgentCardOptions {
	/** Base URL where this agent is reachable (advertised as the A2A endpoint). */
	url: string;
	/** Built-in tools active for this instance. */
	activeTools: BuiltinToolName[];
	/** SKILL.md files discovered for the current working directory. */
	skills?: DiscoverableSkill[];
	/** HooCode package version, advertised as the agent version. */
	version: string;
	/** Override the agent name (defaults to "HooCode"). */
	name?: string;
	/** Override the agent description. */
	description?: string;
	/** Provider metadata (organization + url). */
	provider?: AgentProvider;
	/** Optional documentation URL. */
	documentationUrl?: string;
}

const DEFAULT_NAME = "HooCode";
const DEFAULT_DESCRIPTION =
	"Deterministic terminal coding agent. Exposes its active tools and skills for A2A discovery.";

/** Slugify a skill name into a stable, spec-valid A2A skill id. */
function skillId(name: string): string {
	const slug = name
		.trim()
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-+|-+$/g, "");
	return `skill-${slug || "unnamed"}`;
}

/** Collapse a skill description to a single line for the card. */
function oneLine(text: string): string {
	return text
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0)
		.join(" ")
		.trim();
}

/**
 * Build the advertised skill list: one skill per active built-in tool group,
 * followed by one skill per discovered SKILL.md. Discovered-skill ids are
 * de-duplicated against the built-in group ids and each other so the card never
 * carries two skills with the same id.
 */
function buildSkills(activeTools: Set<BuiltinToolName>, discovered: DiscoverableSkill[]): AgentSkill[] {
	const skills: AgentSkill[] = [];
	const usedIds = new Set<string>();

	for (const group of TOOL_SKILL_GROUPS) {
		const present = group.tools.filter((tool) => activeTools.has(tool));
		if (present.length === 0) continue;
		skills.push({
			id: group.id,
			name: group.name,
			description: group.description,
			tags: [...group.tags],
			examples: [...group.examples],
			inputModes: ["text/plain"],
			outputModes: ["text/plain"],
		});
		usedIds.add(group.id);
	}

	for (const skill of discovered) {
		let id = skillId(skill.name);
		if (usedIds.has(id)) {
			let suffix = 2;
			while (usedIds.has(`${id}-${suffix}`)) suffix++;
			id = `${id}-${suffix}`;
		}
		usedIds.add(id);
		skills.push({
			id,
			name: skill.name,
			description: oneLine(skill.description),
			tags: ["skill"],
			inputModes: ["text/plain"],
			outputModes: ["text/plain"],
		});
	}

	return skills;
}

/**
 * Build a complete {@link AgentCard} for a HooCode instance.
 *
 * The `capabilities` block advertises the honest state of the discovery-only
 * server: no task streaming, no push notifications. No `securitySchemes` are
 * declared by default because the discovery endpoint itself is unauthenticated
 * — a caller that fronts HooCode behind auth can add schemes to the card.
 */
export function buildAgentCard(options: BuildAgentCardOptions): AgentCard {
	const activeTools = new Set(options.activeTools);
	const skills = buildSkills(activeTools, options.skills ?? []);

	const card: AgentCard = {
		protocolVersion: A2A_PROTOCOL_VERSION,
		name: options.name ?? DEFAULT_NAME,
		description: options.description ?? DEFAULT_DESCRIPTION,
		url: options.url,
		version: options.version,
		capabilities: {
			streaming: false,
			pushNotifications: false,
			stateTransitionHistory: false,
		},
		defaultInputModes: ["text/plain"],
		defaultOutputModes: ["text/plain"],
		skills,
	};

	if (options.provider) card.provider = options.provider;
	if (options.documentationUrl) card.documentationUrl = options.documentationUrl;

	return card;
}

/** Re-export for callers that need the security-scheme shape when extending a card. */
export type { SecurityScheme };
