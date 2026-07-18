// Core coding tools.
export {
	type BashOperations,
	type BashSpawnContext,
	type BashSpawnHook,
	type BashToolDetails,
	type BashToolInput,
	type BashToolOptions,
	createBashTool,
	createBashToolDefinition,
	createLocalBashOperations,
} from "./bash.js";
// Optional feature tools, grouped by feature (off by default or enabled per session).
export * from "./browser/index.js";
export * from "./doc/index.js";
export {
	createEditTool,
	createEditToolDefinition,
	type EditOperations,
	type EditToolDetails,
	type EditToolInput,
	type EditToolOptions,
} from "./edit.js";
export { withFileMutationQueue } from "./file-mutation-queue.js";
export {
	createFindTool,
	createFindToolDefinition,
	type FindOperations,
	type FindToolDetails,
	type FindToolInput,
	type FindToolOptions,
} from "./find.js";
export {
	createGrepTool,
	createGrepToolDefinition,
	type GrepOperations,
	type GrepToolDetails,
	type GrepToolInput,
	type GrepToolOptions,
} from "./grep.js";
export {
	createLsTool,
	createLsToolDefinition,
	type LsOperations,
	type LsToolDetails,
	type LsToolInput,
	type LsToolOptions,
} from "./ls.js";
export { expandPath, resolveReadPath, resolveToCwd } from "./path-utils.js";
export {
	createReadTool,
	createReadToolDefinition,
	type ReadOperations,
	type ReadToolDetails,
	type ReadToolInput,
	type ReadToolOptions,
} from "./read.js";
// Off-by-default tools (enabled per session via flags/settings).
export {
	createSearchTool,
	createSearchToolDefinition,
	type SearchToolDetails,
	type SearchToolInput,
	type SearchToolOptions,
} from "./search.js";
export {
	buildTaskMainPrompt,
	createTaskOutputToolDefinition,
	createTaskToolDefinition,
	type TaskOutputDetails,
	type TaskToolDetails,
} from "./subagent.js";
export { createTodoWriteToolDefinition, type TodoWriteDetails } from "./todo.js";
export {
	DEFAULT_MAX_BYTES,
	DEFAULT_MAX_LINES,
	formatSize,
	type TruncationOptions,
	type TruncationResult,
	truncateHead,
	truncateLine,
	truncateTail,
} from "./truncate.js";
export {
	createWebFetchTool,
	createWebFetchToolDefinition,
	type WebFetchToolDetails,
	type WebFetchToolInput,
	type WebFetchToolOptions,
} from "./webfetch.js";
export {
	createWebSearchTool,
	createWebSearchToolDefinition,
	type WebSearchToolDetails,
	type WebSearchToolInput,
	type WebSearchToolOptions,
} from "./websearch.js";
export {
	createWriteTool,
	createWriteToolDefinition,
	type WriteOperations,
	type WriteToolInput,
	type WriteToolOptions,
} from "./write.js";

import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import type { ToolDefinition } from "../extensions/types.js";
import { type BashToolOptions, createBashToolDefinition } from "./bash.js";
import {
	type BrowserContinueToolOptions,
	type BrowserRunToolOptions,
	createBrowserContinueToolDefinition,
	createBrowserRunToolDefinition,
} from "./browser/index.js";
import {
	createDocEditToolDefinition,
	createDocGrepToolDefinition,
	createDocPeekToolDefinition,
	createDocReadToolDefinition,
	createDocScanToolDefinition,
	createDocWriteToolDefinition,
	type DocEditToolOptions,
	type DocGrepToolOptions,
	type DocPeekToolOptions,
	type DocReadToolOptions,
	type DocScanToolOptions,
	type DocWriteToolOptions,
} from "./doc/index.js";
import { createEditToolDefinition, type EditToolOptions } from "./edit.js";
import { createFindToolDefinition, type FindToolOptions } from "./find.js";
import { createGrepToolDefinition, type GrepToolOptions } from "./grep.js";
import { createLsToolDefinition, type LsToolOptions } from "./ls.js";
import { createReadToolDefinition, type ReadToolOptions } from "./read.js";
import { createSearchToolDefinition, type SearchToolOptions } from "./search.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";
import { createWebFetchToolDefinition, type WebFetchToolOptions } from "./webfetch.js";
import { createWebSearchToolDefinition, type WebSearchToolOptions } from "./websearch.js";
import { createWriteToolDefinition, type WriteToolOptions } from "./write.js";

export type Tool = AgentTool<any>;
export type ToolDef = ToolDefinition<any, any>;

