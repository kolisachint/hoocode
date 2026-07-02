/**
 * Permission gate — prompts before bash/write/edit/webfetch/websearch; checks
 * modes.{mode}.auto_allow from the merged (global + project) config; persists
 * "always" choices back to the global config. Hard enforcement (denied tools,
 * enabled_tools allowlists, bash command patterns, .webtoolsignore hosts)
 * applies even without a UI.
 */

import type {
	ExtensionAPI,
	ExtensionContext,
	ToolCallEvent,
	ToolCallEventResult,
} from "../../core/extensions/types.js";
import { isToolCallEventType } from "../../core/extensions/types.js";
import { blockedHostForUrl } from "../../core/tools/webtools-shared.js";
import { readConfig, readMergedConfig, writeConfig } from "./config.js";

const GATED_TOOLS = new Set(["bash", "write", "edit", "webfetch", "websearch"]);

/**
 * Checks if a file path matches any of the allowed patterns.
 * Supports glob patterns with * and exact paths.
 */
function matchesAllowedPath(filePath: string, allowedPatterns: string[]): boolean {
	if (allowedPatterns.length === 0) return true;
	for (const pattern of allowedPatterns) {
		// Exact match
		if (pattern === filePath) return true;
		// Glob pattern matching for *
		if (pattern.includes("*")) {
			const regex = new RegExp(`^${pattern.replace(/\*/g, ".*")}$`);
			if (regex.test(filePath)) return true;
		}
	}
	return false;
}

/**
 * Tests a bash command string against a regex pattern string.
 * Returns false (no match) if the pattern is an invalid regex.
 */
function matchesBashPattern(pattern: string, command: string): boolean {
	try {
		return new RegExp(pattern).test(command);
	} catch {
		return false;
	}
}

function describeTool(event: ToolCallEvent): string {
	if (isToolCallEventType("bash", event)) {
		return `$ ${event.input.command.replace(/\s+/g, " ").slice(0, 100)}`;
	}
	if (isToolCallEventType("edit", event)) {
		const p = (event.input as { file_path?: string }).file_path ?? "(unknown)";
		return `edit ${p}`;
	}
	if (isToolCallEventType("write", event)) {
		const p = (event.input as { file_path?: string }).file_path ?? "(unknown)";
		return `write ${p}`;
	}
	if (event.toolName === "webfetch") {
		const url = (event.input as { url?: string }).url ?? "(unknown)";
		return `webfetch ${url}`;
	}
	if (event.toolName === "websearch") {
		const query = (event.input as { query?: string }).query ?? "(unknown)";
		return `websearch "${query}"`;
	}
	return event.toolName;
}

export function setupPermissionGate(pi: ExtensionAPI): void {
	pi.on("tool_call", async (event: ToolCallEvent, ctx: ExtensionContext): Promise<ToolCallEventResult | undefined> => {
		// Use the merged config so project-local entries are respected
		const config = readMergedConfig(ctx.cwd);
		const mode = config.active_mode ?? "build";
		const modeCfg = config.modes?.[mode];

		// ── Hard enforcement (always applies, regardless of UI) ───────────────────

		// Explicitly denied tools are blocked unconditionally
		if (modeCfg?.denied_tools?.includes(event.toolName)) {
			return {
				block: true,
				reason: `Tool "${event.toolName}" is denied in mode "${mode}".`,
			};
		}

		// enabled_tools acts as a strict allowlist: only listed tools may execute
		if (
			modeCfg?.enabled_tools &&
			modeCfg.enabled_tools.length > 0 &&
			!modeCfg.enabled_tools.includes(event.toolName)
		) {
			return {
				block: true,
				reason:
					`Tool "${event.toolName}" is not enabled in mode "${mode}" ` +
					`(enabled: ${modeCfg.enabled_tools.join(", ")}).`,
			};
		}

		// Bash command-level filtering
		if (isToolCallEventType("bash", event)) {
			const command = (event.input as { command?: string }).command ?? "";

			// denied_bash_commands: block if any pattern matches
			if (modeCfg?.denied_bash_commands?.length) {
				for (const pattern of modeCfg.denied_bash_commands) {
					if (matchesBashPattern(pattern, command)) {
						return {
							block: true,
							reason: `Bash command matches a denied pattern in mode "${mode}": ${pattern}`,
						};
					}
				}
			}

			// allowed_bash_commands: block unless at least one pattern matches
			if (modeCfg?.allowed_bash_commands?.length) {
				const permitted = modeCfg.allowed_bash_commands.some((p) => matchesBashPattern(p, command));
				if (!permitted) {
					return {
						block: true,
						reason:
							`Bash command is not permitted in mode "${mode}". ` +
							`Allowed patterns: ${modeCfg.allowed_bash_commands.join(", ")}`,
					};
				}
			}
		}

		// webfetch host policy (.webtoolsignore). Hard enforcement, always applies:
		// a blocked host is denied even in headless runs. SSRF/private-address
		// blocking lives in the webtools binary; this is host allow/deny policy only.
		if (event.toolName === "webfetch") {
			const url = (event.input as { url?: string }).url ?? "";
			const blockedHost = url ? blockedHostForUrl(ctx.cwd, url) : undefined;
			if (blockedHost) {
				return {
					block: true,
					reason: `Host "${blockedHost}" is blocked by .webtoolsignore policy.`,
				};
			}
		}

		// ── UI-based permission prompting (interactive sessions only) ─────────────

		if (!GATED_TOOLS.has(event.toolName) || !ctx.hasUI) return;

		const autoAllow = modeCfg?.auto_allow ?? [];

		// Check allowed_write_paths for write/edit operations
		if ((event.toolName === "write" || event.toolName === "edit") && modeCfg?.allowed_write_paths) {
			const filePath = (event.input as { file_path?: string }).file_path ?? "";
			if (!matchesAllowedPath(filePath, modeCfg.allowed_write_paths)) {
				return {
					block: true,
					reason:
						`Mode "${mode}" only allows writes to: ${modeCfg.allowed_write_paths.join(", ")}. ` +
						`Attempted to ${event.toolName}: ${filePath}. ` +
						`Switch to "/mode build" to modify source files.`,
				};
			}
		}

		if (autoAllow.includes(event.toolName)) return;

		const choice = await ctx.ui.select(`Allow: ${describeTool(event)}`, [
			"Yes (once)",
			"No (block)",
			"Always (add to auto-allow for this mode)",
		]);

		if (!choice || choice.startsWith("No")) {
			return { block: true, reason: "Denied by permission gate" };
		}

		if (choice.startsWith("Always")) {
			// Write "always" choices to the global config only
			const latest = readConfig();
			const currentMode = latest.active_mode ?? "build";
			latest.modes ??= {};
			latest.modes[currentMode] ??= {};
			latest.modes[currentMode].auto_allow = Array.from(
				new Set([...(latest.modes[currentMode].auto_allow ?? []), event.toolName]),
			);
			writeConfig(latest);
			ctx.ui.notify(`"${event.toolName}" added to auto-allow for mode "${currentMode}"`, "info");
		}
	});
}
