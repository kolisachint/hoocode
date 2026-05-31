import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { getDispatchTaskDir } from "../config.js";

export interface VerificationResult {
	valid: boolean;
	reason?: string;
}

const VALID_STATUSES = ["complete", "partial", "failed"] as const;

type Status = (typeof VALID_STATUSES)[number];

/**
 * Verifies that a subagent's result.json exists, matches the expected schema,
 * and meets quality thresholds (non-empty summary, confidence >= 0.5).
 */
export class OutputVerifier {
	constructor(private readonly defaultCwd: string = process.cwd()) {}

	/**
	 * Verify the output for a given task.
	 * @param task_id The task identifier.
	 * @param cwd Optional working directory override (defaults to constructor value).
	 */
	verify(task_id: string, cwd?: string): VerificationResult {
		const base = cwd ?? this.defaultCwd;
		const path = join(getDispatchTaskDir(base, task_id), "result.json");

		if (!existsSync(path)) {
			return {
				valid: false,
				reason: `result.json not found for task ${task_id}`,
			};
		}

		let raw: string;
		try {
			raw = readFileSync(path, "utf-8");
		} catch {
			return {
				valid: false,
				reason: `Cannot read result.json for task ${task_id}`,
			};
		}

		let parsed: unknown;
		try {
			parsed = JSON.parse(raw);
		} catch {
			return {
				valid: false,
				reason: `Invalid JSON in result.json for task ${task_id}`,
			};
		}

		if (!parsed || typeof parsed !== "object") {
			return {
				valid: false,
				reason: `result.json is not an object for task ${task_id}`,
			};
		}

		const result = parsed as Record<string, unknown>;

		// summary
		if (typeof result.summary !== "string") {
			return {
				valid: false,
				reason: `Missing or invalid 'summary' in result.json for task ${task_id}`,
			};
		}
		if (result.summary.trim().length === 0) {
			return {
				valid: false,
				reason: `Empty 'summary' in result.json for task ${task_id}`,
			};
		}

		// files_changed
		if (!Array.isArray(result.files_changed)) {
			return {
				valid: false,
				reason: `Missing or invalid 'files_changed' in result.json for task ${task_id}`,
			};
		}
		if (!result.files_changed.every((f) => typeof f === "string")) {
			return {
				valid: false,
				reason: `Non-string entries in 'files_changed' for task ${task_id}`,
			};
		}

		// confidence
		if (typeof result.confidence !== "number") {
			return {
				valid: false,
				reason: `Missing or invalid 'confidence' in result.json for task ${task_id}`,
			};
		}
		if (result.confidence < 0.5) {
			return {
				valid: false,
				reason: `Confidence ${result.confidence} below threshold (0.5) for task ${task_id}`,
			};
		}

		// status
		if (typeof result.status !== "string") {
			return {
				valid: false,
				reason: `Missing or invalid 'status' in result.json for task ${task_id}`,
			};
		}
		if (!VALID_STATUSES.includes(result.status as Status)) {
			return {
				valid: false,
				reason: `Invalid status '${result.status}' in result.json for task ${task_id}`,
			};
		}

		return { valid: true };
	}
}
