/**
 * Orchestrates semantic indexing and search for a repository.
 *
 * Lifecycle (all behind --enable-embsearchtools):
 *  1. `start()` — resolve the embsearch binary, scan the repo (ignore-aware),
 *     apply the byte threshold. Under threshold → dormant. Over → spawn the
 *     daemon, verify the backend is not the mock embedder, then index changed
 *     files in the background in small batches, reporting progress.
 *  2. `search()` — top-k semantic query, mapping chunk ids back to
 *     `path:start-end` via the sidecar metadata.
 *  3. `dispose()` — save + close the daemon.
 *
 * Every failure degrades to `unavailable` with a reason; nothing here ever
 * blocks session startup or affects grep/find.
 */

import { execFileSync } from "child_process";
import { readFileSync } from "fs";
import { minimatch } from "minimatch";
import { ensureTool } from "../../utils/tools-manager.js";
import { chunkFile } from "./chunker.js";
import {
	type DaemonRetriever,
	EmbSearchClient,
	type EmbSearchRerankPassage,
	type EmbSearchRerankResult,
} from "./client.js";
import {
	emptyIndexMeta,
	type FileMeta,
	getEmbsearchStoreDir,
	getVectorStoreDir,
	hashContent,
	type IndexMeta,
	loadIndexMeta,
	saveIndexMeta,
} from "./index-meta.js";
import { type RepoScanFile, scanRepo } from "./repo-scan.js";

/** Chunks per bulk request. Small enough that a concurrent query is never
 *  stuck long behind one padded batch inference. */
const BULK_BATCH_SIZE = 48;
/** Yield between batches so background indexing doesn't starve the session. */
const BATCH_YIELD_MS = 15;
/** The Rust mock backend's model id — semantically meaningless, never index with it. */
const MOCK_MODEL_ID = "mock-hash-v1";
/**
 * First embsearch release serving `retriever: "lexical"`.
 *
 * The guard matters because the daemon does not reject unknown request fields:
 * an older binary silently ignores `retriever` and answers with dense results.
 * Fusing that list a second time as a "bm25" leg would double-count it and
 * corrupt the ranking with no error anywhere — so refuse instead of degrading.
 */
const MIN_LEXICAL_RETRIEVER_VERSION = [0, 2, 0] as const;
/**
 * First embsearch release serving the `rerank` op.
 *
 * Necessary but no longer sufficient. Releases from 0.3.1 carry the op without
 * the ~23 MB cross-encoder weights — they measured worse than the
 * deterministic reranker on five of six query classes, so they are no longer
 * bundled — and such a daemon answers `rerank` with an error. The version is
 * therefore only the floor for *asking*; `info.rerank` is the answer, and
 * {@link EmbsearchService.supportsCrossEncoder} needs both.
 */
const MIN_RERANK_VERSION = [0, 3, 0] as const;

/** `embsearch 0.2.0` -> [0, 2, 0]; undefined when it cannot be parsed. */
function parseBinaryVersion(output: string): number[] | undefined {
	const match = output.match(/(\d+)\.(\d+)\.(\d+)/);
	return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : undefined;
}

function atLeast(version: readonly number[], minimum: readonly number[]): boolean {
	for (let i = 0; i < minimum.length; i++) {
		const part = version[i] ?? 0;
		if (part !== minimum[i]) return part > minimum[i];
	}
	return true;
}

export type EmbsearchState =
	| { phase: "idle" }
	| { phase: "skipped"; reason: string }
	| { phase: "downloading"; receivedBytes: number; totalBytes: number | null }
	| { phase: "indexing"; done: number; total: number }
	| { phase: "ready"; chunkCount: number }
	| { phase: "unavailable"; reason: string };

export interface EmbsearchServiceOptions {
	cwd: string;
	/** Explicit binary path (settings override). Default: "embsearch" from PATH. */
	binaryPath?: string;
	/** Minimum indexable bytes before indexing kicks in. */
	thresholdBytes: number;
	/**
	 * Override the store location. Only the eval harness sets this, so a
	 * second index (e.g. a BM25-hybrid store) can exist for the same repo
	 * without colliding with the primary one.
	 */
	storeDir?: string;
	/**
	 * Create the store with the daemon's BM25 lexical index. Fixed at store
	 * creation, so this must pair with a distinct `storeDir`.
	 */
	hybridStore?: boolean;
	/** Progress callback for UI (footer / stderr lines). */
	onProgress?: (state: EmbsearchState) => void;
}

export interface SemanticHit {
	path: string;
	startLine: number;
	endLine: number;
	score: number;
}

export interface SemanticChunkHit extends SemanticHit {
	/** Per-build chunk id (`relpath#index`) — the fusion identity for hybrid search. */
	id: string;
}

