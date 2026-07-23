import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { clampMaxTokens, createWebFetchTool } from "../src/core/tools/webfetch.js";
import { createWebSearchTool } from "../src/core/tools/websearch.js";
import {
	resolveWebtoolsTimeoutSecs,
	resolveWebtoolsTLSConfig,
	WebToolsCache,
} from "../src/core/tools/webtools-shared.js";

// A fake `webtools` binary placed on PATH. It echoes canned --json output for
// the fetch/search subcommands so the tools' spawn + parse + filter paths are
// exercised without real network access or the real Rust binary.
const FAKE_BIN = `#!/bin/sh
sub="$1"
if [ -n "$WEBTOOLS_ARGV_LOG" ]; then
  echo "$@" >> "$WEBTOOLS_ARGV_LOG"
fi
if [ "$sub" = "fetch" ]; then
  cat <<'JSON'
{
  "title": "Example Domain",
  "final_url": "https://example.com/",
  "content": "Example Domain\\nSee more [1]\\n\\nReferences:\\n[1] https://iana.org/domains/example",
  "content_type": "text",
  "media": "html",
  "token_estimate": 42,
  "references": [{ "index": 1, "url": "https://iana.org/domains/example", "text": "See more" }],
  "metadata": { "lang": "en" },
  "source": "https://example.com/"
}
JSON
elif [ "$sub" = "search" ]; then
  cat <<'JSON'
{
  "query": "rust async",
  "results": [
    { "title": "Smol runtime", "snippet": "A small async runtime.", "url": "https://github.com/smol-rs/smol", "ref_index": 1 },
    { "title": "Blocked Site", "snippet": "Should be filtered.", "url": "https://blocked.example/page", "ref_index": 2 }
  ],
  "references": [
    { "index": 1, "url": "https://github.com/smol-rs/smol" },
    { "index": 2, "url": "https://blocked.example/page" }
  ],
  "token_estimate": 50,
  "result_count": 2
}
JSON
else
  echo "unknown subcommand" 1>&2
  exit 1
fi
`;

function getText(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content
		.filter((c) => c.type === "text")
		.map((c) => c.text)
		.join("\n");
}

