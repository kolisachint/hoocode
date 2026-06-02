/**
 * Module-level stores for agent paths:
 *
 *   manifestPaths — agents declared via `hoocode.agents` in package.json,
 *                   refreshed on every DefaultResourceLoader.reload().
 *   cliPaths      — agents injected via `--agent <path>` at startup,
 *                   set once and never cleared across reloads.
 */

let manifestPaths: string[] = [];
let cliPaths: string[] = [];

/** Replace the stored manifest paths (called by DefaultResourceLoader.reload). */
export function setAgentManifestPaths(paths: string[]): void {
	manifestPaths = [...paths];
}

/** Return the current manifest paths. */
export function getAgentManifestPaths(): string[] {
	return [...manifestPaths];
}

/** Set CLI-injected agent paths (called once at startup from main). */
export function setAgentCliPaths(paths: string[]): void {
	cliPaths = [...paths];
}

/** Return the CLI-injected agent paths. */
export function getAgentCliPaths(): string[] {
	return [...cliPaths];
}
