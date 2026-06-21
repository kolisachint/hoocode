/**
 * Local-inference routing.
 *
 * Optional, opt-in routing of certain non-critical work (conversation
 * compaction, and large bash tool-result compression) to a local
 * "executor" model running on an OpenAI-compatible endpoint (for example an
 * MLX server), while the primary model handles all planning, reasoning, edits,
 * and tool-call synthesis.
 *
 * Everything here is INERT unless explicitly enabled via the
 * `--enable-local-inference` flag or the `HOOCODE_ROUTING_MODE` env var. On any
 * executor resolution/availability problem the caller falls back to the primary
 * model (compaction) or the raw tool result (tool-result compression). The
 * router never throws into the agent loop.
 *
 * Design and validation: see docs/local-executor-routing.md.
 */

import type { Api, Model } from "@kolisachint/hoocode-ai";
import type { ModelRegistry } from "../model-registry.js";

/** Work that may be routed to the executor instead of the primary model. */
export type TurnKind = "primary" | "summarization" | "tool-result";

export type RoutingMode =
	| "primary-only"
	| "executor-for-summarization"
	| "executor-for-tool-results";

export const ROUTING_MODES: readonly RoutingMode[] = [
	"primary-only",
	"executor-for-summarization",
	"executor-for-tool-results",
] as const;

/**
 * Tool names whose output is worth compressing (validated). Others pass
 * through. Only `bash` qualifies: its verbose output is mostly low-value noise
 * around a few load-bearing facts. `read` was measured to compress ~0% on real
 * source code (every line is a keep-line) and was removed. Fact-list tools
 * (grep/find/ls) were never compressible (every line is a distinct fact).
 */
export const COMPRESSIBLE_TOOLS = new Set(["bash"]);

/**
 * Global size band (bytes) for local-inference routing. Applies to BOTH
 * tool-result compression and compaction summarization. Inputs below the
 * minimum are not worth offloading; inputs above the maximum are slow and risk
 * GPU OOM on small machines, so they fall back to the primary model. Tunable
 * per-machine via `minBytes`/`maxBytes` in the executor config block.
 */
export const DEFAULT_MIN_BYTES = 2048;
export const DEFAULT_MAX_BYTES = 8192;

/** Optional local server the harness manages for the executor. */
export interface ExecutorServerConfig {
	/** Command to launch (default: "mlx_lm.server"). */
	command?: string;
	/** Extra args appended to the launch command. */
	args?: string[];
	/** Host to health-check and bind (default: derived from executor baseUrl or 127.0.0.1). */
	host?: string;
	/** Port to health-check and bind (default: derived from executor baseUrl or 8080). */
	port?: number;
	/** Max milliseconds to wait for the server to become healthy (default: 30000). */
	startupTimeoutMs?: number;
}

/** Executor model reference as configured in models.json. */
export interface ExecutorConfig {
	provider: string;
	model: string;
	/** Minimum input size (bytes) before local inference is attempted. */
	minBytes?: number;
	/** Maximum input size (bytes); larger inputs fall back to the primary model. */
	maxBytes?: number;
	/** When set, the harness spawns/health-checks/stops this local server. */
	server?: ExecutorServerConfig;
}

/** `routing` block in models.json. */
export interface RoutingConfig {
	mode?: RoutingMode;
	executor?: ExecutorConfig;
}

function isRoutingMode(value: unknown): value is RoutingMode {
	return typeof value === "string" && (ROUTING_MODES as readonly string[]).includes(value);
}

/**
 * Resolve the effective routing mode from CLI flag, env var, and config.
 *
 * Activation requires either the flag or the env var; config alone never
 * activates routing (decision: explicit opt-in only). When activated without an
 * explicit mode, defaults to `executor-for-summarization` (the lowest-risk
 * mode). When not activated, always `primary-only`.
 */
