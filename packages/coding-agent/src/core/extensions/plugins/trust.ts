/**
 * Workspace trust — the per-machine record of which working directories the user
 * has agreed to run repository-supplied plugin code from.
 *
 * The problem it solves (docs/plugin-system-architecture.md §5.9): a plugin
 * committed to a repository is code that runs for whoever clones it next. Its
 * skills and commands are text the model reads, which is no worse than reading
 * the repository itself, but its **hooks and MCP servers are processes** that
 * start on session load. Nothing about a plugin's location can distinguish "I
 * installed this here" from "this arrived in the clone", because everything in
 * the repository travels with it — including any marker a plugin might carry to
 * claim otherwise.
 *
 * So the record lives **outside the repository**, in the agent dir, keyed by
 * absolute path. That is the whole design: trust cannot be forged by repository
 * content, because repository content cannot write here. It is the same shape
 * Claude Code's workspace trust dialog and VS Code's trusted folders use, and it
 * carries the same known consequence — once a directory is trusted, code pulled
 * into it later is trusted too. Trust is a statement about a *place you work*,
 * not about a specific commit.
 *
 * Granting is a human act (`/plugin trust`, or an explicit `/plugin install
 * --scope project`, where the person is demonstrably operating in the directory
 * on purpose). The autonomous install path never grants it: a model deciding
 * that a workspace should execute repository code is exactly the decision this
 * record exists to keep with a person.
 */

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import * as path from "node:path";
import { getAgentDir } from "../../../config.js";

/** One trusted working directory. */
export interface TrustedWorkspace {
	/** Absolute, resolved path. */
	path: string;
	/** When trust was granted, ISO-8601. Informational — nothing expires today. */
	at: string;
}

interface TrustStoreFile {
	workspaces?: TrustedWorkspace[];
}

/**
 * `~/.agents/trusted-workspaces.json`.
 *
 * Beside the marketplace registry rather than inside the repo, for the reason in
 * the module docstring: a file the repository can write is not a trust record.
 */
export function trustStorePath(agentDir: string = getAgentDir()): string {
	return path.join(path.dirname(agentDir), ".agents", "trusted-workspaces.json");
}

function readStore(agentDir: string): TrustedWorkspace[] {
	try {
		const parsed = JSON.parse(readFileSync(trustStorePath(agentDir), "utf8")) as TrustStoreFile;
		return Array.isArray(parsed?.workspaces)
			? parsed.workspaces.filter((w): w is TrustedWorkspace => typeof w?.path === "string")
			: [];
	} catch {
		return [];
	}
}

function writeStore(agentDir: string, workspaces: TrustedWorkspace[]): void {
	const file = trustStorePath(agentDir);
	mkdirSync(path.dirname(file), { recursive: true });
	writeFileSync(file, `${JSON.stringify({ workspaces }, null, 2)}\n`, "utf8");
}

/** Every trusted workspace, newest grant first. */
export function listTrustedWorkspaces(agentDir: string = getAgentDir()): TrustedWorkspace[] {
	return [...readStore(agentDir)].sort((a, b) => (a.at < b.at ? 1 : -1));
}

/**
 * Whether `cwd` is trusted.
 *
 * Exact-path only, deliberately: trusting `~/src` must not silently trust every
 * repository ever cloned beneath it, which is what a prefix match would do the
 * first time someone trusts a directory one level too high.
 */
export function isWorkspaceTrusted(cwd: string, agentDir: string = getAgentDir()): boolean {
	const target = path.resolve(cwd);
	return readStore(agentDir).some((w) => path.resolve(w.path) === target);
}

/** Grant trust to `cwd`. Idempotent — re-granting refreshes the timestamp. */
export function trustWorkspace(cwd: string, agentDir: string = getAgentDir()): TrustedWorkspace {
	const target = path.resolve(cwd);
	const record: TrustedWorkspace = { path: target, at: new Date().toISOString() };
	writeStore(agentDir, [...readStore(agentDir).filter((w) => path.resolve(w.path) !== target), record]);
	return record;
}

/** Revoke trust for `cwd`. Returns false when it was not trusted to begin with. */
export function untrustWorkspace(cwd: string, agentDir: string = getAgentDir()): boolean {
	const target = path.resolve(cwd);
	const before = readStore(agentDir);
	const after = before.filter((w) => path.resolve(w.path) !== target);
	if (after.length === before.length) return false;
	writeStore(agentDir, after);
	return true;
}
