/**
 * Agent registry: data-driven loading of subagent definitions.
 *
 * Replaces the hardcoded `SubagentMode` enum + `MODE_TOOLS` map with frontmatter
 * `.md` files (see agent-frontmatter.ts). Definitions are discovered from, in
 * increasing order of precedence:
 *
 *   1. builtin            embedded templates (EMBEDDED_AGENT_PROMPTS)
 *   2. claude-user        ~/.claude/agents/*.md          (D7 native import)
 *   3. user               ~/.hoocode/agent/agents/*.md
 *   4. claude-project     <cwd>/.claude/agents/*.md       (D7 native import)
 *   5. project            <cwd>/.hoocode/agents/*.md
 *
 * Higher-precedence sources override lower ones by name. Overrides are recorded
 * as collision diagnostics. Loading never throws; problems surface as
 * diagnostics, matching skills.ts.
 */

import { existsSync, readdirSync, readFileSync, statSync } from "fs";
import { homedir } from "os";
import { join, resolve } from "path";
import { CONFIG_DIR_NAME, getAgentDir } from "../config.js";
import { EMBEDDED_AGENT_PROMPTS } from "../init-templates.generated.js";
import { getAgentManifestPaths } from "./agent-manifest-paths.js";
import { type AgentDefinition, type AgentSource, parseAgentDefinition } from "./agent-frontmatter.js";
import type { ResourceDiagnostic } from "./diagnostics.js";

/** Registry of agent definitions keyed by name. */
export class AgentRegistry {
	private agents = new Map<string, AgentDefinition>();
	private diagnostics: ResourceDiagnostic[] = [];

	/**
	 * Add or override a definition. Later registrations win (used both by the
	 * loader for precedence and as an escape hatch for programmatic agents).
	 * Overriding an existing name records a collision diagnostic.
	 */
	register(def: AgentDefinition): void {
		const existing = this.agents.get(def.name);
		if (existing) {
			this.diagnostics.push({
				type: "collision",
				message: `agent "${def.name}" from ${def.source} overrides ${existing.source}`,
				path: def.filePath,
				collision: {
					resourceType: "skill",
					name: def.name,
					winnerPath: def.filePath ?? `<${def.source}>`,
					loserPath: existing.filePath ?? `<${existing.source}>`,
					winnerSource: def.source,
					loserSource: existing.source,
				},
			});
		}
		this.agents.set(def.name, def);
	}

	get(name: string): AgentDefinition | undefined {
		return this.agents.get(name);
	}

	has(name: string): boolean {
		return this.agents.has(name);
	}

	list(): AgentDefinition[] {
		return Array.from(this.agents.values());
	}

	/** Diagnostics accumulated during loading/registration. */
	getDiagnostics(): ResourceDiagnostic[] {
		return this.diagnostics;
	}

	/** Append externally-produced diagnostics (e.g. from a parse step). */
	addDiagnostics(diagnostics: ResourceDiagnostic[]): void {
		this.diagnostics.push(...diagnostics);
	}
}

/** Load and register every built-in (embedded) agent definition. */
function registerBuiltins(registry: AgentRegistry): void {
	for (const [key, raw] of Object.entries(EMBEDDED_AGENT_PROMPTS)) {
		const { agent, diagnostics } = parseAgentDefinition(raw, { source: "builtin", fallbackName: key });
		registry.addDiagnostics(diagnostics);
		if (agent) registry.register(agent);
	}
}

/** Load flat `*.md` agent files from a directory. Non-`.md` entries and
 *  subdirectories are skipped (so runtime dispatch dirs are ignored). */
