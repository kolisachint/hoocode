/**
 * Canvas extensions that arrive inside an installed plugin.
 *
 * Design: `docs/canvas-extensions-design.md` §4.3. Discovery's three search roots
 * (`.agents/extensions`, `.github/extensions`, `~/.copilot/extensions`) describe
 * where a person *puts* a canvas by hand. They say nothing about where a package
 * manager puts one, and a plugin install lands in `~/.agents/plugins/<name>/`
 * instead — so a canvas plugin installed exactly as instructed was copied to disk
 * intact and then never listed, because nothing looked there.
 *
 * The plugin readers already resolve each plugin's canvas directories
 * (`NormalizedPlugin.canvasExtensions`), so this module is only the join: read
 * the plugins on the discovery path, and present their canvases in the shape
 * `discovery.ts` produces for everything else. Keeping the shape identical is
 * what lets the trust gate, the listing and `open` stay unaware that a plugin was
 * involved at all.
 */

import * as path from "node:path";
import { getAgentDir } from "../../config.js";
import { defaultPluginDirs } from "../extensions/loader.js";
import { discoverPlugins } from "../extensions/plugins/index.js";
import { pluginScopeOf } from "../extensions/plugins/locations.js";
import { CANVAS_ENTRY_FILE, type DiscoveredCanvasExtension } from "./discovery.js";

/**
 * Canvases shipped by every plugin on the discovery path.
 *
 * Scope is derived from where the plugin lives, not from the canvas directory:
 * a plugin installed at project scope (or committed into the working tree) is
 * repository-supplied whichever subdirectory its canvas sits in, and that is the
 * question the trust gate asks. `pluginScopeOf`'s third answer, `repo`, folds
 * into `project` here — for a canvas the two mean the same thing, since neither
 * can distinguish "I put this here" from "this arrived in the clone".
 */
export function pluginCanvasExtensions(cwd: string, agentDir: string = getAgentDir()): DiscoveredCanvasExtension[] {
	const found: DiscoveredCanvasExtension[] = [];
	for (const plugin of discoverPlugins(defaultPluginDirs(cwd, agentDir))) {
		if (!plugin.canvasExtensions?.length) continue;
		const scope = pluginScopeOf(plugin.root, cwd) === "user" ? "user" : "project";
		for (const extension of plugin.canvasExtensions) {
			found.push({
				id: extension.id,
				dir: extension.dir,
				entry: path.join(extension.dir, CANVAS_ENTRY_FILE),
				scope,
			});
		}
	}
	return found;
}
