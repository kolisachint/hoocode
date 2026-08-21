/**
 * Canvas instance registry — owns children, instances, and reaping.
 *
 * Design: `docs/canvas-extensions-design.md` §4, §6, §7. One child process per
 * extension, many instances per child, keyed by `(extensionId, canvasId,
 * instanceId)` because `joinSession({ canvases: [...] })` takes an array and each
 * canvas can be opened more than once.
 *
 * **Correction to the design doc's §6.** That section called for an "SSE-liveness
 * heartbeat — an instance with no connected client for N seconds is idle". That is
 * not implementable. The SSE endpoint and its client set live inside the
 * extension's own HTTP server (`entry.sseClients` in `pr-artifact-explorer`'s
 * `server.mjs`); the host never sees them. Learning otherwise would take either
 * proxying the canvas URL — which breaks the token, origin and CSP model the
 * extension built — or adding a liveness call to the contract, which breaks tier-2
 * portability. Neither is worth it for a reaper.
 *
 * So idleness here means something narrower and honest: **time since hoocode last
 * touched the instance** (opened it, or invoked an action on it). A person reading
 * a canvas in a browser tab is invisible to us, so a generous timeout is the point
 * rather than a limitation, and `reapIdle` is advisory cleanup — not a claim about
 * whether anybody is watching.
 *
 * The registry starts no timers. `reapIdle()` is driven by the caller and `now` is
 * injectable, so lifetime policy belongs to whoever owns the session clock and the
 * tests do not sleep.
 */

import { randomUUID } from "node:crypto";
import * as path from "node:path";
import type { DiscoveredCanvasExtension } from "./discovery.js";
import type {
	CanvasActionDeclaration,
	CanvasDeclaration,
	CanvasProviderOpenResult,
	CanvasReadyMessage,
	JsonValue,
} from "./protocol.js";
import {
	type CanvasCallOptions,
	type CanvasExtensionProcess,
	type CanvasRunnerOptions,
	type CanvasRuntime,
	spawnCanvasExtension,
} from "./runner.js";
import { CanvasTrustError, shouldWithholdCanvas } from "./trust.js";

/** Default idle ceiling before an untouched instance is reaped. */
export const CANVAS_INSTANCE_IDLE_MS = 30 * 60 * 1_000;

/** Default grace period a child is kept alive after its last instance closes. */
export const CANVAS_CHILD_LINGER_MS = 60 * 1_000;

/** Default cap on concurrent instances of a single canvas. */
export const CANVAS_MAX_INSTANCES_PER_CANVAS = 8;

/** Stable identity of one open instance. */
export interface CanvasInstanceKey {
	extensionId: string;
	canvasId: string;
	instanceId: string;
}

/** An open canvas instance. */
export interface CanvasInstance extends CanvasInstanceKey {
	/** URL the host hands to a browser. */
	url: string | undefined;
	title: string | undefined;
	status: string | undefined;
	/** When hoocode last opened this instance or invoked one of its actions. */
	lastTouchedAt: number;
	/**
	 * The `input` this instance was opened with, kept so {@link CanvasRegistry.reload}
	 * can re-open it the same way. `canvas.open` is the only place a canvas is told
	 * what it is opening *onto*, so replaying it is what makes a reload a reload
	 * rather than a fresh, emptier canvas.
	 */
	openInput: JsonValue | undefined;
}

/**
 * One agent-callable action on an open instance. This is the input the future
 * tool bridge consumes; nothing registers it as a tool yet, deliberately — that
 * makes canvases reachable by the agent and must follow the trust gate (§5).
 */
export interface CanvasActionBinding extends CanvasInstanceKey {
	action: CanvasActionDeclaration;
}

/** An instance that did not survive a {@link CanvasRegistry.reload}, and why. */
export interface CanvasReloadDrop {
	instanceId: string;
	canvasId: string;
	reason: string;
}

/** What a {@link CanvasRegistry.reload} did. */
export interface CanvasReloadResult {
	extensionId: string;
	/**
	 * Instances that came back, with their **new** urls — the old ones are dead
	 * ports. Instance ids are unchanged.
	 */
	reopened: CanvasInstance[];
	/** Instances that could not be re-opened. */
	dropped: CanvasReloadDrop[];
	/** Canvas ids the reloaded extension declares, which the edit may have changed. */
	canvases: string[];
}

