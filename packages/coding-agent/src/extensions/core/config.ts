/**
 * hoo-config.json — types, I/O, and merge rules shared by the hoo-core extensions.
 *
 * Config merge order (lowest → highest priority):
 *   1. ~/.hoocode/hoo-config.json    (global defaults)
 *   2. ./.hoocode/hoo-config.json   (project overrides — scalars win; arrays union)
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { ThinkingLevel } from "@kolisachint/hoocode-agent-core";
import { getHooCodeDir } from "../../config.js";

const HOOCODE_DIR = getHooCodeDir();
const GLOBAL_CONFIG_PATH = join(HOOCODE_DIR, "hoo-config.json");

export interface ModeConfig {
	/** Tool names that bypass the permission gate in this mode */
	auto_allow?: string[];
	/** Tool names available in this mode (if set, only these tools are active) */
	enabled_tools?: string[];
	/** Tool names explicitly blocked in this mode regardless of enabled_tools */
	denied_tools?: string[];
	/** Allowed write paths in this mode (glob patterns, only applies if write/edit is enabled) */
	allowed_write_paths?: string[];
	/** Regex patterns for allowed bash commands. If set, a command must match at least one to execute. */
	allowed_bash_commands?: string[];
	/** Regex patterns for denied bash commands. A command matching any pattern is blocked. */
	denied_bash_commands?: string[];
}

/**
 * Tool-outcome-driven thinking escalation (on by default).
 *
 * The default (fast) path keeps thinking low so mechanical tool turns — reads,
 * greps, successful edits — don't pay the extended-thinking prefill tax. When a
 * tool *fails*, the next turn(s) escalate to a higher thinking level so the model
 * reasons through the failure, then the level is restored. This buys low latency
 * on the happy path while preserving deep reasoning exactly when something breaks.
 *
 * Enabled by default; set `thinking_escalation.enabled` to `false` to turn it off.
 *
 * Note: escalation uses the same setter as manual thinking control, so the
 * escalated level is briefly written to settings and restored when the window
 * ends. If a run is interrupted mid-window, the escalated level may persist until
 * the next change.
 */
export interface ThinkingEscalationConfig {
	/** Master switch. Default: true (set to false to disable). */
	enabled?: boolean;
	/** Level to escalate to after a tool error. Default: "high". */
	on_error?: ThinkingLevel;
	/** Restrict escalation to errors from these tool names. Default: any tool. */
	tools?: string[];
	/** Number of subsequent turns to stay escalated after an error. Default: 1. */
	cooldown_turns?: number;
}

/** LLM defaults seeded in hoo-config.json and honoured during model selection. */
export interface HooLlmConfig {
	/** Preferred provider when the pi-layer settings.json has no saved default. */
	default_provider?: string;
	/** Preferred model id for `default_provider` (otherwise the provider's built-in default). */
	default_model?: string;
	/** Informational map of provider → env var holding its API key. */
	providers?: Record<string, { api_key_env?: string }>;
}

export interface HooConfig {
	/** LLM default provider/model preferences. */
	llm?: HooLlmConfig;
	/** Manually-pinned active mode (overrides default "build") */
	active_mode?: string;
	/** Per-mode configuration keyed by mode name */
	modes?: Record<string, ModeConfig>;
	/** Extra directories to search for `{name}/system.md` mode files (after project + user). */
	mode_paths?: string[];
	/** Raise thinking after tool failures, restore on success. On by default; set `enabled: false` to disable. */
	thinking_escalation?: ThinkingEscalationConfig;
}

export function readConfig(): HooConfig {
	try {
		return JSON.parse(readFileSync(GLOBAL_CONFIG_PATH, "utf8")) as HooConfig;
	} catch {
		return {};
	}
}

export function writeConfig(config: HooConfig): void {
	if (!existsSync(HOOCODE_DIR)) mkdirSync(HOOCODE_DIR, { recursive: true });
	writeFileSync(GLOBAL_CONFIG_PATH, `${JSON.stringify(config, null, 2)}\n`, "utf8");
}

/**
 * Deep-merges a project-local config on top of the global config.
 *
 * Merge rules:
 * - active_mode: project wins if set
 * - modes[x].auto_allow: union of global + project arrays
 * - modes[x].allowed_write_paths: union of global + project arrays
 * - modes[x].enabled_tools: project wins if set, else falls back to global
 * - mode_paths: project list is prepended so project paths are searched first
 * - thinking_escalation: project wins as a whole if set, else inherit global
 */
export function mergeConfigs(global: HooConfig, project: HooConfig): HooConfig {
	const merged: HooConfig = { ...global };

	if (project.active_mode !== undefined) merged.active_mode = project.active_mode;

	// thinking_escalation: project wins as a whole if set, else inherit global.
	if (project.thinking_escalation !== undefined) merged.thinking_escalation = project.thinking_escalation;

	if (project.modes) {
		merged.modes = { ...(global.modes ?? {}) };
		for (const [mode, projectCfg] of Object.entries(project.modes)) {
			const globalCfg = global.modes?.[mode] ?? {};
			merged.modes[mode] = {
				...globalCfg,
				...projectCfg,
				// Union both auto_allow lists so project can extend, not just replace
				auto_allow: Array.from(new Set([...(globalCfg.auto_allow ?? []), ...(projectCfg.auto_allow ?? [])])),
				// Union allowed_write_paths so project can extend
				allowed_write_paths: Array.from(
					new Set([...(globalCfg.allowed_write_paths ?? []), ...(projectCfg.allowed_write_paths ?? [])]),
				),
				// enabled_tools: project wins if set, else falls back to global
				enabled_tools: projectCfg.enabled_tools ?? globalCfg.enabled_tools,
				// denied_tools: union so project can add more denied tools on top of global
				denied_tools: Array.from(new Set([...(globalCfg.denied_tools ?? []), ...(projectCfg.denied_tools ?? [])])),
				// allowed_bash_commands: project wins if set, else falls back to global
				allowed_bash_commands: projectCfg.allowed_bash_commands ?? globalCfg.allowed_bash_commands,
				// denied_bash_commands: union so project can add more denied patterns on top of global
				denied_bash_commands: Array.from(
					new Set([...(globalCfg.denied_bash_commands ?? []), ...(projectCfg.denied_bash_commands ?? [])]),
				),
			};
		}
	}

	if (project.mode_paths || global.mode_paths) {
		// Project paths first so they're searched before global paths
		merged.mode_paths = dedupePaths([...(project.mode_paths ?? []), ...(global.mode_paths ?? [])]);
	}

	return merged;
}

function dedupePaths(paths: string[]): string[] {
	const seen = new Set<string>();
	const out: string[] = [];
	for (const p of paths) {
		if (!seen.has(p)) {
			seen.add(p);
			out.push(p);
		}
	}
	return out;
}

export function mergeSearchPaths(...sources: (string[] | undefined)[]): string[] {
	const merged: string[] = [];
	for (const source of sources) {
		if (!source) continue;
		merged.push(...source);
	}
	return dedupePaths(merged);
}

/**
 * Reads the global config and optionally overlays the project-local config at
 * `./.hoocode/hoo-config.json`. Project values win on all scalar fields; arrays are
 * unioned (see mergeConfigs for full rules).
 */
export function readMergedConfig(cwd: string): HooConfig {
	const global = readConfig();
	const projectPath = join(cwd, ".hoocode", "hoo-config.json");
	if (!existsSync(projectPath)) return global;
	try {
		const project = JSON.parse(readFileSync(projectPath, "utf8")) as HooConfig;
		return mergeConfigs(global, project);
	} catch {
		return global;
	}
}
