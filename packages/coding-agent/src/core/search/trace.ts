/**
 * Search trace sink (docs/hybrid-retrieval-design.md, Decision 5).
 *
 * Full per-call diagnostics — resolved mode, per-retriever latency/hits,
 * fused ranks, raw scores — go to a jsonl sidecar in the embsearch store dir,
 * never into model context or session files. Best-effort by design: a failed
 * trace write must never fail a search.
 */

import { appendFileSync, mkdirSync } from "fs";
import { join } from "path";
import { getEmbsearchStoreDir } from "../embsearch/index-meta.js";
import type { SearchTrace } from "./types.js";

const TRACE_FILE = "search-trace.jsonl";

export function getSearchTracePath(cwd: string): string {
	return join(getEmbsearchStoreDir(cwd), TRACE_FILE);
}

export function writeSearchTrace(cwd: string, trace: SearchTrace): void {
	try {
		const dir = getEmbsearchStoreDir(cwd);
		mkdirSync(dir, { recursive: true });
		appendFileSync(join(dir, TRACE_FILE), `${JSON.stringify(trace)}\n`);
	} catch {
		// Diagnostics only — never surface trace failures to the caller.
	}
}