describe("web tools", () => {
	let binDir: string;
	let cwd: string;
	let originalPath: string | undefined;

	beforeAll(() => {
		binDir = join(tmpdir(), `webtools-fake-bin-${Date.now()}`);
		mkdirSync(binDir, { recursive: true });
		const binPath = join(binDir, "webtools");
		writeFileSync(binPath, FAKE_BIN);
		chmodSync(binPath, 0o755);
		originalPath = process.env.PATH;
		process.env.PATH = `${binDir}:${process.env.PATH ?? ""}`;
	});

	afterAll(() => {
		process.env.PATH = originalPath;
		rmSync(binDir, { recursive: true, force: true });
	});

	beforeEach(() => {
		cwd = join(tmpdir(), `webtools-cwd-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	describe("webfetch", () => {
		it("fetches a URL and returns title + final url + content", async () => {
			const tool = createWebFetchTool(cwd);
			const result = await tool.execute("call-1", { url: "https://example.com" });
			const text = getText(result);
			expect(text).toContain("Example Domain");
			expect(text).toContain("https://example.com/");
			expect(text).toContain("[1] https://iana.org/domains/example");
			expect((result.details as { tokenEstimate?: number }).tokenEstimate).toBe(42);
		});

		it("blocks a host listed in .webtoolsignore", async () => {
			writeFileSync(join(cwd, ".webtoolsignore"), "example.com\n");
			const tool = createWebFetchTool(cwd);
			await expect(tool.execute("call-2", { url: "https://example.com" })).rejects.toThrow(/\.webtoolsignore/);
		});

		it("allows a host re-included with a ! rule", async () => {
			writeFileSync(join(cwd, ".webtoolsignore"), "*\n!example.com\n");
			const tool = createWebFetchTool(cwd);
			const result = await tool.execute("call-3", { url: "https://example.com" });
			expect(getText(result)).toContain("Example Domain");
		});
	});

	describe("websearch", () => {
		it("returns ranked results in reference style", async () => {
			const tool = createWebSearchTool(cwd);
			const result = await tool.execute("call-4", { query: "rust async" });
			const text = getText(result);
			expect(text).toContain("Smol runtime [1]");
			expect(text).toContain("References:");
			expect(text).toContain("[1] https://github.com/smol-rs/smol");
		});

		it("filters result links blocked by .webtoolsignore and notes the hidden count", async () => {
			writeFileSync(join(cwd, ".webtoolsignore"), "blocked.example\n");
			const tool = createWebSearchTool(cwd);
			const result = await tool.execute("call-5", { query: "rust async" });
			const text = getText(result);
			expect(text).toContain("Smol runtime [1]");
			expect(text).not.toContain("blocked.example");
			expect(text).toContain("1 result hidden");
			expect((result.details as { hiddenCount?: number }).hiddenCount).toBe(1);
		});
	});

	describe("TLS flag forwarding", () => {
		let argvLog: string;
		const savedEnv = {
			HOOCODE_WEBTOOLS_CA_CERT: process.env.HOOCODE_WEBTOOLS_CA_CERT,
			HOOCODE_WEBTOOLS_INSECURE: process.env.HOOCODE_WEBTOOLS_INSECURE,
			WEBTOOLS_ARGV_LOG: process.env.WEBTOOLS_ARGV_LOG,
		};

		beforeEach(() => {
			argvLog = join(cwd, "argv.log");
			process.env.WEBTOOLS_ARGV_LOG = argvLog;
			delete process.env.HOOCODE_WEBTOOLS_CA_CERT;
			delete process.env.HOOCODE_WEBTOOLS_INSECURE;
		});

		afterEach(() => {
			for (const [key, value] of Object.entries(savedEnv)) {
				if (value === undefined) delete process.env[key];
				else process.env[key] = value;
			}
		});

		function loggedArgs(): string {
			return existsSync(argvLog) ? readFileSync(argvLog, "utf8") : "";
		}

		it("does not forward --ca-cert or --insecure when nothing is configured", async () => {
			const tool = createWebFetchTool(cwd);
			await tool.execute("tls-1", { url: "https://example.com" });
			const args = loggedArgs();
			expect(args).toContain("fetch");
			expect(args).not.toContain("--ca-cert");
			expect(args).not.toContain("--insecure");
		});

		it("forwards --ca-cert when HOOCODE_WEBTOOLS_CA_CERT points to a readable file", async () => {
			const caPath = join(cwd, "corporate-ca.pem");
			writeFileSync(caPath, "-----BEGIN CERTIFICATE-----\nFAKE\n-----END CERTIFICATE-----\n");
			process.env.HOOCODE_WEBTOOLS_CA_CERT = caPath;

			const tool = createWebFetchTool(cwd);
			await tool.execute("tls-2", { url: "https://example.com" });
			expect(loggedArgs()).toContain(`--ca-cert ${caPath}`);
		});

		it("does not forward --ca-cert when the configured file is missing/unreadable", async () => {
			process.env.HOOCODE_WEBTOOLS_CA_CERT = join(cwd, "nope-does-not-exist.pem");
			const tool = createWebFetchTool(cwd);
			await tool.execute("tls-3", { url: "https://example.com" });
			expect(loggedArgs()).not.toContain("--ca-cert");
		});

		it("forwards --insecure only when HOOCODE_WEBTOOLS_INSECURE is truthy (websearch)", async () => {
			process.env.HOOCODE_WEBTOOLS_INSECURE = "1";
			const tool = createWebSearchTool(cwd);
			await tool.execute("tls-4", { query: "rust async" });
			expect(loggedArgs()).toContain("--insecure");
		});

		it("prefers explicit options over env for CA path", async () => {
			const envCa = join(cwd, "env-ca.pem");
			const optCa = join(cwd, "opt-ca.pem");
			writeFileSync(envCa, "env\n");
			writeFileSync(optCa, "opt\n");
			process.env.HOOCODE_WEBTOOLS_CA_CERT = envCa;

			const tool = createWebFetchTool(cwd, { caCertPath: optCa });
			await tool.execute("tls-5", { url: "https://example.com" });
			const args = loggedArgs();
			expect(args).toContain(`--ca-cert ${optCa}`);
			expect(args).not.toContain(envCa);
		});

		it("resolveWebtoolsTLSConfig reads env and honors overrides", () => {
			process.env.HOOCODE_WEBTOOLS_CA_CERT = "/etc/ssl/proxy.pem";
			process.env.HOOCODE_WEBTOOLS_INSECURE = "yes";
			expect(resolveWebtoolsTLSConfig()).toEqual({ caCertPath: "/etc/ssl/proxy.pem", insecure: true });
			// Explicit overrides win over env.
			expect(resolveWebtoolsTLSConfig({ caCertPath: "/o.pem", insecure: false })).toEqual({
				caCertPath: "/o.pem",
				insecure: false,
			});
		});
	});
});

describe("resolveWebtoolsTimeoutSecs", () => {
	const saved = process.env.HOOCODE_WEBTOOLS_TIMEOUT;
	afterEach(() => {
		if (saved === undefined) delete process.env.HOOCODE_WEBTOOLS_TIMEOUT;
		else process.env.HOOCODE_WEBTOOLS_TIMEOUT = saved;
	});

	it("defaults to 15 when nothing is configured", () => {
		delete process.env.HOOCODE_WEBTOOLS_TIMEOUT;
		expect(resolveWebtoolsTimeoutSecs()).toBe(15);
	});

	it("reads the env override and clamps it to 1-120", () => {
		process.env.HOOCODE_WEBTOOLS_TIMEOUT = "30";
		expect(resolveWebtoolsTimeoutSecs()).toBe(30);
		process.env.HOOCODE_WEBTOOLS_TIMEOUT = "999";
		expect(resolveWebtoolsTimeoutSecs()).toBe(120);
		process.env.HOOCODE_WEBTOOLS_TIMEOUT = "0";
		expect(resolveWebtoolsTimeoutSecs()).toBe(15); // non-positive env falls back to default
	});

	it("prefers an explicit override over env, still clamped", () => {
		process.env.HOOCODE_WEBTOOLS_TIMEOUT = "30";
		expect(resolveWebtoolsTimeoutSecs(45)).toBe(45);
		expect(resolveWebtoolsTimeoutSecs(500)).toBe(120);
		expect(resolveWebtoolsTimeoutSecs(-1)).toBe(1);
	});

	it("falls back to default on a malformed env value", () => {
		process.env.HOOCODE_WEBTOOLS_TIMEOUT = "not-a-number";
		expect(resolveWebtoolsTimeoutSecs()).toBe(15);
	});
});

describe("clampMaxTokens", () => {
	it("defaults when unset or non-positive", () => {
		expect(clampMaxTokens(undefined)).toBe(4000);
		expect(clampMaxTokens(0)).toBe(4000);
		expect(clampMaxTokens(-5)).toBe(4000);
	});

	it("passes values through within range and caps above the ceiling", () => {
		expect(clampMaxTokens(1000)).toBe(1000);
		expect(clampMaxTokens(999999)).toBe(25000);
	});
});

describe("WebToolsCache.getOrCompute", () => {
	it("collapses concurrent identical calls onto one computation and then serves from cache", async () => {
		const cache = new WebToolsCache<number>();
		let calls = 0;
		const compute = () =>
			new Promise<number>((resolve) => {
				calls++;
				setTimeout(() => resolve(42), 20);
			});

		const [a, b] = await Promise.all([
			cache.getOrCompute("k", undefined, compute),
			cache.getOrCompute("k", undefined, compute),
		]);
		expect(a).toBe(42);
		expect(b).toBe(42);
		expect(calls).toBe(1);

		// A later call is served from the cache without recomputing.
		expect(await cache.getOrCompute("k", undefined, compute)).toBe(42);
		expect(calls).toBe(1);
	});

	it("does not cache failures", async () => {
		const cache = new WebToolsCache<number>();
		let calls = 0;
		await expect(
			cache.getOrCompute("k", undefined, () => {
				calls++;
				return Promise.reject(new Error("boom"));
			}),
		).rejects.toThrow("boom");

		const ok = await cache.getOrCompute("k", undefined, () => {
			calls++;
			return Promise.resolve(7);
		});
		expect(ok).toBe(7);
		expect(calls).toBe(2);
	});

	it("keeps shared work running for remaining callers when one aborts", async () => {
		const cache = new WebToolsCache<number>();
		let calls = 0;
		let computeAborted = false;
		const compute = (sig: AbortSignal) =>
			new Promise<number>((resolve, reject) => {
				calls++;
				sig.addEventListener("abort", () => {
					computeAborted = true;
					reject(new Error("Operation aborted"));
				});
				setTimeout(() => resolve(99), 30);
			});

		const ac = new AbortController();
		const aborter = cache.getOrCompute("k", ac.signal, compute);
		const stayer = cache.getOrCompute("k", undefined, compute);
		ac.abort();

		await expect(aborter).rejects.toThrow("Operation aborted");
		expect(await stayer).toBe(99);
		expect(calls).toBe(1);
		expect(computeAborted).toBe(false);
	});
});