export class EmbsearchService {
	private readonly options: EmbsearchServiceOptions;
	private client: EmbSearchClient | undefined;
	private meta: IndexMeta | undefined;
	private state: EmbsearchState = { phase: "idle" };
	private disposed = false;
	/** Whether the resolved binary serves `retriever: "lexical"`. */
	private lexicalRetriever = false;
	/** Whether the resolved binary serves the cross-encoder `rerank` op. */
	private crossEncoder = false;

	constructor(options: EmbsearchServiceOptions) {
		this.options = options;
	}

	getState(): EmbsearchState {
		return this.state;
	}

	/** Semantic search is usable (index ready, or still building with partial data). */
	isAvailable(): boolean {
		return this.state.phase === "ready" || this.state.phase === "indexing";
	}

	private setState(state: EmbsearchState): void {
		this.state = state;
		this.options.onProgress?.(state);
	}

	/** Whether the running daemon can serve a BM25-only query. */
	supportsLexicalRetriever(): boolean {
		return this.lexicalRetriever;
	}

	/**
	 * Repo files whose on-disk content the index does not have — unknown to it,
	 * or changed since it last read them.
	 *
	 * This is the set BM25 is structurally blind to, and the only place the
	 * grep leg still earns its keep once BM25 is available. An agent that edits
	 * a file and immediately searches for what it wrote is asking about exactly
	 * these files; the index cannot answer until the next pass.
	 *
	 * Compares mtime and size only, never hashing: the check runs per query, and
	 * a false positive merely lets grep cover a file BM25 already covers, while
	 * a false negative would lose the edit.
	 *
	 * Deliberately uncached. A cache here caches the *absence* of an edit, which
	 * is the one thing this must never do — an agent writes a file and searches
	 * for it in the same breath. A 1s TTL was tried and cost the live-edit set
	 * 75% to 100% of its score depending on how the timing fell, which is worse
	 * than wrong: it was non-deterministic. One scan is ~25ms over ~1k files and
	 * happens once per search, against retrieval that already costs more.
	 */
	staleFiles(signal?: AbortSignal): string[] {
		if (!this.meta) return [];
		const meta = this.meta;
		const files: string[] = [];
		try {
			for (const file of scanRepo(this.options.cwd, signal).files) {
				const known = meta.files[file.rel];
				if (!known || known.mtimeMs !== file.mtimeMs || known.size !== file.size) files.push(file.rel);
			}
		} catch {
			// A failed scan must not silently narrow the grep leg to nothing;
			// report no staleness and let the indexed legs answer.
			return [];
		}
		return files;
	}

	/** Whether the running daemon can score with a cross-encoder. */
	supportsCrossEncoder(): boolean {
		return this.crossEncoder;
	}

	/**
	 * Cross-encoder rerank of caller-supplied passages.
	 *
	 * Unlike the retrievers this does not consult the index at all — it scores
	 * exactly the text passed in, which is why the caller sends its expanded
	 * windows rather than chunk ids.
	 */
	async rerank(query: string, passages: EmbSearchRerankPassage[], k: number): Promise<EmbSearchRerankResult[]> {
		if (!this.client || this.client.isClosed) {
			throw new Error("semantic index is not available");
		}
		if (!this.crossEncoder) {
			throw new Error(
				`this embsearch daemon cannot rerank (needs >= ${MIN_RERANK_VERSION.join(".")} reporting ` +
					"`rerank: true`); released binaries ship without cross-encoder weights — start the daemon " +
					"with --reranker-model <dir>",
			);
		}
		return await this.client.rerank(query, passages, k);
	}

	private probeBinaryVersion(binary: string): number[] | undefined {
		try {
			return parseBinaryVersion(execFileSync(binary, ["--version"], { encoding: "utf-8", timeout: 10_000 }));
		} catch {
			return undefined;
		}
	}

	private async resolveBinary(): Promise<string | undefined> {
		if (this.options.binaryPath) {
			return this.options.binaryPath;
		}
		// Surface the on-demand binary download through the same progress channel as
		// indexing, so the first-run fetch renders a progress bar instead of a stall.
		// A cached binary resolves without ever invoking this callback.
		return await ensureTool("embsearch", true, (receivedBytes, totalBytes) => {
			this.setState({ phase: "downloading", receivedBytes, totalBytes });
		});
	}

	/**
	 * Scan, threshold-check, and (when needed) index in the background.
	 * Resolves when indexing completes or the feature settles dormant.
	 */
	async start(signal?: AbortSignal): Promise<void> {
		try {
			await this.run(signal);
		} catch (e) {
			const reason = e instanceof Error ? e.message : String(e);
			this.setState({ phase: "unavailable", reason });
			await this.closeClient();
		}
	}

