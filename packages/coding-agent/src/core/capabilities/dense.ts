/**
 * Optional dense leg for capability retrieval.
 *
 * A separate embsearch store from the repo index, for two reasons that both had
 * to hold. The repo service goes **dormant below a byte threshold**, and a
 * capability index that stops working in a small repo is worse than none — the
 * whole point is finding tools, which have nothing to do with how much code is
 * checked out. And the lifecycles do not line up: the repo index invalidates on
 * file edits, this one on a server appearing or a plugin being installed.
 *
 * Everything here is best-effort. It never downloads the binary — it uses one
 * already on disk, and if there is none, retrieval is lexical-only and says so
 * rather than waiting. Nothing on this path may block a session or a tool call.
 *
 * See docs/plugin-system-architecture.md §6.2.
 */

import { join } from "node:path";
import { getAgentDir } from "../../config.js";
import { getToolPath } from "../../utils/tools-manager.js";
import { EmbSearchClient } from "../embsearch/client.js";
import { type CapabilityDoc, capabilitySetHash } from "./registry.js";

/** The Rust mock backend's model id — semantically meaningless, never index with it. */
const MOCK_MODEL_ID = "mock-hash-v1";

/** Where a capability index for a given capability-set hash lives. */
export function capabilityStoreDir(hash: string, agentDir: string = getAgentDir()): string {
	return join(agentDir, "capability-index", hash);
}

export interface DenseHit {
	id: string;
	score: number;
}

/**
 * State of the dense leg, as reported to callers so they can say *why* a search
 * was lexical-only rather than silently returning fewer results.
 */
export type DenseState =
	| { status: "off"; reason: string }
	| { status: "building" }
	| { status: "ready"; count: number };

let client: EmbSearchClient | undefined;
let state: DenseState = { status: "off", reason: "not started" };
let building: Promise<void> | undefined;
let indexedHash: string | undefined;

export function denseState(): DenseState {
	return state;
}

/**
 * Bring the dense index up for `docs`, if it can be. Idempotent, and a no-op
 * when the same capability set is already indexed.
 *
 * Returns once the index is usable or has been ruled out. Callers that must not
 * wait should not await it — {@link denseSearch} works the moment it is ready
 * and returns nothing before that.
 */
export async function ensureDenseIndex(docs: readonly CapabilityDoc[], agentDir?: string): Promise<void> {
	const hash = capabilitySetHash(docs);
	if (indexedHash === hash && state.status === "ready") return;
	if (building) return building;

	building = (async () => {
		try {
			const binary = getToolPath("embsearch");
			if (!binary) {
				// Deliberately not `ensureTool`: downloading a model runtime because
				// someone has MCP servers installed is not a trade the user agreed to.
				state = { status: "off", reason: "embsearch binary not installed" };
				return;
			}
			if (docs.length === 0) {
				state = { status: "off", reason: "no capabilities registered" };
				return;
			}

			state = { status: "building" };
			await closeClient();
			const next = new EmbSearchClient({
				binaryPath: binary,
				storePath: capabilityStoreDir(hash, agentDir),
				hybrid: false, // The lexical leg is ours; the daemon only provides dense.
			});
			const info = await next.info();
			if (info.modelId === MOCK_MODEL_ID) {
				// The mock embedder returns hashes, not semantics. Indexing with it
				// would produce a store that looks healthy and ranks at random.
				await next.close().catch(() => {});
				state = { status: "off", reason: "embsearch is using the mock embedder" };
				return;
			}

			// The store is keyed on the content hash, so a hit means these exact
			// documents were embedded before and nothing needs re-embedding.
			if (info.count < docs.length) {
				await next.bulk(docs.map((d) => ({ id: d.id, text: `${d.name}. ${d.description}` })));
				await next.save();
			}
			client = next;
			indexedHash = hash;
			state = { status: "ready", count: docs.length };
		} catch (error) {
			state = { status: "off", reason: `embsearch unavailable: ${(error as Error).message}` };
			await closeClient();
		} finally {
			building = undefined;
		}
	})();
	return building;
}

/** Top `k` dense hits, or nothing when the leg is not ready. Never throws. */
export async function denseSearch(query: string, k = 10): Promise<DenseHit[]> {
	if (!client || state.status !== "ready") return [];
	try {
		return await client.query(query, k, "dense");
	} catch {
		return [];
	}
}

async function closeClient(): Promise<void> {
	const current = client;
	client = undefined;
	if (current) await current.close().catch(() => {});
}

/** Shut the daemon down and forget the index. Session teardown, and tests. */
export async function disposeDenseIndex(): Promise<void> {
	await closeClient();
	indexedHash = undefined;
	building = undefined;
	state = { status: "off", reason: "not started" };
}