export interface ToolsOptions {
	read?: ReadToolOptions;
	bash?: BashToolOptions;
	write?: WriteToolOptions;
	edit?: EditToolOptions;
	grep?: GrepToolOptions;
	find?: FindToolOptions;
	ls?: LsToolOptions;
	search?: SearchToolOptions;
	webfetch?: WebFetchToolOptions;
	websearch?: WebSearchToolOptions;
	browser_run?: BrowserRunToolOptions;
	browser_continue?: BrowserContinueToolOptions;
	DocRead?: DocReadToolOptions;
	DocEdit?: DocEditToolOptions;
	DocWrite?: DocWriteToolOptions;
	DocScan?: DocScanToolOptions;
	DocGrep?: DocGrepToolOptions;
	DocPeek?: DocPeekToolOptions;
}

/**
 * Single source of truth for built-in tools: name → definition factory.
 * Everything else (name unions, option lookups, bundle helpers) derives
 * from this table, so adding or removing a tool touches exactly one entry
 * (plus its `ToolsOptions` key).
 */
const TOOL_FACTORIES = {
	read: createReadToolDefinition,
	bash: createBashToolDefinition,
	edit: createEditToolDefinition,
	write: createWriteToolDefinition,
	grep: createGrepToolDefinition,
	find: createFindToolDefinition,
	ls: createLsToolDefinition,
	search: createSearchToolDefinition,
	webfetch: createWebFetchToolDefinition,
	websearch: createWebSearchToolDefinition,
	browser_run: createBrowserRunToolDefinition,
	browser_continue: createBrowserContinueToolDefinition,
	DocRead: createDocReadToolDefinition,
	DocEdit: createDocEditToolDefinition,
	DocWrite: createDocWriteToolDefinition,
	DocScan: createDocScanToolDefinition,
	DocGrep: createDocGrepToolDefinition,
	DocPeek: createDocPeekToolDefinition,
} satisfies { [K in keyof ToolsOptions]-?: (cwd: string, options?: ToolsOptions[K]) => ToolDef };

export type ToolName = keyof typeof TOOL_FACTORIES;

export const allToolNames: Set<ToolName> = new Set(Object.keys(TOOL_FACTORIES) as ToolName[]);

/** The default coding bundle (read/write + shell). */
const CODING_TOOL_NAMES: ToolName[] = ["read", "bash", "edit", "write", "grep", "find", "ls"];

/** Read-only exploration bundle. */
const READ_ONLY_TOOL_NAMES: ToolName[] = ["read", "grep", "find", "ls"];

export function createToolDefinition(toolName: ToolName, cwd: string, options?: ToolsOptions): ToolDef {
	const factory = TOOL_FACTORIES[toolName] as (cwd: string, options?: ToolsOptions[ToolName]) => ToolDef;
	if (!factory) {
		throw new Error(`Unknown tool name: ${toolName}`);
	}
	return factory(cwd, options?.[toolName]);
}

export function createTool(toolName: ToolName, cwd: string, options?: ToolsOptions): Tool {
	return wrapToolDefinition(createToolDefinition(toolName, cwd, options)) as Tool;
}

export function createCodingToolDefinitions(cwd: string, options?: ToolsOptions): ToolDef[] {
	return CODING_TOOL_NAMES.map((name) => createToolDefinition(name, cwd, options));
}

export function createReadOnlyToolDefinitions(cwd: string, options?: ToolsOptions): ToolDef[] {
	return READ_ONLY_TOOL_NAMES.map((name) => createToolDefinition(name, cwd, options));
}

export function createAllToolDefinitions(cwd: string, options?: ToolsOptions): Record<ToolName, ToolDef> {
	const entries = [...allToolNames].map((name) => [name, createToolDefinition(name, cwd, options)] as const);
	return Object.fromEntries(entries) as Record<ToolName, ToolDef>;
}

export function createCodingTools(cwd: string, options?: ToolsOptions): Tool[] {
	return CODING_TOOL_NAMES.map((name) => createTool(name, cwd, options));
}

export function createReadOnlyTools(cwd: string, options?: ToolsOptions): Tool[] {
	return READ_ONLY_TOOL_NAMES.map((name) => createTool(name, cwd, options));
}

export function createAllTools(cwd: string, options?: ToolsOptions): Record<ToolName, Tool> {
	const entries = [...allToolNames].map((name) => [name, createTool(name, cwd, options)] as const);
	return Object.fromEntries(entries) as Record<ToolName, Tool>;
}