	private async run(signal?: AbortSignal): Promise<void> {
		const binary = await this.resolveBinary();
		if (!binary) {
			this.setState({
				phase: "unavailable",
				reason: "embsearch binary not found (PATH or embsearchBinaryPath setting)",
			});
			return;
		}

		const scan = scanRepo(this.options.cwd, signal);
		if (scan.totalBytes < this.options.thresholdBytes) {
			this.setState({
				phase: "skipped",
				reason: `repo under threshold (${scan.totalBytes} < ${this.options.thresholdBytes} bytes)`,
			});
			return;
		}

		const binaryVersion = this.probeBinaryVersion(binary);
		this.lexicalRetriever = binaryVersion !== undefined && atLeast(binaryVersion, MIN_LEXICAL_RETRIEVER_VERSION);
		// Provisional: the version says the daemon understands `rerank`. Whether
		// it can serve one is answered by `info` below, once the client is up.
		this.crossEncoder = binaryVersion !== undefined && atLeast(binaryVersion, MIN_RERANK_VERSION);

		const storeDir = this.options.storeDir ?? getEmbsearchStoreDir(this.options.cwd);
		this.client = new EmbSearchClient({
			binaryPath: binary,
			storePath: getVectorStoreDir(storeDir),
			hybrid: this.options.hybridStore,
		});
		await this.client.ready();

		const info = await this.client.info();
		// A daemon old enough to omit the field is left on the version verdict —
		// back then the weights were bundled, so version did imply capability.
		if (info.rerank === false) this.crossEncoder = false;
		if (info.modelId === MOCK_MODEL_ID) {
			throw new Error("embsearch binary uses the mock embedder (not semantic); install an onnx build");
		}

		// Missing/stale sidecar (format, chunker, or model changed) → clean rebuild.
		this.meta = loadIndexMeta(storeDir, info.modelId) ?? emptyIndexMeta(this.options.cwd, info.modelId);
		this.meta.lastUsedMs = Date.now();

		await this.indexChangedFiles(scan.files, storeDir, signal);
	}

	private async indexChangedFiles(files: RepoScanFile[], storeDir: string, signal?: AbortSignal): Promise<void> {
		const meta = this.meta!;
		const client = this.client!;

		// Diff scan vs sidecar: cheap mtime+size check first, hash only on delta.
		const toIndex: Array<{ file: RepoScanFile; content: string; hash: string }> = [];
		const seen = new Set<string>();
		for (const file of files) {
			seen.add(file.rel);
			const known = meta.files[file.rel];
			if (known && known.mtimeMs === file.mtimeMs && known.size === file.size) continue;
			let content: string;
			try {
				content = readFileSync(file.abs, "utf-8");
			} catch {
				continue;
			}
			const hash = hashContent(content);
			if (known && known.hash === hash) {
				// Touched but unchanged — refresh stat info only.
				known.mtimeMs = file.mtimeMs;
				known.size = file.size;
				continue;
			}
			toIndex.push({ file, content, hash });
		}
		const toRemove = Object.keys(meta.files).filter((rel) => !seen.has(rel));

		if (toIndex.length === 0 && toRemove.length === 0) {
			saveIndexMeta(storeDir, meta);
			this.setState({ phase: "ready", chunkCount: this.countChunks(meta) });
			return;
		}

		// Chunk changed files; count total upserts for exact progress.
		const work: Array<{ rel: string; fileMeta: FileMeta; chunks: Array<{ id: string; text: string }> }> = [];
		let totalChunks = 0;
		for (const { file, content, hash } of toIndex) {
			const chunks = chunkFile(file.rel, content);
			work.push({
				rel: file.rel,
				fileMeta: {
					mtimeMs: file.mtimeMs,
					size: file.size,
					hash,
					chunks: chunks.map((c) => [c.startLine, c.endLine]),
				},
				chunks: chunks.map((c) => ({ id: c.id, text: c.text })),
			});
			totalChunks += chunks.length;
		}

		this.setState({ phase: "indexing", done: 0, total: totalChunks });

		// Drop vectors of deleted files and superseded chunk tails.
		for (const rel of toRemove) {
			for (let i = 0; i < meta.files[rel].chunks.length; i++) await client.remove(`${rel}#${i}`);
			delete meta.files[rel];
		}

		let done = 0;
		for (const item of work) {
			if (signal?.aborted || this.disposed) return;
			const oldChunkCount = meta.files[item.rel]?.chunks.length ?? 0;
			// Remove old chunks beyond the new count (upsert covers the rest).
			for (let i = item.chunks.length; i < oldChunkCount; i++) await client.remove(`${item.rel}#${i}`);

			for (let offset = 0; offset < item.chunks.length; offset += BULK_BATCH_SIZE) {
				if (signal?.aborted || this.disposed) return;
				const batch = item.chunks.slice(offset, offset + BULK_BATCH_SIZE);
				await client.bulk(batch);
				done += batch.length;
				this.setState({ phase: "indexing", done, total: totalChunks });
				// Yield so queries and the event loop stay responsive.
				await new Promise((resolve) => setTimeout(resolve, BATCH_YIELD_MS));
			}
			meta.files[item.rel] = item.fileMeta;
		}

		await client.compact();
		await client.save();
		saveIndexMeta(storeDir, meta);
		this.setState({ phase: "ready", chunkCount: this.countChunks(meta) });
	}

