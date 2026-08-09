#!/usr/bin/env bun
/**
 * Tier 2 drift check (§2.2) — fetch the vendor plugin references and report
 * anything they document that our adapters have never heard of.
 *
 * Run in CI or by hand; never at runtime. The whole point of the tier is that
 * the network informs a *human's* decision to edit an adapter, exactly as
 * `AGENTS.md` requires for `models.generated.ts`.
 *
 *   bun scripts/plugin-drift-check.ts
 *
 * Exit codes are three-way on purpose. A network failure must not be reported as
 * a clean sweep, or the check silently stops checking the day a docs URL moves:
 *
 *   0  every source read, nothing new
 *   1  drift found — a human should look
 *   2  a source could not be read; the sweep is incomplete
 *
 * Copilot's reference is read from the `github/docs` source repo rather than
 * `docs.github.com`, because the rendered site is unreachable from some
 * environments (including the one this was written in) while the raw markdown
 * is the same content, closer to the source, and cacheable.
 */

import { execFileSync } from "node:child_process";

import { analyzeDrift, type DriftSource, formatDriftReport } from "../src/core/extensions/plugins/drift.js";

const SOURCES: ReadonlyArray<{ label: string; url: string }> = [
	{ label: "claude/plugins", url: "https://code.claude.com/docs/en/plugins.md" },
	{ label: "claude/plugins-reference", url: "https://code.claude.com/docs/en/plugins-reference.md" },
	{
		label: "copilot/cli-plugin-reference",
		url: "https://raw.githubusercontent.com/github/docs/main/content/copilot/reference/copilot-cli-reference/cli-plugin-reference.md",
	},
	{
		label: "copilot/plugins-creating",
		url: "https://raw.githubusercontent.com/github/docs/main/content/copilot/how-tos/copilot-cli/customize-copilot/plugins-creating.md",
	},
];

const FETCH_TIMEOUT_MS = 20_000;

/**
 * Two transports, because one is not reliably enough.
 *
 * The runtime's `fetch` is tried first. It fails in environments whose egress
 * goes through a CA-terminating proxy that the runtime's own HTTP client does
 * not honour — which is not exotic, and a drift check that reports UNREACHABLE
 * forever is a check that has been switched off without anyone deciding to. So
 * `curl` gets a turn, since it reads the standard proxy and CA variables.
 */
async function fetchSource(label: string, url: string): Promise<DriftSource> {
	let firstError: string;
	try {
		const response = await fetch(url, { signal: AbortSignal.timeout(FETCH_TIMEOUT_MS) });
		if (response.ok) return { label, url, text: await response.text() };
		firstError = `HTTP ${response.status}`;
	} catch (error) {
		firstError = (error as Error).message;
	}

	try {
		const text = execFileSync(
			"curl",
			["-sSL", "--max-time", String(Math.round(FETCH_TIMEOUT_MS / 1000)), "--fail", url],
			{ encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
		);
		if (text.trim().length > 0) return { label, url, text };
	} catch {
		// curl absent; fall through to reporting the original failure.
	}
	return { label, url, error: firstError };
}

const sources = await Promise.all(SOURCES.map((s) => fetchSource(s.label, s.url)));
const report = analyzeDrift(sources);
console.log(formatDriftReport(report));

if (report.unreachable.length > 0) process.exit(2);
process.exit(report.findings.length > 0 ? 1 : 0);