/** Diagnostics the registry emits. The host decides how to surface them. */
export interface CanvasRegistryEvents {
	/** A `session.log` call from an extension. */
	onLog?: (extensionId: string, message: string, level: string | undefined) => void;
	/** A non-protocol stdout line — almost always a stray `console.log`. */
	onStray?: (extensionId: string, line: string) => void;
	/** The child's stderr. */
	onStderr?: (extensionId: string, chunk: string) => void;
	/** Something the host should tell the user about once. */
	onDiagnostic?: (extensionId: string, message: string) => void;
}

/** Registry configuration. */
export interface CanvasRegistryOptions extends CanvasRegistryEvents {
	runtime: CanvasRuntime;
	/**
	 * Working directory the trust gate is evaluated against (§5). Required: forking
	 * a canvas that arrived in a clone is exactly what the gate exists to prevent,
	 * so there is no sensible default to fall back to.
	 */
	cwd: string;
	/** Trust-store location. Defaults to the agent dir; injectable for tests. */
	agentDir?: string;
	/** Clock, injectable so idle policy is testable without sleeping. */
	now?: () => number;
	idleTimeoutMs?: number;
	childLingerMs?: number;
	/**
	 * Per-method provider-call ceilings, merged over the runner's defaults.
	 *
	 * Plumbed through because the registry is the entry point everything real goes
	 * via: without this the ceilings in `runner.ts` were only reachable by calling
	 * `spawnCanvasExtension` directly, which nothing does.
	 */
	requestTimeoutMs?: CanvasRunnerOptions["requestTimeoutMs"];
	maxInstancesPerCanvas?: number;
	/** Instance id generator, injectable for deterministic tests. */
	newInstanceId?: () => string;
}

interface ChildEntry {
	process: CanvasExtensionProcess;
	declarations: Map<string, CanvasDeclaration>;
	/** When the child's instance count last dropped to zero; undefined while in use. */
	idleSince: number | undefined;
	/**
	 * The descriptor this child was forked from.
	 *
	 * Kept because {@link CanvasRegistry.reload} is reached by extension id — from a
	 * tool call, or from `/canvas reload` — and re-forking needs the entry file and
	 * the scope the trust gate reads. Re-discovering it here would duplicate the
	 * search roots and could silently resolve a *different* extension than the one
	 * that is running.
	 */
	extension: DiscoveredCanvasExtension;
}

/** Render a key as a stable string, for maps and messages. */
export function canvasInstanceKeyOf(key: CanvasInstanceKey): string {
	return `${key.extensionId}::${key.canvasId}::${key.instanceId}`;
}

export class CanvasRegistry {
	private readonly children = new Map<string, ChildEntry>();
	private readonly instances = new Map<string, CanvasInstance>();
	private readonly options: CanvasRegistryOptions;
	private readonly now: () => number;
	private readonly newInstanceId: () => string;

	constructor(options: CanvasRegistryOptions) {
		this.options = options;
		this.now = options.now ?? Date.now;
		this.newInstanceId = options.newInstanceId ?? randomUUID;
	}

	/** Canvases an extension declares, forking it if it is not already running. */
	async declarations(extension: DiscoveredCanvasExtension): Promise<CanvasDeclaration[]> {
		const child = await this.child(extension);
		return [...child.declarations.values()];
	}

	/** Open a canvas instance and return what the host needs to render it. */
	async open(
		extension: DiscoveredCanvasExtension,
		canvasId: string,
		input?: JsonValue,
		options?: CanvasCallOptions,
	): Promise<CanvasInstance> {
		const child = await this.child(extension);
		if (!child.declarations.has(canvasId)) {
			const known = [...child.declarations.keys()].join(", ") || "none";
			throw new Error(`Extension "${extension.id}" declares no canvas "${canvasId}" (declares: ${known}).`);
		}

		const limit = this.options.maxInstancesPerCanvas ?? CANVAS_MAX_INSTANCES_PER_CANVAS;
		const open = this.listInstances().filter(
			(instance) => instance.extensionId === extension.id && instance.canvasId === canvasId,
		);
		if (open.length >= limit) {
			throw new Error(`Canvas "${canvasId}" already has ${open.length} open instances (limit ${limit}).`);
		}

		const instanceId = this.newInstanceId();
		let result: CanvasProviderOpenResult | null;
		try {
			result = (await child.process.open(
				{ sessionId: extension.id, extensionId: extension.id, canvasId, instanceId, input },
				options,
			)) as CanvasProviderOpenResult | null;
		} catch (cause) {
			await this.abandon(extension.id, canvasId, instanceId);
			throw cause;
		}

		const instance: CanvasInstance = {
			extensionId: extension.id,
			canvasId,
			instanceId,
			url: result?.url,
			title: result?.title,
			status: result?.status,
			lastTouchedAt: this.now(),
			openInput: input,
		};
		this.instances.set(canvasInstanceKeyOf(instance), instance);
		child.idleSince = undefined;
		return instance;
	}

