/**
 * Sidecar metadata for a semantic index store.
 *
 * The Rust store (manifest.json/ids.json/vectors.bin) only knows vector ids;
 * this sidecar (index-meta.json, written next to the store) tracks the source
 * side: per-file freshness (mtime+size, content hash recomputed only when they
 * change) and each file's chunk line-ranges so search hits can be rendered as
 * `path:start-end`. It also pins the chunker version and embedding model id —
 * a mismatch on either triggers a clean rebuild instead of an inconsistent
 * incremental update.
 */

import { createHash } from "crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "fs";
import { homedir } from "os";
import path, { join } from "path";
import { getAgentDir } from "../../config.js";
import { CHUNKER_VERSION } from "./chunker.js";

const META_FILE = "index-meta.json";
const META_FORMAT_VERSION = 1;

export interface FileMeta {
	mtimeMs: number;
	size: number;
	/** SHA-256 of content, hex. Recomputed only when mtime/size differ. */
	hash: string;
	/** Per-chunk 1-based inclusive [start, end] line ranges; index = chunk number. */
	chunks: Array<[number, number]>;
}

export interface IndexMeta {
	formatVersion: number;
	chunkerVersion: number;
	modelId: string;
	/** Absolute repo root this index was built from. */
	repoRoot: string;
	/** Last time this index was opened (ms). Enables later GC of dead stores. */
	lastUsedMs: number;
	files: Record<string, FileMeta>;
}

/** Directory holding the vector store + sidecar for `repoRoot`. */
export function getEmbsearchStoreDir(repoRoot: string): string {
	const resolved = path.resolve(repoRoot);
	const hash = createHash("sha256").update(resolved).digest("hex").slice(0, 16);
	return join(getAgentDir(), "embsearch", hash);
}

/** Subdirectory the Rust store lives in (sidecar sits next to it). */
export function getVectorStoreDir(storeDir: string): string {
	return join(storeDir, "store");
}

export function emptyIndexMeta(repoRoot: string, modelId: string): IndexMeta {
	return {
		formatVersion: META_FORMAT_VERSION,
		chunkerVersion: CHUNKER_VERSION,
		modelId,
		repoRoot: path.resolve(repoRoot),
		lastUsedMs: Date.now(),
		files: {},
	};
}

/**
 * Load the sidecar. Returns undefined when absent, unreadable, or built with a
 * different format/chunker/model — callers treat all of those as "rebuild".
 */
export function loadIndexMeta(storeDir: string, modelId: string): IndexMeta | undefined {
	const file = join(storeDir, META_FILE);
	if (!existsSync(file)) return undefined;
	let meta: IndexMeta;
	try {
		meta = JSON.parse(readFileSync(file, "utf-8")) as IndexMeta;
	} catch {
		return undefined;
	}
	if (
		meta.formatVersion !== META_FORMAT_VERSION ||
		meta.chunkerVersion !== CHUNKER_VERSION ||
		meta.modelId !== modelId
	) {
		return undefined;
	}
	return meta;
}

/** Atomically persist the sidecar (temp + rename, matching the Rust store). */
export function saveIndexMeta(storeDir: string, meta: IndexMeta): void {
	mkdirSync(storeDir, { recursive: true });
	const file = join(storeDir, META_FILE);
	const tmp = `${file}.tmp`;
	writeFileSync(tmp, JSON.stringify(meta));
	renameSync(tmp, file);
}

export function hashContent(content: string): string {
	return createHash("sha256").update(content).digest("hex");
}

/** Display helper: `~/.hoocode/embsearch/<hash>` instead of the absolute path. */
export function shortenStoreDir(storeDir: string): string {
	const home = homedir();
	return storeDir.startsWith(home) ? `~${storeDir.slice(home.length)}` : storeDir;
}
