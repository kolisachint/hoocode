/**
 * Canvas extension discovery.
 *
 * Design: `docs/canvas-extensions-design.md` §4.1, §4.3. GitHub's
 * `docs/extensions.md` is specific: the CLI "scans `.github/extensions/`
 * (project) and the user's copilot config extensions directory for
 * subdirectories containing `extension.mjs`", the entry file is required and must
 * be named `extension.mjs`, and only ES modules are supported. All 23 extensions
 * in `github/awesome-copilot` comply.
 *
 * So discovery keys off `<dir>/extension.mjs` and nothing else. In particular it
 * does not read `package.json`: 3 of those 23 extensions ship without one, none
 * ships `node_modules`, and where a `package.json` does exist its `main` is
 * always `extension.mjs` anyway. Keying off it would add a failure mode without
 * adding information.
 *
 * The extension id is the directory name, matching how the catalog and the
 * Copilot app refer to extensions (`pr-artifact-explorer`).
 *
 * Some extensions also carry a `copilot-extension.json`, but it is neither
 * required nor informative: 8 of the 23 catalog extensions have one, every
 * instance holds exactly `{ name, version }`, and `name` always equals the
 * directory name. Reading it would add a parse and a disagreement case without
 * telling us anything the directory name does not.
 */

import { readdirSync, statSync } from "node:fs";
import * as path from "node:path";

/** Required entry file name. */
export const CANVAS_ENTRY_FILE = "extension.mjs";

/** Where a canvas extension came from, in precedence order. */
export type CanvasScope = "agents" | "project" | "user";

/** Directory searched for canvas extensions. */
export interface CanvasSearchRoot {
	dir: string;
	scope: CanvasScope;
}

/** One discovered extension. */
export interface DiscoveredCanvasExtension {
	/** Directory name, used as the provider identifier. */
	id: string;
	/** Absolute path to the extension directory. */
	dir: string;
	/** Absolute path to `extension.mjs`. */
	entry: string;
	scope: CanvasScope;
}

/**
 * Search roots in precedence order. `.agents/` first per
 * `docs/plugin-format-mapping.md` §0; the Copilot conventions follow as
 * compatibility inputs.
 *
 * `.github/extensions/` travels with a clone, so anything found there is subject
 * to the workspace-trust gate before it is ever forked (design doc §5). Discovery
 * itself is read-only and always safe to run.
 */
export function canvasSearchRoots(cwd: string, homeDir: string): CanvasSearchRoot[] {
	return [
		{ dir: path.join(cwd, ".agents", "extensions"), scope: "agents" },
		{ dir: path.join(cwd, ".github", "extensions"), scope: "project" },
		{ dir: path.join(homeDir, ".copilot", "extensions"), scope: "user" },
	];
}

function subdirectories(dir: string): string[] {
	try {
		return readdirSync(dir, { withFileTypes: true })
			.filter((entry) => entry.isDirectory() || entry.isSymbolicLink())
			.map((entry) => entry.name)
			.sort();
	} catch {
		return [];
	}
}

function hasEntryFile(dir: string): boolean {
	try {
		return statSync(path.join(dir, CANVAS_ENTRY_FILE)).isFile();
	} catch {
		return false;
	}
}

/**
 * Discover canvas extensions across `roots`, then append `extra`. First wins on
 * an id collision, so a project-local extension shadows a same-named user-scope
 * one — and anything a person placed in a search root by hand shadows a
 * same-named canvas that arrived inside a package.
 *
 * `extra` is how plugin-shipped canvases join the same list (see
 * `plugin-canvases.ts`): a plugin's canvas directories are named by its manifest
 * rather than by their position under a search root, so they cannot be expressed
 * as a root — but once resolved they are ordinary discovered extensions, and
 * every consumer downstream is better off not knowing the difference.
 */
export function discoverCanvasExtensions(
	roots: CanvasSearchRoot[],
	extra: readonly DiscoveredCanvasExtension[] = [],
): DiscoveredCanvasExtension[] {
	const found = new Map<string, DiscoveredCanvasExtension>();
	for (const root of roots) {
		for (const id of subdirectories(root.dir)) {
			if (found.has(id)) continue;
			const dir = path.join(root.dir, id);
			if (!hasEntryFile(dir)) continue;
			found.set(id, { id, dir, entry: path.join(dir, CANVAS_ENTRY_FILE), scope: root.scope });
		}
	}
	for (const extension of extra) {
		if (found.has(extension.id)) continue;
		if (!hasEntryFile(extension.dir)) continue;
		found.set(extension.id, extension);
	}
	return [...found.values()];
}