	private countChunks(meta: IndexMeta): number {
		let n = 0;
		for (const rel of Object.keys(meta.files)) n += meta.files[rel].chunks.length;
		return n;
	}

	/** Top-`k` semantic hits as `path` + line range + score. */
	async search(query: string, k = 10): Promise<SemanticHit[]> {
		return await this.searchChunks(query, k);
	}

	/** Top-`k` semantic hits including their chunk ids, for rank fusion. */
	async searchChunks(
		query: string,
		k = 10,
		glob?: string,
		retriever: DaemonRetriever = "dense",
	): Promise<SemanticChunkHit[]> {
		if (!this.client || this.client.isClosed || !this.meta) {
			throw new Error("semantic index is not available");
		}
		if (retriever === "lexical" && !this.lexicalRetriever) {
			throw new Error(
				`embsearch is too old for retriever "lexical" (needs >= ${MIN_LEXICAL_RETRIEVER_VERSION.join(".")}); ` +
					"an older daemon ignores the field and answers with dense results",
			);
		}
		const results = await this.client.query(query, k, retriever);
		const hits: SemanticChunkHit[] = [];
		const matchGlob = (rel: string): boolean => {
			if (!glob) return true;
			return minimatch(rel, glob, { dot: true, matchBase: !glob.includes("/") });
		};
		for (const result of results) {
			const sep = result.id.lastIndexOf("#");
			if (sep === -1) continue;
			const rel = result.id.slice(0, sep);
			if (!matchGlob(rel)) continue;
			const chunkIndex = Number.parseInt(result.id.slice(sep + 1), 10);
			const range = this.meta.files[rel]?.chunks[chunkIndex];
			if (!range) continue;
			hits.push({ id: result.id, path: rel, startLine: range[0], endLine: range[1], score: result.score });
		}
		return hits;
	}

	/**
	 * Resolve a repo-relative path + line to its enclosing indexed chunk, or
	 * undefined when the file/line is not covered by the index. Chunks overlap
	 * by a few lines; the first (lowest-index) containing chunk wins so the
	 * mapping is deterministic.
	 */
	findEnclosingChunk(
		rel: string,
		line: number,
	): { id: string; path: string; startLine: number; endLine: number } | undefined {
		const file = this.meta?.files[rel];
		if (!file) return undefined;
		for (let i = 0; i < file.chunks.length; i++) {
			const [startLine, endLine] = file.chunks[i];
			if (line >= startLine && line <= endLine) {
				return { id: `${rel}#${i}`, path: rel, startLine, endLine };
			}
		}
		return undefined;
	}

	private async closeClient(): Promise<void> {
		const client = this.client;
		this.client = undefined;
		if (client && !client.isClosed) {
			try {
				await client.close();
			} catch {
				// already dead
			}
		}
	}

	/** Persist state and shut the daemon down. Safe to call twice. */
	async dispose(): Promise<void> {
		if (this.disposed) return;
		this.disposed = true;
		if (this.client && !this.client.isClosed) {
			try {
				await this.client.save();
			} catch {
				// daemon may have exited; nothing to save
			}
		}
		await this.closeClient();
	}
}

// --- Per-cwd service registry ---
//
// The search tool is constructed by the generic tool factory table and
// only receives `cwd`; the service is created later during session init (flag
// gated). This registry connects the two without threading a service instance
// through every layer between main.ts and the tool factories.

const services = new Map<string, EmbsearchService>();

export function registerEmbsearchService(cwd: string, service: EmbsearchService): void {
	const old = services.get(cwd);
	if (old && old !== service) {
		old.dispose().catch(() => {});
	}
	services.set(cwd, service);
}

export function getEmbsearchService(cwd: string): EmbsearchService | undefined {
	return services.get(cwd);
}

export function unregisterEmbsearchService(cwd: string): void {
	services.delete(cwd);
}