export function resolveRoutingMode(opts: {
	enableFlag?: boolean;
	envMode?: string;
	configMode?: RoutingMode;
}): RoutingMode {
	const envMode = opts.envMode?.trim();
	const envActivates = envMode !== undefined && envMode !== "" && envMode !== "primary-only";
	const activated = opts.enableFlag === true || envActivates;
	if (!activated) return "primary-only";

	if (envMode && isRoutingMode(envMode)) return envMode;
	if (opts.configMode && isRoutingMode(opts.configMode)) return opts.configMode;
	return "executor-for-summarization";
}

/**
 * Router that decides, per turn kind, whether to use the executor model and
 * resolves it from the registry. Holds no mutable state beyond the resolved
 * executor model.
 */
export class LocalInferenceRouter {
	private readonly mode: RoutingMode;
	private readonly executorConfig: ExecutorConfig | undefined;
	private readonly executor: Model<Api> | undefined;
	private readonly minBytes: number;
	private readonly maxBytes: number;

	private constructor(
		mode: RoutingMode,
		executorConfig: ExecutorConfig | undefined,
		executor: Model<Api> | undefined,
	) {
		this.mode = mode;
		this.executorConfig = executorConfig;
		this.executor = executor;
		this.minBytes = executorConfig?.minBytes ?? DEFAULT_MIN_BYTES;
		this.maxBytes = executorConfig?.maxBytes ?? DEFAULT_MAX_BYTES;
	}

	static create(opts: {
		mode: RoutingMode;
		config: RoutingConfig | undefined;
		registry: ModelRegistry;
	}): LocalInferenceRouter {
		const executorConfig = opts.config?.executor;
		let executor: Model<Api> | undefined;
		if (opts.mode !== "primary-only" && executorConfig) {
			executor = opts.registry.find(executorConfig.provider, executorConfig.model);
		}
		return new LocalInferenceRouter(opts.mode, executorConfig, executor);
	}

	getMode(): RoutingMode {
		return this.mode;
	}

	/** True when routing is active and an executor model is resolved and usable. */
	isExecutorAvailable(): boolean {
		return this.mode !== "primary-only" && this.executor !== undefined;
	}

	getExecutorConfig(): ExecutorConfig | undefined {
		return this.executorConfig;
	}

	/**
	 * Pick the model to use for a turn. Returns the executor when the mode routes
	 * that turn kind and the executor is available; otherwise returns the primary
	 * model.
	 */
	selectModel(turnKind: TurnKind, primary: Model<Api>): Model<Api> {
		if (!this.isExecutorAvailable() || !this.executor) return primary;
		switch (this.mode) {
			case "executor-for-summarization":
				return turnKind === "summarization" ? this.executor : primary;
			case "executor-for-tool-results":
				return turnKind === "tool-result" ? this.executor : primary;
			case "primary-only":
				return primary;
		}
	}

	/** True when an input size falls within the configured local-inference band. */
	withinSizeBand(bytes: number): boolean {
		return bytes >= this.minBytes && bytes <= this.maxBytes;
	}

	/** The configured size band, for logging/diagnostics. */
	getSizeBand(): { minBytes: number; maxBytes: number } {
		return { minBytes: this.minBytes, maxBytes: this.maxBytes };
	}

	/** Whether a given tool's result should be compressed via the executor. */
	shouldCompressToolResult(toolName: string, contentBytes: number): boolean {
		if (this.mode !== "executor-for-tool-results") return false;
		if (!this.isExecutorAvailable()) return false;
		if (!COMPRESSIBLE_TOOLS.has(toolName)) return false;
		return this.withinSizeBand(contentBytes);
	}

	/**
	 * Whether to route summarization to the executor for a conversation of the
	 * given serialized size. Requires summarization routing active, the executor
	 * available, and the size within the band (oversized conversations fall back
	 * to the primary to avoid slow local runs and GPU OOM).
	 */
	shouldRouteSummarization(bytes: number): boolean {
		if (this.mode !== "executor-for-summarization") return false;
		if (!this.isExecutorAvailable()) return false;
		return this.withinSizeBand(bytes);
	}

	getExecutorModel(): Model<Api> | undefined {
		return this.executor;
	}
}