function registerDir(registry: AgentRegistry, dir: string, source: AgentSource): void {
	if (!existsSync(dir)) return;
	let entries: string[];
	try {
		entries = readdirSync(dir);
	} catch {
		return;
	}
	for (const entry of entries) {
		if (entry.startsWith(".") || !entry.endsWith(".md")) continue;
		const filePath = join(dir, entry);
		try {
			if (!statSync(filePath).isFile()) continue;
			const raw = readFileSync(filePath, "utf-8");
			const { agent, diagnostics } = parseAgentDefinition(raw, { source, filePath });
			registry.addDiagnostics(diagnostics);
			if (agent) registry.register(agent);
		} catch (error) {
			const message = error instanceof Error ? error.message : "failed to read agent file";
			registry.addDiagnostics([{ type: "warning", message, path: filePath }]);
		}
	}
}

export interface LoadAgentRegistryOptions {
	/** Working directory for project-local agents. */
	cwd: string;
	/** User agent config directory (parent of `agents/`). Defaults to getAgentDir(). */
	agentDir?: string;
	/** Include embedded built-in agents. Defaults to true. */
	includeBuiltins?: boolean;
	/** Discover `.claude/agents/` directories for native Claude Code import (D7). Defaults to true. */
	includeClaude?: boolean;
}

/**
 * Build an AgentRegistry from all configured locations, applying precedence.
 * Agent files discovered from package manifests (via `hoocode.agents` in
 * package.json) are automatically included from the module-level store set by
 * DefaultResourceLoader.reload().
 */
export function loadAgentRegistry(options: LoadAgentRegistryOptions): AgentRegistry {
	const { cwd, includeBuiltins = true, includeClaude = true } = options;
	const userAgentDir = options.agentDir ?? getAgentDir();
	const registry = new AgentRegistry();

	// Lowest precedence first; later sources override earlier ones by name.
	if (includeBuiltins) {
		registerBuiltins(registry);
	}

	// Package-manifest agents (declared via `hoocode.agents` in package.json).
	for (const filePath of getAgentManifestPaths()) {
		if (!existsSync(filePath)) continue;
		try {
			if (!statSync(filePath).isFile()) continue;
			const raw = readFileSync(filePath, "utf-8");
			const { agent, diagnostics } = parseAgentDefinition(raw, { source: "user", filePath });
			registry.addDiagnostics(diagnostics);
			if (agent) registry.register(agent);
		} catch (error) {
			const message = error instanceof Error ? error.message : "failed to read agent file";
			registry.addDiagnostics([{ type: "warning", message, path: filePath }]);
		}
	}

	if (includeClaude) {
		registerDir(registry, join(homedir(), ".claude", "agents"), "claude-user");
	}
	registerDir(registry, join(userAgentDir, "agents"), "user");
	if (includeClaude) {
		registerDir(registry, resolve(cwd, ".claude", "agents"), "claude-project");
	}
	registerDir(registry, resolve(cwd, CONFIG_DIR_NAME, "agents"), "project");

	return registry;
}

/**
 * Format a list of agent definitions as an XML block for inclusion in a system
 * prompt, mirroring the `<available_skills>` format used by formatSkillsForPrompt.
 * Only intended for display when the Task tool is active.
 */
export function formatAgentsForPrompt(agents: AgentDefinition[]): string {
	if (agents.length === 0) return "";

	const lines = [
		"\n\nThe following specialized agents are available for delegation via the Task tool.",
		"Choose the agent whose description best matches the task and pass it as `subagent_type`.",
		"",
		"<available_agents>",
	];

	for (const agent of agents) {
		lines.push("  <agent>");
		lines.push(`    <name>${escapeXml(agent.name)}</name>`);
		lines.push(`    <description>${escapeXml(agent.description)}</description>`);
		if (agent.tools && agent.tools.length > 0) {
			lines.push(`    <tools>${escapeXml(agent.tools.join(", "))}</tools>`);
		}
		if (agent.model) {
			lines.push(`    <model>${escapeXml(agent.model)}</model>`);
		}
		lines.push("  </agent>");
	}

	lines.push("</available_agents>");

	return lines.join("\n");
}

function escapeXml(str: string): string {
	return str
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;")
		.replace(/"/g, "&quot;")
		.replace(/'/g, "&apos;");
}
