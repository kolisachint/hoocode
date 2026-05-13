/**
 * hoo-core — HooCode built-in core extension
 *
 * A. Permission Gate    — prompts before bash/write/edit; checks modes.{mode}.auto_allow
 *                         from the merged (global + project) config; persists "always"
 *                         choices back to the global config
 * B. MCP Server Loader  — discovers ~/.hoocode/mcp-servers and ./.hoocode/mcp-servers JSON
 *                         configs, connects via JSON-RPC 2.0, registers server tools
 * C. Mode + Profile     — resolves active mode (ask/plan/build/agent/debug) and profile
 *                         (default/data/devops/…), merges system prompt from three template
 *                         layers, filters active tools, and exposes /mode, /profile,
 *                         /plan, and /approve commands
 *
 * Config merge order (lowest → highest priority):
 *   1. ~/.hoocode/agent/hoo-config.json   (global defaults)
 *   2. ./.hoocode/config.json             (project overrides — scalars win; arrays union)
 *   3. profile_detectors from project prepend global list (project markers checked first)
 */
import type { ExtensionAPI } from "../../core/extensions/types.js";
interface ProfileDetector {
    marker: string;
    profile: string;
}
interface ModeConfig {
    /** Tool names that bypass the permission gate in this mode */
    auto_allow?: string[];
    /** Tool names available in this mode (if set, only these tools are active) */
    enabled_tools?: string[];
    /** Allowed write paths in this mode (glob patterns, only applies if write/edit is enabled) */
    allowed_write_paths?: string[];
}
interface ProfileConfig {
    /** Tool names to activate for this profile */
    enabled_tools?: string[];
}
export interface HooConfig {
    /** Manually-pinned active mode (overrides default "build") */
    active_mode?: string;
    /** Manually-pinned active profile (overrides auto-detection) */
    active_profile?: string;
    /** Per-mode configuration keyed by mode name */
    modes?: Record<string, ModeConfig>;
    /** Per-profile configuration keyed by profile name */
    profiles?: Record<string, ProfileConfig>;
    /** Ordered list of file-marker → profile mappings for auto-detection */
    profile_detectors?: ProfileDetector[];
    /** Extra directories to search for `{name}/system.md` mode files (after project + user). */
    mode_paths?: string[];
    /** Extra directories to search for `{name}/context.md` profile files (after project + user). */
    profile_paths?: string[];
}
/**
 * Deep-merges a project-local config on top of the global config.
 *
 * Merge rules:
 * - Scalars (active_mode, active_profile): project wins if set
 * - modes[x].auto_allow: union of global + project arrays
 * - modes[x].allowed_write_paths: union of global + project arrays
 * - modes[x].enabled_tools: project wins if set, else falls back to global
 * - profiles[x].enabled_tools: project wins if set, else falls back to global
 * - profile_detectors: project list is prepended so project markers are checked first
 */
export declare function mergeConfigs(global: HooConfig, project: HooConfig): HooConfig;
/**
 * Reads the global config and optionally overlays the project-local config at
 * `./.hoocode/config.json`. Project values win on all scalar fields; arrays are
 * unioned (see mergeConfigs for full rules).
 */
export declare function readMergedConfig(cwd: string): HooConfig;
export declare function setupPermissionGate(pi: ExtensionAPI): void;
export interface McpServerConfig {
    /** Unique server identifier used as prefix for registered tool names */
    name: string;
    /** Executable to spawn */
    command: string;
    /** Optional arguments passed to the command */
    args?: string[];
    /** Optional extra environment variables for the server process */
    env?: Record<string, string>;
}
export declare function setupMcpLoader(pi: ExtensionAPI): void;
/**
 * Resolves which profile should be active.
 * Priority: config override → file-marker detection → "default"
 */
export declare function resolveProfile(config: HooConfig, cwd: string): string;
/**
 * Merges the system prompt from up to three layers (lowest → highest priority):
 *   1. {project|user|external}/modes/{mode}/system.md        (mode behaviour)
 *   2. {project|user|external}/profiles/{profile}/context.md (domain context; skipped for "default")
 *   3. ./.hoocode/agents.md                                  (project-local override; appended last)
 *
 * For each of layers 1 and 2 the search order is:
 *   - `./.hoocode/{modes,profiles}/{name}/...`
 *   - `~/.hoocode/{modes,profiles}/{name}/...`
 *   - each of `externalDirs` in declared order (config + CLI + extension contributions)
 *
 * Each present layer is joined with a `---` separator.
 */
export declare function buildSystemPrompt(mode: string, profile: string, cwd: string, options?: {
    modePaths?: string[];
    profilePaths?: string[];
}): string | undefined;
export interface PlanSections {
    goal?: string;
    filesToModify?: string;
    newFiles?: string;
    tests?: string;
    verification?: string;
    /** Original full text, used as fallback if no sections parsed */
    raw: string;
}
/**
 * Parses `.hoocode/plan.md` into named sections.
 *
 * Recognises both ATX headings (`## Goal`) and bold labels (`**Goal**`).
 * Section names matched (case-insensitive): Goal, Files to modify, New files,
 * Tests, Verification.
 */
export declare function parsePlanSections(planContent: string): PlanSections;
/**
 * Builds the user message sent to the agent when `/approve` is run.
 *
 * If the plan has recognisable sections, each is presented as a numbered step
 * so the agent works through them sequentially. Otherwise the raw plan is used.
 *
 * Execution order:
 *   1. Modify existing files
 *   2. Create new files
 *   3. Update / add tests
 *   4. Run verification commands
 */
export declare function buildApproveMessage(sections: PlanSections): string;
export declare function setupModeAndProfile(pi: ExtensionAPI): void;
export default function hooCore(pi: ExtensionAPI): void;
export {};
//# sourceMappingURL=hoo-core.d.ts.map