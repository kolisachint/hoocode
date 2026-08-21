/**
 * Session-scoped canvas facade: everything the TUI needs, with no TUI in it.
 *
 * Design: `docs/canvas-extensions-design.md` §11. The pieces underneath — discovery,
 * the trust gate, availability, the registry — are each small and separately tested.
 * This is what stitches them into the four questions a user surface actually asks:
 * what is there, can it run, open this one, close that one.
 *
 * It holds no TUI types on purpose. `extensions/core/canvas.ts` renders and supplies
 * an `AbortSignal` from a cancellable loader; everything decided here stays testable
 * without a terminal.
 *
 * Availability is resolved once and cached, because resolving can spawn
 * `node --version` (§11.1) and the answer cannot change within a session.
 */

import { getAgentDir } from "../../config.js";
import type { DiscoveredCanvasExtension } from "./discovery.js";
import { type CanvasSearchRoot, canvasSearchRoots, discoverCanvasExtensions } from "./discovery.js";
import { type CanvasAvailability, resolveCanvasRuntime } from "./launch.js";
import { pluginCanvasExtensions } from "./plugin-canvases.js";
import { type CanvasInstance, CanvasRegistry, type CanvasRegistryEvents, type CanvasReloadResult } from "./registry.js";
import type { CanvasCallOptions } from "./runner.js";
import { gateCanvasExtensions } from "./trust.js";

/** One canvas a person could open, or has open. */
export interface CanvasListing {
	extensionId: string;
	/** Undefined until the extension has been forked, since declarations come from it. */
	canvasId: string | undefined;
	displayName: string | undefined;
	scope: DiscoveredCanvasExtension["scope"];
	/** Why it cannot be opened, if it cannot. */
	withheld: "untrusted-workspace" | undefined;
	/** Instances of this canvas that are currently open. */
	open: CanvasInstance[];
}

/** What `list()` reports. */
export interface CanvasOverview {
	/** Absent `reason` means canvases can run here. */
	availability: CanvasAvailability;
	listings: CanvasListing[];
	/** Extensions withheld by the trust gate — surfaced, never hidden (§5.1). */
	withheldCount: number;
}

/** Configuration for a session's canvas facade. */
export interface CanvasSessionOptions extends CanvasRegistryEvents {
	cwd: string;
	homeDir: string;
	agentDir?: string;
	/** Override the search roots; defaults to {@link canvasSearchRoots}. */
	roots?: CanvasSearchRoot[];
	/**
	 * Override how plugin-shipped canvases are found; defaults to
	 * {@link pluginCanvasExtensions}. Pass `() => []` to look at the search roots
	 * and nothing else.
	 */
	pluginExtensions?: () => DiscoveredCanvasExtension[];
	/** Override availability resolution, for tests and for hosts that already know. */
	resolveRuntime?: () => Promise<CanvasAvailability>;
}

/** Reference to a canvas: an extension id, optionally narrowed to one of its canvases. */
export interface CanvasRef {
	extensionId: string;
	canvasId?: string;
}

/**
 * Parse `extension` or `extension:canvas`.
 *
 * Extension ids are directory names and canvas ids are provider-local, so a single
 * colon is unambiguous and needs no quoting.
 */
export function parseCanvasRef(input: string): CanvasRef | undefined {
	const trimmed = input.trim();
	if (trimmed.length === 0) return undefined;
	const colon = trimmed.indexOf(":");
	if (colon === -1) return { extensionId: trimmed };
	const extensionId = trimmed.slice(0, colon).trim();
	const canvasId = trimmed.slice(colon + 1).trim();
	if (extensionId.length === 0 || canvasId.length === 0) return undefined;
	return { extensionId, canvasId };
}

export class CanvasSession {
	private readonly options: CanvasSessionOptions;
	private readonly roots: CanvasSearchRoot[];
	private readonly pluginExtensions: () => DiscoveredCanvasExtension[];
	/** Cached because resolving can spawn `node --version` and cannot change mid-session. */
	private availabilityPromise: Promise<CanvasAvailability> | undefined;
	/** Created on first successful open, not at construction: listing must not fork. */
	private registry: CanvasRegistry | undefined;

	constructor(options: CanvasSessionOptions) {
		this.options = options;
		this.roots = options.roots ?? canvasSearchRoots(options.cwd, options.homeDir);
		// Re-read on every discover rather than cached: /plugin install can add a
		// canvas mid-session, and a listing that cannot see it is the bug this
		// exists to fix.
		this.pluginExtensions =
			options.pluginExtensions ?? (() => pluginCanvasExtensions(options.cwd, options.agentDir ?? getAgentDir()));
	}

	/** Discovered extensions, partitioned by the trust gate. Read-only and always safe. */
	discover(): { runnable: DiscoveredCanvasExtension[]; withheld: DiscoveredCanvasExtension[] } {
		const gated = gateCanvasExtensions(
			discoverCanvasExtensions(this.roots, this.pluginExtensions()),
			this.options.cwd,
			this.options.agentDir ?? getAgentDir(),
		);
		return { runnable: gated.runnable, withheld: gated.withheld.map((entry) => entry.extension) };
	}

	/** Whether canvases can run here. Resolved once per session and cached. */
	async availability(): Promise<CanvasAvailability> {
		this.availabilityPromise ??= (this.options.resolveRuntime ?? resolveCanvasRuntime)();
		return this.availabilityPromise;
	}