	/** Invoke an action on an open instance. */
	async invokeAction(
		key: CanvasInstanceKey,
		actionName: string,
		input?: JsonValue,
		options?: CanvasCallOptions,
	): Promise<JsonValue> {
		const instance = this.instances.get(canvasInstanceKeyOf(key));
		if (!instance) throw new Error(`No open canvas instance ${canvasInstanceKeyOf(key)}.`);
		const child = this.children.get(key.extensionId);
		if (!child) throw new Error(`Canvas extension "${key.extensionId}" is not running.`);

		const result = await child.process.invokeAction(
			{
				sessionId: key.extensionId,
				extensionId: key.extensionId,
				canvasId: key.canvasId,
				instanceId: key.instanceId,
				actionName,
				input,
			},
			options,
		);
		instance.lastTouchedAt = this.now();
		return result;
	}

	/** Close one instance. Unknown keys are a no-op, so close is idempotent. */
	async close(key: CanvasInstanceKey): Promise<void> {
		const id = canvasInstanceKeyOf(key);
		if (!this.instances.delete(id)) return;
		const child = this.children.get(key.extensionId);
		if (!child) return;
		try {
			await child.process.close({
				sessionId: key.extensionId,
				extensionId: key.extensionId,
				canvasId: key.canvasId,
				instanceId: key.instanceId,
			});
		} catch (cause) {
			// onClose is fire-and-forget in the SDK contract, so a failure here must not
			// leave the instance half-closed in our table.
			this.options.onDiagnostic?.(
				key.extensionId,
				`Closing canvas instance ${id} failed: ${cause instanceof Error ? cause.message : String(cause)}`,
			);
		}
		if (this.instancesOf(key.extensionId).length === 0) child.idleSince = this.now();
	}

	/**
	 * Re-fork a running extension from disk and put its open instances back.
	 *
	 * This is what makes a canvas *iterable*. A canvas has no passive half — its
	 * id, its actions and its UI all come from running its code — so editing
	 * `extension.mjs` changes nothing at all while the child that was forked from
	 * the old bytes is still serving: not the open page, and not even a freshly
	 * opened second instance, because {@link child} hands back the child already in
	 * the table. Without a reload the only way to see an edit is to end the session.
	 *
	 * The order is deliberate. The new child is forked and asked for its
	 * declarations **before** the old one is touched, so an edit that does not run —
	 * a syntax error, a throw at module scope, a `joinSession` that never resolves —
	 * leaves the person looking at exactly the canvas they had, and the error is
	 * reported instead of being paid for with their open surface.
	 *
	 * Instance ids are preserved, so an `instanceId` the model already holds keeps
	 * working across a reload. **URLs are not**: the extension binds a fresh
	 * ephemeral port and mints a fresh capability token in `open()`, and the host
	 * has no way to make it reuse either. So a reload always hands back new URLs,
	 * and the caller must show them — an already-open browser tab is pointing at a
	 * port that is now closed.
	 *
	 * The `input` each instance was opened with is replayed, so a reload restores
	 * the canvas rather than a blank one. Everything the *extension* kept in memory
	 * is gone, which is the honest meaning of restarting a process.
	 */
	async reload(extensionId: string, options?: CanvasCallOptions): Promise<CanvasReloadResult> {
		const previous = this.children.get(extensionId);
		if (!previous) {
			throw new Error(
				`Canvas extension "${extensionId}" is not running, so there is nothing to reload. Open it first.`,
			);
		}

		// Snapshot before anything moves: `close` mutates the instance table.
		const carried = this.instancesOf(extensionId);

		// Fork the edited code first. If it does not come up, the old child is still
		// registered and still serving, and this throws without costing anything.
		const next = await this.spawn(previous.extension);

		// The swap. Registering the new child before stopping the old one is what
		// makes the old one's `onExit` a no-op rather than a table-clearing race.
		this.children.set(extensionId, next);
		for (const instance of carried) this.instances.delete(canvasInstanceKeyOf(instance));
		for (const instance of carried) {
			try {
				await previous.process.close({
					sessionId: extensionId,
					extensionId,
					canvasId: instance.canvasId,
					instanceId: instance.instanceId,
				});
			} catch {
				// The old child is about to be killed, so a refused close costs nothing:
				// its ports go with the process. Reporting it would be noise on a path
				// the person asked for.
			}
		}
		await previous.process.terminate();

		const reopened: CanvasInstance[] = [];
		const dropped: CanvasReloadDrop[] = [];
		for (const instance of carried) {
			// The edit may have renamed or removed the canvas. That is a legitimate
			// thing for an author to do mid-iteration, so it is reported rather than
			// thrown — the other instances still come back.
			if (!next.declarations.has(instance.canvasId)) {
				dropped.push({
					instanceId: instance.instanceId,
					canvasId: instance.canvasId,
					reason: `the reloaded extension no longer declares canvas "${instance.canvasId}" (declares: ${[...next.declarations.keys()].join(", ") || "none"})`,
				});
				continue;
			}
			try {
				const result = (await next.process.open(
					{
						sessionId: extensionId,
						extensionId,
						canvasId: instance.canvasId,
						instanceId: instance.instanceId,
						input: instance.openInput,
					},
					options,
				)) as CanvasProviderOpenResult | null;
				const fresh: CanvasInstance = {
					...instance,
					url: result?.url,
					title: result?.title,
					status: result?.status,
					lastTouchedAt: this.now(),
				};
				this.instances.set(canvasInstanceKeyOf(fresh), fresh);
				reopened.push(fresh);
			} catch (cause) {
				await this.abandon(extensionId, instance.canvasId, instance.instanceId);
				dropped.push({
					instanceId: instance.instanceId,
					canvasId: instance.canvasId,
					reason: cause instanceof Error ? cause.message : String(cause),
				});
			}
		}
		next.idleSince = reopened.length === 0 ? this.now() : undefined;

		return { extensionId, reopened, dropped, canvases: [...next.declarations.keys()] };
	}

