/**
 * Client for the `embsearch` stdio daemon (vendored from
 * github.com/kolisachint/embeddingsearchtools ts/client.ts).
 *
 * Spawns `embsearch serve` once and talks newline-delimited JSON over its
 * stdin/stdout. The process stays alive so the model and index load a single
 * time; every query after startup is hot.
 */

import { type ChildProcessWithoutNullStreams, spawn } from "child_process";

export interface EmbSearchResult {
	id: string;
	score: number;
}

export interface EmbSearchDaemonInfo {
	modelId: string;
	dim: number;
	count: number;
	/**
	 * Whether the open store carries a BM25 index alongside its vectors.
	 *
	 * Fixed when the store is created — passing `--hybrid` at a non-hybrid
	 * store only warns — so this is the only reliable way to find out, and the
	 * signal that an existing store has to be rebuilt rather than reused.
	 */
	hybrid?: boolean;
	/**
	 * Whether this daemon can actually serve `rerank`, or `undefined` from a
	 * daemon too old to say.
	 *
	 * Reported separately from the version because the two came apart:
	 * released binaries carry the `rerank` op but no longer bundle the ~23 MB
	 * cross-encoder weights, so a version check alone would advertise a
	 * reranker that fails on first call.
	 */
	rerank?: boolean;
}

/** One candidate sent for cross-encoder scoring. */
export interface EmbSearchRerankPassage {
	id: string;
	text: string;
}

/** A cross-encoder relevance logit. Higher is more relevant, but the scale is
 *  unnormalized and comparable only within one call. */
export interface EmbSearchRerankResult {
	id: string;
	score: number;
}

export interface EmbSearchBulkResult {
	inserted: number;
	updated: number;
}

/** Which daemon-side retriever answers a query (embsearch >= 0.2.0). */
export type DaemonRetriever = "dense" | "lexical" | "hybrid";

export interface EmbSearchClientOptions {
	/** Open/create the store with a BM25 lexical index alongside the vectors. */
	hybrid?: boolean;
	/** Path to the `embsearch` binary. */
	binaryPath: string;
	/** Store directory passed as `--path`. */
	storePath: string;
	/**
	 * Model directory passed as `--model` (onnx builds only): a dir holding
	 * `model.onnx`, `tokenizer.json` and `model.json`.
	 *
	 * Omitted, the daemon uses the model bundled into the binary. Set, the
	 * binary no longer determines which model produced a vector — which is why
	 * the eval harness records `info.model_id` rather than the binary version.
	 */
	modelDir?: string;
	/** Metric for a freshly created store. Default: "cosine". */
	metric?: "cosine" | "dot" | "euclidean";
}

interface Pending {
	resolve: (value: EmbSearchRawResponse) => void;
	reject: (err: Error) => void;
}

interface EmbSearchRawResponse {
	ok: boolean;
	error?: string;
	results?: EmbSearchResult[];
	inserted?: boolean;
	removed?: boolean;
	count?: number;
	inserted_count?: number;
	updated_count?: number;
	model_id?: string;
	dim?: number;
	/** Cross-encoder scores. Separate from `results` because the daemon's
	 *  logits are not comparable to retrieval scores. */
	reranked?: EmbSearchRerankResult[];
	/** `info` only: whether the open store has a BM25 index. */
	hybrid?: boolean;
	/** `info` only: whether `rerank` will work. Absent on daemons too old to
	 *  report it. */
	rerank?: boolean;
}

export class EmbSearchClient {
	private proc: ChildProcessWithoutNullStreams;
	private queue: Pending[] = [];
	private buffer = "";
	private closed = false;
	private readyPromise: Promise<void>;

	constructor(opts: EmbSearchClientOptions) {
		const args = ["serve", "--path", opts.storePath];
		if (opts.metric) args.push("--metric", opts.metric);
		if (opts.modelDir) args.push("--model", opts.modelDir);
		// Hybrid-ness is fixed when a store is created: passing --hybrid against
		// an existing non-hybrid store warns and is ignored daemon-side, so a
		// hybrid store needs its own directory.
		if (opts.hybrid) args.push("--hybrid");

		this.proc = spawn(opts.binaryPath, args, { stdio: ["pipe", "pipe", "pipe"] });

		this.proc.stdout.setEncoding("utf8");
		this.proc.stdout.on("data", (chunk: string) => this.onStdout(chunk));
		// Swallow the informational stderr banner; errors surface via responses/exit.
		this.proc.stderr.resume();

		// Readiness = the daemon answering a ping (probes the actual request loop).
		this.readyPromise = this.send({ op: "ping" }).then(() => undefined);
		// Spawn failures reject the pending ping via the exit handler; mark the
		// promise handled so a failed spawn doesn't raise an unhandled rejection
		// before ready() is awaited.
		this.readyPromise.catch(() => {});

		this.proc.on("error", (err) => {
			this.closed = true;
			for (const p of this.queue.splice(0)) p.reject(err);
		});
		this.proc.on("exit", (code) => {
			this.closed = true;
			const err = new Error(`embsearch daemon exited (code ${code})`);
			for (const p of this.queue.splice(0)) p.reject(err);
		});
	}

