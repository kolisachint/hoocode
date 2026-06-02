/**
 * Module-level store for agent paths discovered from package manifests.
 *
 * Agents declared via `hoocode.agents` in a package.json are resolved by
 * DefaultResourceLoader.reload() and stored here so every subsequent call to
 * loadAgentRegistry() picks them up without requiring callers to plumb paths
 * through every call site.
 */

let manifestPaths: string[] = [];

/** Replace the stored manifest paths (called by DefaultResourceLoader.reload). */
export function setAgentManifestPaths(paths: string[]): void {
	manifestPaths = [...paths];
}

/** Return the current manifest paths. */
export function getAgentManifestPaths(): string[] {
	return [...manifestPaths];
}