	/** Every open instance. */
	listInstances(): CanvasInstance[] {
		return [...this.instances.values()];
	}

	/**
	 * Actions currently invocable, one entry per open instance per declared action.
	 * Empty when nothing is open — which is the point: a canvas that is not open
	 * costs the prompt nothing (§7).
	 */
	activeActions(): CanvasActionBinding[] {
		const bindings: CanvasActionBinding[] = [];
		for (const instance of this.instances.values()) {
			const declaration = this.children.get(instance.extensionId)?.declarations.get(instance.canvasId);
			for (const action of declaration?.actions ?? []) {
				bindings.push({
					extensionId: instance.extensionId,
					canvasId: instance.canvasId,
					instanceId: instance.instanceId,
					action,
				});
			}
		}
		return bindings;
	}

	/**
	 * Close instances hoocode has not touched within the idle timeout, then reap
	 * children that have had no instances for the linger period. Advisory cleanup:
	 * see the module header on what "idle" can and cannot mean here.
	 *
	 * @returns The instance keys that were closed.
	 */
	async reapIdle(): Promise<string[]> {
		const idleTimeout = this.options.idleTimeoutMs ?? CANVAS_INSTANCE_IDLE_MS;
		const linger = this.options.childLingerMs ?? CANVAS_CHILD_LINGER_MS;
		const now = this.now();

		const expired = this.listInstances().filter((instance) => now - instance.lastTouchedAt >= idleTimeout);
		for (const instance of expired) await this.close(instance);

		for (const [extensionId, child] of [...this.children.entries()]) {
			const unused = this.instancesOf(extensionId).length === 0;
			if (!unused) continue;
			const since = child.idleSince ?? now;
			child.idleSince = since;
			if (now - since >= linger) {
				this.children.delete(extensionId);
				await child.process.terminate();
			}
		}

		return expired.map((instance) => canvasInstanceKeyOf(instance));
	}

	/** Close everything and terminate every child. Safe to call twice. */
	async shutdown(): Promise<void> {
		for (const instance of this.listInstances()) await this.close(instance);
		const children = [...this.children.values()];
		this.children.clear();
		await Promise.all(children.map((child) => child.process.terminate()));
	}

