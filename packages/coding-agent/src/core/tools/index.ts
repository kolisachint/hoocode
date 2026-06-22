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
	createGlobTool,
	createGlobToolDefinition,
	type GlobOperations,
	type GlobToolDetails,
	type GlobToolInput,
	type GlobToolOptions,
} from "./glob.js";
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
import { type BashToolOptions, createBashTool, createBashToolDefinition } from "./bash.js";
import { createEditTool, createEditToolDefinition, type EditToolOptions } from "./edit.js";
import { createFindTool, createFindToolDefinition, type FindToolOptions } from "./find.js";
import { createGlobTool, createGlobToolDefinition, type GlobToolOptions } from "./glob.js";
import { createGrepTool, createGrepToolDefinition, type GrepToolOptions } from "./grep.js";
import { createLsTool, createLsToolDefinition, type LsToolOptions } from "./ls.js";
import { createReadTool, createReadToolDefinition, type ReadToolOptions } from "./read.js";
import { createWebFetchTool, createWebFetchToolDefinition, type WebFetchToolOptions } from "./webfetch.js";
import { createWebSearchTool, createWebSearchToolDefinition, type WebSearchToolOptions } from "./websearch.js";
import { createWriteTool, createWriteToolDefinition, type WriteToolOptions } from "./write.js";

export type Tool = AgentTool<any>;
export type ToolDef = ToolDefinition<any, any>;
export type ToolName = "read" | "bash" | "edit" | "write" | "grep" | "find" | "glob" | "ls" | "webfetch" | "websearch";
export const allToolNames: Set<ToolName> = new Set([
	"read",
	"bash",
	"edit",
	"write",
	"grep",
	"find",
	"glob",
	"ls",
	"webfetch",
	"websearch",
]);

export interface ToolsOptions {
	read?: ReadToolOptions;
	bash?: BashToolOptions;
	write?: WriteToolOptions;
	edit?: EditToolOptions;
	grep?: GrepToolOptions;
	find?: FindToolOptions;
	glob?: GlobToolOptions;
	ls?: LsToolOptions;
	webfetch?: WebFetchToolOptions;
	websearch?: WebSearchToolOptions;
}

export function createToolDefinition(toolName: ToolName, cwd: string, options?: ToolsOptions): ToolDef {
	switch (toolName) {
		case "read":
			return createReadToolDefinition(cwd, options?.read);
		case "bash":
			return createBashToolDefinition(cwd, options?.bash);
		case "edit":
			return createEditToolDefinition(cwd, options?.edit);
		case "write":
			return createWriteToolDefinition(cwd, options?.write);
		case "grep":
			return createGrepToolDefinition(cwd, options?.grep);
		case "find":
			return createFindToolDefinition(cwd, options?.find);
		case "glob":
			return createGlobToolDefinition(cwd, options?.glob);
		case "ls":
			return createLsToolDefinition(cwd, options?.ls);
		case "webfetch":
			return createWebFetchToolDefinition(cwd, options?.webfetch);
		case "websearch":
			return createWebSearchToolDefinition(cwd, options?.websearch);
		default:
			throw new Error(`Unknown tool name: ${toolName}`);
	}
}

export function createTool(toolName: ToolName, cwd: string, options?: ToolsOptions): Tool {
	switch (toolName) {
		case "read":
			return createReadTool(cwd, options?.read);
		case "bash":
			return createBashTool(cwd, options?.bash);
		case "edit":
			return createEditTool(cwd, options?.edit);
		case "write":
			return createWriteTool(cwd, options?.write);
		case "grep":
			return createGrepTool(cwd, options?.grep);
		case "find":
			return createFindTool(cwd, options?.find);
		case "glob":
			return createGlobTool(cwd, options?.glob);
		case "ls":
			return createLsTool(cwd, options?.ls);
		case "webfetch":
			return createWebFetchTool(cwd, options?.webfetch);
		case "websearch":
			return createWebSearchTool(cwd, options?.websearch);
		default:
			throw new Error(`Unknown tool name: ${toolName}`);
	}
}

export function createCodingToolDefinitions(cwd: string, options?: ToolsOptions): ToolDef[] {
	return [
		createReadToolDefinition(cwd, options?.read),
		createBashToolDefinition(cwd, options?.bash),
		createEditToolDefinition(cwd, options?.edit),
		createWriteToolDefinition(cwd, options?.write),
		createGrepToolDefinition(cwd, options?.grep),
		createFindToolDefinition(cwd, options?.find),
		createGlobToolDefinition(cwd, options?.glob),
		createLsToolDefinition(cwd, options?.ls),
	];
}

export function createReadOnlyToolDefinitions(cwd: string, options?: ToolsOptions): ToolDef[] {
	return [
		createReadToolDefinition(cwd, options?.read),
		createGrepToolDefinition(cwd, options?.grep),
		createFindToolDefinition(cwd, options?.find),
		createGlobToolDefinition(cwd, options?.glob),
		createLsToolDefinition(cwd, options?.ls),
	];
}

export function createAllToolDefinitions(cwd: string, options?: ToolsOptions): Record<ToolName, ToolDef> {
	return {
		read: createReadToolDefinition(cwd, options?.read),
		bash: createBashToolDefinition(cwd, options?.bash),
		edit: createEditToolDefinition(cwd, options?.edit),
		write: createWriteToolDefinition(cwd, options?.write),
		grep: createGrepToolDefinition(cwd, options?.grep),
		find: createFindToolDefinition(cwd, options?.find),
		glob: createGlobToolDefinition(cwd, options?.glob),
		ls: createLsToolDefinition(cwd, options?.ls),
		webfetch: createWebFetchToolDefinition(cwd, options?.webfetch),
		websearch: createWebSearchToolDefinition(cwd, options?.websearch),
	};
}

export function createCodingTools(cwd: string, options?: ToolsOptions): Tool[] {
	return [
		createReadTool(cwd, options?.read),
		createBashTool(cwd, options?.bash),
		createEditTool(cwd, options?.edit),
		createWriteTool(cwd, options?.write),
		createGrepTool(cwd, options?.grep),
		createFindTool(cwd, options?.find),
		createGlobTool(cwd, options?.glob),
		createLsTool(cwd, options?.ls),
	];
}

export function createReadOnlyTools(cwd: string, options?: ToolsOptions): Tool[] {
	return [
		createReadTool(cwd, options?.read),
		createGrepTool(cwd, options?.grep),
		createFindTool(cwd, options?.find),
		createGlobTool(cwd, options?.glob),
		createLsTool(cwd, options?.ls),
	];
}

export function createAllTools(cwd: string, options?: ToolsOptions): Record<ToolName, Tool> {
	return {
		read: createReadTool(cwd, options?.read),
		bash: createBashTool(cwd, options?.bash),
		edit: createEditTool(cwd, options?.edit),
		write: createWriteTool(cwd, options?.write),
		grep: createGrepTool(cwd, options?.grep),
		find: createFindTool(cwd, options?.find),
		glob: createGlobTool(cwd, options?.glob),
		ls: createLsTool(cwd, options?.ls),
		webfetch: createWebFetchTool(cwd, options?.webfetch),
		websearch: createWebSearchTool(cwd, options?.websearch),
	};
}
