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
}

export interface EmbSearchBulkResult {
	inserted: number;
	updated: number;
}

export interface EmbSearchClientOptions {
	/** Path to the `embsearch` binary. */
	binaryPath: string;
	/** Store directory passed as `--path`. */
	storePath: string;
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

	/** Search for the top-`k` matches for `text`. */
	async query(text: string, k = 10): Promise<EmbSearchResult[]> {
		const res = await this.send({ op: "query", text, k });
		return res.results ?? [];
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
		return { modelId: res.model_id ?? "", dim: res.dim ?? 0, count: res.count ?? 0 };
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
