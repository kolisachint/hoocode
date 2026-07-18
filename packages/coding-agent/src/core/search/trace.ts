/**
 * Search trace sink (docs/hybrid-retrieval-design.md, Decision 5).
 *
 * Full per-call diagnostics — resolved mode, per-retriever latency/hits,
 * fused ranks, raw scores — go to a jsonl sidecar in the embsearch store dir,
 * never into model context or session files. Best-effort by design: a failed
 * trace write must never fail a search.
 */

import { appendFileSync, mkdirSync, renameSync, statSync } from "fs";
import { join } from "path";
import { getEmbsearchStoreDir } from "../embsearch/index-meta.js";
import type { SearchTrace } from "./types.js";

const TRACE_FILE = "search-trace.jsonl";
/** One rotation (`.1` suffix) once the live file passes this size, so traces
 *  stay bounded without a GC pass. */
const TRACE_ROTATE_BYTES = 5 * 1024 * 1024;

export function getSearchTracePath(cwd: string): string {
	return join(getEmbsearchStoreDir(cwd), TRACE_FILE);
}

export function writeSearchTrace(cwd: string, trace: SearchTrace): void {
	try {
		const dir = getEmbsearchStoreDir(cwd);
		mkdirSync(dir, { recursive: true });
		const file = join(dir, TRACE_FILE);
		try {
			if (statSync(file).size >= TRACE_ROTATE_BYTES) renameSync(file, `${file}.1`);
		} catch {
			// Missing file — nothing to rotate.
		}
		appendFileSync(file, `${JSON.stringify(trace)}\n`);
	} catch {
		// Diagnostics only — never surface trace failures to the caller.
	}
}
