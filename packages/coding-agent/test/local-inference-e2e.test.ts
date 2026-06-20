/**
 * End-to-end test against a real local MLX server.
 *
 * Skipped unless a server is reachable at HOOCODE_MLX_E2E_URL (default
 * http://127.0.0.1:8080/v1) serving HOOCODE_MLX_E2E_MODEL (default
 * mlx-community/Qwen3-4B-4bit). Run manually:
 *
 *   mlx_lm.server --model mlx-community/Qwen3-4B-4bit --host 127.0.0.1 --port 8080
 *   npx tsx ../../node_modules/vitest/dist/cli.js --run test/local-inference-e2e.test.ts
 *
 * Verifies the real provider pipeline: promptSuffix "/no_think" disables Qwen3
 * thinking, and per-tool extractive compression actually shrinks read/bash
 * output while preserving critical facts.
 */

import { completeSimple, type Model } from "@kolisachint/hoocode-ai";
import { beforeAll, describe, expect, it } from "vitest";
import {
	buildToolResultPrompt,
	stripThinkTags,
	TOOL_RESULT_SYSTEM_PROMPT,
} from "../src/core/routing/tool-result-prompts.js";

const BASE_URL = process.env.HOOCODE_MLX_E2E_URL ?? "http://127.0.0.1:8080/v1";
const MODEL_ID = process.env.HOOCODE_MLX_E2E_MODEL ?? "mlx-community/Qwen3-4B-4bit";

let serverUp = false;

async function probe(): Promise<boolean> {
	try {
		const res = await fetch(`${BASE_URL.replace(/\/v1$/, "")}/v1/models`, { method: "GET" });
		return res.ok;
	} catch {
		return false;
	}
}

beforeAll(async () => {
	serverUp = await probe();
});

function executorModel(promptSuffix?: string): Model<"openai-completions"> {
	return {
		id: MODEL_ID,
		name: "mlx-executor",
		api: "openai-completions",
		provider: "mlx",
		baseUrl: BASE_URL,
		reasoning: true,
		input: ["text"],
		cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
		contextWindow: 32000,
		maxTokens: 768,
		compat: { promptSuffix },
	};
}

async function compress(toolName: string, output: string, suffix?: string): Promise<string> {
	const prompt = buildToolResultPrompt(toolName, output);
	if (!prompt) throw new Error(`no prompt for ${toolName}`);
	const res = await completeSimple(
		executorModel(suffix),
		{
			systemPrompt: TOOL_RESULT_SYSTEM_PROMPT,
			messages: [{ role: "user", content: [{ type: "text", text: prompt }], timestamp: Date.now() }],
		},
		{ apiKey: "not-needed" },
	);
	expect(res.stopReason).not.toBe("error");
	const raw = res.content
		.filter((c): c is { type: "text"; text: string } => c.type === "text")
		.map((c) => c.text)
		.join("\n");
	return stripThinkTags(raw);
}

const BASH_OUTPUT = `TOOL bash(npm test):
${Array.from({ length: 60 }, (_, i) => `  PASS test/unit/mod_${i}.test.ts (${(i % 9) + 1} tests, 0.${i % 9}s)`).join("\n")}
  FAIL test/auth/refresh.test.ts
    Expected AuthError 'EXPIRED_REFRESH' received TypeError at oauth-client.ts:48:22
  Tests: 1 failed, 60 passed, 61 total
  Time: 8.412s
  exit code: 1`;

// Realistic file: declarations interleaved with body-statement noise (as in real
// source), not all noise up front. The extractive prompt should drop the body
// statements and keep declarations/throws/TODOs.
function buildReadOutput(): string {
	const lines: string[] = ["TOOL read(src/auth/oauth-client.ts):", "  12:   export class OAuthClient {"];
	let n = 13;
	const noise = (count: number) => {
		for (let i = 0; i < count; i++) lines.push(`  ${n++}:     logger.debug("step ${n}"); await flush();`);
	};
	noise(8);
	lines.push(`  ${n++}:   private tokenUrl = "https://auth.example.com/oauth/token";`);
	noise(8);
	lines.push(`  ${n++}:   async refreshToken(token: string): Promise<TokenResponse> {`);
	noise(4);
	lines.push(`  ${n++}:     if (res.status === 401) throw new AuthError("EXPIRED_REFRESH");`);
	noise(8);
	lines.push(`  ${n++}:   private backoffMs = 1500;`);
	noise(8);
	lines.push(`  ${n++}:   // TODO: handle clock skew of 300s`);
	return lines.join("\n");
}
const READ_OUTPUT = buildReadOutput();

/**
 * Count how many of the given facts survive in the output. The 4B executor is
 * ~88% faithful (see docs/local-executor-routing.md), so e2e asserts a high
 * retention threshold rather than every individual fact (which is probabilistic).
 * Deterministic guarantees (no think tags, real compression, no error) are
 * asserted exactly.
 */
function retained(out: string, facts: string[]): number {
	return facts.filter((f) => out.includes(f)).length;
}

describe("local-inference e2e (real MLX server)", () => {
	it("compresses bash output via /no_think, stripping think tags and retaining facts", async () => {
		if (!serverUp) {
			console.warn(`[e2e] skipped: no MLX server at ${BASE_URL}`);
			return;
		}
		const out = await compress("bash", BASH_OUTPUT, "/no_think");
		// Deterministic guarantees.
		expect(out).not.toContain("<think>");
		expect(out).not.toContain("</think>");
		expect(out.length).toBeLessThan(BASH_OUTPUT.length);
		// Probabilistic retention: keep most critical facts (>= 3 of 4).
		const facts = ["refresh.test.ts", "EXPIRED_REFRESH", "oauth-client.ts:48:22", "exit code: 1"];
		expect(retained(out, facts)).toBeGreaterThanOrEqual(3);
	}, 120_000);

	it("compresses read output, dropping body noise and keeping declarations", async () => {
		if (!serverUp) {
			console.warn(`[e2e] skipped: no MLX server at ${BASE_URL}`);
			return;
		}
		const out = await compress("read", READ_OUTPUT, "/no_think");
		// Deterministic guarantees.
		expect(out).not.toContain("<think>");
		expect(out).toContain("oauth-client.ts");
		expect(out.length).toBeLessThan(READ_OUTPUT.length);
		// Probabilistic retention: keep most declarations (>= 3 of 4).
		const facts = ["refreshToken", "EXPIRED_REFRESH", "1500", "300s"];
		expect(retained(out, facts)).toBeGreaterThanOrEqual(3);
	}, 120_000);
});