	/**
	 * Reconcile an instance we asked to open but never saw open — because a person
	 * cancelled, or the call timed out. One path serves both.
	 *
	 * The provider protocol has no cancel verb, so the child may have finished opening
	 * and be holding a port. `canvas.close` is the only way to tell it to let go, and
	 * it can be sent because the instance id was generated before the open call.
	 *
	 * If the close itself goes unanswered the child is wedged, and the only remaining
	 * lever is terminating it — but that kills every instance of that extension, so it
	 * is done only when no other instance is live. When siblings exist the child is left
	 * alone and the leak is reported, rather than paid for by someone else's open canvas.
	 */
	private async abandon(extensionId: string, canvasId: string, instanceId: string): Promise<void> {
		const child = this.children.get(extensionId);
		if (!child) return;
		try {
			await child.process.close({ sessionId: extensionId, extensionId, canvasId, instanceId });
			return;
		} catch (cause) {
			const detail = cause instanceof Error ? cause.message : String(cause);
			if (this.instancesOf(extensionId).length > 0) {
				this.options.onDiagnostic?.(
					extensionId,
					`Stopped opening canvas "${canvasId}" but the extension did not confirm the close (${detail}). ` +
						"It has other canvases open, so it was left running; a port may stay bound until it exits.",
				);
				return;
			}
			this.children.delete(extensionId);
			await child.process.terminate();
			this.options.onDiagnostic?.(
				extensionId,
				`Stopped opening canvas "${canvasId}" and the extension did not confirm the close (${detail}); it was stopped.`,
			);
		}
	}

	private instancesOf(extensionId: string): CanvasInstance[] {
		return this.listInstances().filter((instance) => instance.extensionId === extensionId);
	}

	private async child(extension: DiscoveredCanvasExtension): Promise<ChildEntry> {
		const existing = this.children.get(extension.id);
		if (existing?.process.running) return existing;
		if (existing) this.children.delete(extension.id);

		const entry = await this.spawn(extension);
		this.children.set(extension.id, entry);
		return entry;
	}

	/**
	 * Fork one extension and wait for its declarations, without registering it.
	 *
	 * Separate from {@link child} because {@link reload} needs to fork a *second*
	 * child while the first is still serving the person's open canvas: if the edit
	 * that prompted the reload does not run, the old child is still there and
	 * nothing was lost. Registering is therefore the caller's step, taken only once
	 * the new child has answered.
	 */
	private async spawn(extension: DiscoveredCanvasExtension): Promise<ChildEntry> {
		// The single choke point: every path that could start a process comes through
		// here, so the gate is enforced once and cannot be bypassed by a caller that
		// forgot to filter. Callers should still filter with `gateCanvasExtensions`
		// so they can explain the refusal; this is the backstop, not the UI.
		if (shouldWithholdCanvas(extension, this.options.cwd, this.options.agentDir)) {
			throw new CanvasTrustError(extension.id, this.options.cwd);
		}

		let spawned: CanvasExtensionProcess | undefined;
		const process = spawnCanvasExtension({
			extensionId: extension.id,
			entry: extension.entry,
			runtime: this.options.runtime,
			requestTimeoutMs: this.options.requestTimeoutMs,
			cwd: path.dirname(extension.dir),
			onLog: (message, level) => this.options.onLog?.(extension.id, message, level),
			onStray: (line) => this.options.onStray?.(extension.id, line),
			onStderr: (chunk) => this.options.onStderr?.(extension.id, chunk),
			// Only the child that is *currently registered* may clear the table. A
			// reload's probe dying before it is adopted must not take the live child's
			// instances with it, and the old child's own exit — which reload causes on
			// purpose, after the new one is registered — must not undo the swap.
			onExit: () => {
				const registered = this.children.get(extension.id);
				if (spawned && registered?.process === spawned) this.forget(extension.id);
			},
		});
		spawned = process;

		let ready: CanvasReadyMessage;
		try {
			ready = await process.ready;
		} catch (cause) {
			// `ready` rejects when the child exits first, but a child that answered
			// nothing and stayed up would otherwise be orphaned by the throw.
			await process.terminate();
			throw cause;
		}

		if (ready.unsupported && ready.unsupported.length > 0) {
			this.options.onDiagnostic?.(
				extension.id,
				`Canvas extension "${extension.id}" declares ${ready.unsupported.join(", ")}, which hoocode does not support; those surfaces are ignored.`,
			);
		}

		return {
			process,
			declarations: new Map(ready.canvases.map((declaration) => [declaration.id, declaration])),
			idleSince: this.now(),
			extension,
		};
	}

	/** Drop a dead child and its instances, so a crash cannot leave stale entries. */
	private forget(extensionId: string): void {
		this.children.delete(extensionId);
		for (const [id, instance] of [...this.instances.entries()]) {
			if (instance.extensionId === extensionId) this.instances.delete(id);
		}
	}
}
