/**
 * Trust gating for canvas extensions.
 *
 * Design: `docs/canvas-extensions-design.md` §5. The argument is already written
 * down in `core/extensions/plugins/trust.ts`, for the same reason:
 *
 * > Its skills and commands are text the model reads, which is no worse than
 * > reading the repository itself, but its **hooks and MCP servers are processes**
 * > that start on session load.
 *
 * A canvas extension is a process **that also opens a listening socket**, so it
 * belongs in that record on identical grounds and needs no new mechanism. The gate
 * itself is shared: `isRepositorySupplied` and `shouldWithholdRepositorySupplied`
 * live in `plugins/trust.ts` and are used by the plugin gate too, so this module
 * supplies only the roots that are specific to canvas extensions. The record lives
 * outside the repository, so repository content cannot forge it.
 *
 * One difference from plugins, and it matters. `shouldWithholdExecutables` can
 * withhold a plugin's hooks and MCP servers while still loading its skills,
 * because a plugin has passive capabilities worth having. **A canvas has none.**
 * Its declaration, its actions, and its UI all come from running its code. There
 * is nothing to partially allow, so an untrusted canvas is withheld whole.
 *
 * What stays available is discovery: `discovery.ts` only reads directory entries,
 * so an untrusted canvas can still be listed, named, and offered — the person can
 * see what is on offer and decide to trust the workspace. Listing is not running.
 */

import * as path from "node:path";
import { getAgentDir } from "../../config.js";
import { isRepositorySupplied, shouldWithholdRepositorySupplied } from "../extensions/plugins/trust.js";
import type { DiscoveredCanvasExtension } from "./discovery.js";

/** Thrown when something tries to run a canvas the workspace has not earned. */
export class CanvasTrustError extends Error {
	readonly extensionId: string;

	constructor(extensionId: string, cwd: string) {
		super(
			`Canvas extension "${extensionId}" came with this repository and "${cwd}" is not a trusted workspace. ` +
				`Running it would start a process and open a listening socket on your machine. ` +
				`Trust the workspace first if that is what you want.`,
		);
		this.name = "CanvasTrustError";
		this.extensionId = extensionId;
	}
}

/**
 * Whether an extension sits in the working tree, and therefore arrived with the
 * repository as far as anyone but its author can tell.
 *
 * Both project-scope homes count. `.agents/extensions/` is where hoocode would
 * author its own, and `.github/extensions/` is Copilot's project scope — the one
 * that travels in every clone. Neither location can distinguish "I put this here"
 * from "this arrived in the clone", which is exactly why location is not the
 * question being asked; it only decides *whether* to ask about trust.
 *
 * User scope (`~/.copilot/extensions/`) is not project-supplied: it is outside any
 * repository and got there by a deliberate local act.
 */
export function isProjectSuppliedCanvas(extension: DiscoveredCanvasExtension, cwd: string): boolean {
	return isRepositorySupplied(extension.dir, canvasProjectScopeRoots(cwd));
}

/** A canvas extension's project-scope homes — the locations that travel with a clone. */
function canvasProjectScopeRoots(cwd: string): string[] {
	return [path.join(cwd, ".agents", "extensions"), path.join(cwd, ".github", "extensions")];
}

/**
 * Whether an extension must not be forked: it came with the repository and this
 * machine has not trusted the workspace.
 */
export function shouldWithholdCanvas(
	extension: DiscoveredCanvasExtension,
	cwd: string,
	agentDir: string = getAgentDir(),
): boolean {
	return shouldWithholdRepositorySupplied(extension.dir, cwd, canvasProjectScopeRoots(cwd), agentDir);
}

/** A withheld extension, with the reason, so a caller can explain rather than hide. */
export interface WithheldCanvasExtension {
	extension: DiscoveredCanvasExtension;
	reason: "untrusted-workspace";
}

/** Discovered extensions split into what may be forked and what may not. */
export interface GatedCanvasExtensions {
	runnable: DiscoveredCanvasExtension[];
	withheld: WithheldCanvasExtension[];
}

/**
 * Partition discovered extensions by trust.
 *
 * Callers should present `withheld` rather than dropping it: the point of the gate
 * is that the person can see a repository offers a canvas and choose, not that the
 * offer disappears.
 */
export function gateCanvasExtensions(
	extensions: DiscoveredCanvasExtension[],
	cwd: string,
	agentDir: string = getAgentDir(),
): GatedCanvasExtensions {
	const gated: GatedCanvasExtensions = { runnable: [], withheld: [] };
	for (const extension of extensions) {
		if (shouldWithholdCanvas(extension, cwd, agentDir)) {
			gated.withheld.push({ extension, reason: "untrusted-workspace" });
		} else {
			gated.runnable.push(extension);
		}
	}
	return gated;
}