	/**
	 * What is installed, what is open, and what is being withheld.
	 *
	 * Deliberately does not fork anything: listing must stay free and safe, so a
	 * `canvasId` is only known for extensions already running. That is the visible
	 * consequence of a canvas having no passive half (§5.1) — even its name comes from
	 * running its code.
	 */
	async list(): Promise<CanvasOverview> {
		const availability = await this.availability();
		const { runnable, withheld } = this.discover();
		const open = this.registryOrUndefined()?.listInstances() ?? [];

		const listings: CanvasListing[] = [];
		for (const extension of runnable) {
			const instances = open.filter((instance) => instance.extensionId === extension.id);
			if (instances.length === 0) {
				listings.push({
					extensionId: extension.id,
					canvasId: undefined,
					displayName: undefined,
					scope: extension.scope,
					withheld: undefined,
					open: [],
				});
				continue;
			}
			for (const canvasId of new Set(instances.map((instance) => instance.canvasId))) {
				const forCanvas = instances.filter((instance) => instance.canvasId === canvasId);
				listings.push({
					extensionId: extension.id,
					canvasId,
					displayName: forCanvas[0]?.title,
					scope: extension.scope,
					withheld: undefined,
					open: forCanvas,
				});
			}
		}
		for (const extension of withheld) {
			listings.push({
				extensionId: extension.id,
				canvasId: undefined,
				displayName: undefined,
				scope: extension.scope,
				withheld: "untrusted-workspace",
				open: [],
			});
		}
		return { availability, listings, withheldCount: withheld.length };
	}

	/**
	 * Open a canvas.
	 *
	 * `options.signal` comes from the caller's cancellable loader, so a person's Esc
	 * reaches the registry's abandon path (§11.6) rather than merely hiding a spinner.
	 */
	async open(ref: CanvasRef, options?: CanvasCallOptions): Promise<CanvasInstance> {
		const availability = await this.availability();
		if (!availability.available) throw new Error(availability.reason);

		const { runnable, withheld } = this.discover();
		const extension = runnable.find((candidate) => candidate.id === ref.extensionId);
		if (!extension) {
			if (withheld.some((candidate) => candidate.id === ref.extensionId)) {
				throw new Error(
					`Canvas extension "${ref.extensionId}" came with this repository, which is not a trusted workspace. ` +
						"Run /plugin trust to allow this directory to run code it ships.",
				);
			}
			const known = runnable.map((candidate) => candidate.id).join(", ") || "none";
			throw new Error(`No canvas extension "${ref.extensionId}" (found: ${known}).`);
		}

		const registry = this.ensureRegistry(availability);
		const canvasId = ref.canvasId ?? (await this.soleCanvasId(registry, extension));
		return registry.open(extension, canvasId, undefined, options);
	}

	/** Close one open instance. Unknown ids are a no-op, so closing twice is harmless. */
	async close(instanceId: string): Promise<CanvasInstance | undefined> {
		const registry = this.registryOrUndefined();
		const instance = registry?.listInstances().find((open) => open.instanceId === instanceId);
		if (!registry || !instance) return undefined;
		await registry.close(instance);
		return instance;
	}

	/**
	 * Re-fork an open extension so an edit to its code takes effect.
	 *
	 * Reached by extension id rather than instance id because a reload restarts the
	 * *process*, and one child serves every instance of every canvas the extension
	 * declares — pretending it could reload one instance would be a lie about what
	 * happens. {@link CanvasRegistry.reload} carries the open instances across.
	 */
	async reload(extensionId: string, options?: CanvasCallOptions): Promise<CanvasReloadResult> {
		const registry = this.registryOrUndefined();
		if (!registry) {
			throw new Error(
				`Canvas extension "${extensionId}" is not running, so there is nothing to reload. Open it first.`,
			);
		}
		return registry.reload(extensionId, options);
	}

	/** The extension ids with at least one open instance — what {@link reload} accepts. */
	runningExtensionIds(): string[] {
		return [...new Set(this.instances().map((instance) => instance.extensionId))];
	}

	/** Every open instance. */
	instances(): CanvasInstance[] {
		return this.registryOrUndefined()?.listInstances() ?? [];
	}

	/**
	 * The live registry, or undefined if nothing has been opened yet.
	 *
	 * Exposed so the host can hand it to the canvas tools, which read
	 * `listInstances()` and `activeActions()` from it.
	 */
	registryOrUndefined(): CanvasRegistry | undefined {
		return this.registry;
	}

	/** Advisory cleanup, driven by whoever owns the session clock. */
	async reapIdle(): Promise<string[]> {
		return (await this.registryOrUndefined()?.reapIdle()) ?? [];
	}

	/** Close everything and stop every child. Safe to call twice. */
	async dispose(): Promise<void> {
		await this.registry?.shutdown();
		this.registry = undefined;
	}

	private ensureRegistry(availability: Extract<CanvasAvailability, { available: true }>): CanvasRegistry {
		if (!this.registry) {
			this.registry = new CanvasRegistry({
				runtime: availability.runtime,
				cwd: this.options.cwd,
				agentDir: this.options.agentDir,
				onLog: this.options.onLog,
				onStray: this.options.onStray,
				onStderr: this.options.onStderr,
				onDiagnostic: this.options.onDiagnostic,
			});
		}
		return this.registry;
	}

	/**
	 * Pick the canvas when the caller named only an extension.
	 *
	 * Forking to read the declarations is unavoidable: they arrive in the child's
	 * `ready` message. A multi-canvas extension must be named explicitly rather than
	 * guessed at.
	 */
	private async soleCanvasId(registry: CanvasRegistry, extension: DiscoveredCanvasExtension): Promise<string> {
		const declarations = await registry.declarations(extension);
		if (declarations.length === 1) return declarations[0]?.id as string;
		if (declarations.length === 0) throw new Error(`Canvas extension "${extension.id}" declares no canvases.`);
		const ids = declarations.map((declaration) => `${extension.id}:${declaration.id}`).join(", ");
		throw new Error(`Canvas extension "${extension.id}" declares several canvases; name one of: ${ids}.`);
	}
}