	/** Resolves once the daemon has loaded the model + index. */
	ready(): Promise<void> {
		return this.readyPromise;
	}

	get isClosed(): boolean {
		return this.closed;
	}

	private onStdout(chunk: string): void {
		this.buffer += chunk;
		// Each response is one line; dispatch FIFO against the pending queue.
		for (let nl = this.buffer.indexOf("\n"); nl !== -1; nl = this.buffer.indexOf("\n")) {
			const line = this.buffer.slice(0, nl).trim();
			this.buffer = this.buffer.slice(nl + 1);
			if (!line) continue;
			const pending = this.queue.shift();
			if (!pending) continue;
			let msg: EmbSearchRawResponse;
			try {
				msg = JSON.parse(line) as EmbSearchRawResponse;
			} catch {
				pending.reject(new Error(`bad response: ${line}`));
				continue;
			}
			if (msg.ok) pending.resolve(msg);
			else pending.reject(new Error(msg.error ?? "unknown error"));
		}
	}

	private send(req: Record<string, unknown>): Promise<EmbSearchRawResponse> {
		if (this.closed) return Promise.reject(new Error("embsearch client is closed"));
		return new Promise<EmbSearchRawResponse>((resolve, reject) => {
			this.queue.push({ resolve, reject });
			this.proc.stdin.write(`${JSON.stringify(req)}\n`);
		});
	}

	/**
	 * Search for the top-`k` matches for `text`.
	 *
	 * `retriever` selects which leg answers:
	 *  - `dense` (default) — vector search, the historical behaviour;
	 *  - `lexical` — BM25 only, raw scores, no embedding computed;
	 *  - `hybrid` — both, pre-fused by the daemon's own RRF constant.
	 *
	 * `lexical` and `hybrid` need a store created with `--hybrid`. Prefer
	 * `lexical` over `hybrid` when fusing here: `hybrid` collapses both legs
	 * into one RRF score, discarding the per-retriever ranks the trace records
	 * and preventing n-way fusion with the grep leg.
	 */
	async query(text: string, k = 10, retriever: DaemonRetriever = "dense"): Promise<EmbSearchResult[]> {
		const res = await this.send(
			retriever === "dense" ? { op: "query", text, k } : { op: "query", text, k, retriever },
		);
		return res.results ?? [];
	}

	/**
	 * Score `passages` against `query` with the daemon's cross-encoder and
	 * return the best `k`, best first.
	 *
	 * Passages are sent inline rather than referenced by id: the caller has the
	 * exact spans it intends to show the model, and a cross-encoder scores the
	 * text it is given, so sending anything else would score the wrong thing.
	 * Requires an onnx build with reranker weights (embsearch >= 0.3.0).
	 */
	async rerank(query: string, passages: EmbSearchRerankPassage[], k: number): Promise<EmbSearchRerankResult[]> {
		const res = await this.send({ op: "rerank", query, passages, k });
		return res.reranked ?? [];
	}

	/**
	 * Batched insert-or-replace. One embedding inference for the whole batch —
	 * the fast path for bulk indexing. Keep batches modest (e.g. 32–64) so a
	 * concurrent query is not stuck behind a huge inference.
	 */
	async bulk(items: Array<{ id: string; text: string }>): Promise<EmbSearchBulkResult> {
		const res = await this.send({ op: "bulk", items });
		return { inserted: res.inserted_count ?? 0, updated: res.updated_count ?? 0 };
	}

	/** Model id, dimensionality, and live vector count of the daemon. */
	async info(): Promise<EmbSearchDaemonInfo> {
		const res = await this.send({ op: "info" });
		return {
			modelId: res.model_id ?? "",
			dim: res.dim ?? 0,
			count: res.count ?? 0,
			hybrid: res.hybrid,
			rerank: res.rerank,
		};
	}

	/** Remove a record. Resolves to `true` if it existed. */
	async remove(id: string): Promise<boolean> {
		const res = await this.send({ op: "remove", id });
		return res.removed === true;
	}

	/** Reclaim tombstoned rows left behind by `remove`. */
	async compact(): Promise<void> {
		await this.send({ op: "compact" });
	}

	/** Persist the index to the store directory. */
	async save(): Promise<void> {
		await this.send({ op: "save" });
	}

	/** Shut the daemon down, closing stdin so it exits cleanly. */
	async close(): Promise<void> {
		if (this.closed) return;
		this.proc.stdin.end();
		await new Promise<void>((resolve) => this.proc.on("exit", () => resolve()));
	}
}
