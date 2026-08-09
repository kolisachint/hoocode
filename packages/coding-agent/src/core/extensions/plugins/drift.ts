/**
 * Tier 2 — offline drift check (§2.2).
 *
 * The adapters had drifted in fifteen places across two vendors and nothing
 * noticed. The defect was never a missing doc-fetcher; it was a missing
 * *signal*. This module is that signal: diff what the vendors document against
 * what {@link declaredVocabulary} says we understand, and report the gap.
 *
 * Two properties are non-negotiable, both inherited from Tier 0:
 *
 *  - **Nothing here runs at runtime.** The fetch happens in a CI job (see
 *    `scripts/plugin-drift-check.ts`). The network informs a human's decision to
 *    edit an adapter; it never informs a parse. That is the same rule
 *    `AGENTS.md` sets for `models.generated.ts`.
 *  - **A failed fetch is not a clean report.** Offline must be distinguishable
 *    from "no drift", or the check quietly stops checking the day the docs move
 *    behind a redirect.
 *
 * The extraction is deliberately conservative. Manifest keys come from JSON code
 * fences — actual manifest examples, not prose — because a regex over English
 * finds every word that happens to be backticked. Paths come from backticked
 * path-shaped tokens, which is noisier, so path findings are leads for a human
 * to confirm rather than assertions. A drift report nobody trusts gets muted,
 * and a muted report is worse than none.
 */

import { declaredVocabulary } from "./formats/index.js";

export interface DriftSource {
	label: string;
	url: string;
	/** Page text, or undefined when it could not be fetched. */
	text?: string;
	error?: string;
}

export interface DriftFinding {
	kind: "manifest-key" | "path";
	value: string;
	/** Which reference page it was seen in. */
	source: string;
}

export interface DriftReport {
	/** True only when every source was fetched *and* nothing new was found. */
	clean: boolean;
	/** Sources that could not be fetched. Non-empty means the report is incomplete. */
	unreachable: Array<{ label: string; error: string }>;
	findings: DriftFinding[];
	checkedSources: string[];
}

/** Top-level keys of every JSON object in a fenced ```json block. */
export function manifestKeysIn(markdown: string): Set<string> {
	const keys = new Set<string>();
	for (const match of markdown.matchAll(/```(?:jsonc?|json5)\n([\s\S]*?)```/g)) {
		const body = match[1];
		// Strip comments and trailing commas so documentation JSON (which is often
		// neither) still parses.
		const cleaned = body
			.replace(/^\s*\/\/.*$/gm, "")
			.replace(/\/\*[\s\S]*?\*\//g, "")
			.replace(/,(\s*[}\]])/g, "$1");
		try {
			const parsed = JSON.parse(cleaned) as unknown;
			if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
				for (const key of Object.keys(parsed)) keys.add(key);
			}
		} catch {
			// A fence that does not parse tells us nothing; guessing at its keys with
			// a regex is how a drift report fills up with noise.
		}
	}
	return keys;
}

/**
 * Plugin component directories worth noticing on their own, without a file
 * extension to identify them.
 */
/**
 * Files every repository has. They appear in vendor references as scaffolding
 * examples, never as a surface an adapter should parse.
 */
const GENERIC_FILES = new Set(["README.md", "CHANGELOG.md", "LICENSE.md", "CLAUDE.md", "AGENTS.md", "package.json"]);

const COMPONENT_DIR_NAMES = new Set([
	"skills",
	"commands",
	"agents",
	"hooks",
	"themes",
	"workflows",
	"output-styles",
	"monitors",
	"bin",
	"prompts",
	"chatmodes",
]);

/**
 * Backticked tokens that look like a **plugin-relative** file or directory path.
 *
 * Tight on purpose. The first cut of this accepted anything path-shaped and
 * produced twenty findings against the live Claude reference, of which none were
 * real drift: MCP method names (`roots/list`), repo slugs
 * (`anthropics/claude-plugins-community`), workspace paths (`.claude/settings.json`),
 * bare extensions (`.zip`), and example scripts. A drift report with that
 * signal-to-noise ratio gets muted, and a muted report is worse than none — so
 * the filter errs toward missing a real surface rather than crying wolf, and the
 * manifest-key half (which reads parsed JSON, not prose) carries the precision.
 */
export function pathsIn(markdown: string): Set<string> {
	const paths = new Set<string>();
	for (const match of markdown.matchAll(/`([^`\s]+)`/g)) {
		const token = match[1].replace(/^\.\//, "").replace(/\/$/, "");
		if (!/^[\w.@-]+(?:\/[\w.@-]+)*$/.test(token)) continue;
		// Workspace surfaces, not plugin surfaces: `.claude/settings.json` and
		// friends are §1.3's territory and are not what an adapter parses.
		if (token.startsWith(".claude/") || token.startsWith(".github/workflows")) continue;
		if (token.startsWith("..")) continue;

		const tail = token.slice(token.lastIndexOf("/") + 1);
		if (GENERIC_FILES.has(tail)) continue;
		// A real surface is a config file or a component directory. Anything else
		// path-shaped in prose is an example, a slug, or a protocol method.
		const isConfigFile = /\.(json|jsonc|md|toml|ya?ml)$/.test(tail);
		const isComponentDir = COMPONENT_DIR_NAMES.has(tail) || COMPONENT_DIR_NAMES.has(token);
		if (!isConfigFile && !isComponentDir) continue;
		// A single token with no separator has to stand on its own: a named config
		// file (`.mcp.json`, `plugin.json`) or a component directory. A bare
		// extension like `.zip` is neither.
		if (!token.includes("/") && !isComponentDir && !/^\.?[\w-]+\.(json|jsonc|md|toml|ya?ml)$/.test(token)) continue;
		paths.add(token);
	}
	return paths;
}

/**
 * Whether a documented path is one we already declare.
 *
 * Suffix match, because references write paths from an example plugin root
 * (`my-plugin/hooks/hooks.json`) while adapters declare them plugin-relative
 * (`hooks/hooks.json`). Comparing the two literally reports every documented
 * example as new.
 */
function isDeclaredPath(documented: string, declared: ReadonlySet<string>): boolean {
	if (declared.has(documented)) return true;
	for (const known of declared) {
		if (documented === known || documented.endsWith(`/${known}`) || known.endsWith(`/${documented}`)) return true;
		// A documented directory whose contents we already declare: the reference
		// writes `monitors`, the adapter declares `monitors/monitors.json`.
		if (known.startsWith(`${documented}/`) || documented.startsWith(`${known}/`)) return true;
	}
	return false;
}

/**
 * Diff the fetched references against the declared vocabulary.
 *
 * Everything already declared — modelled, read, or knowingly unsupported — is
 * filtered out, so what remains answers one question: *is the vendor documenting
 * something we have never heard of?*
 */
export function analyzeDrift(sources: readonly DriftSource[]): DriftReport {
	const declared = declaredVocabulary();
	const knownKeys = new Set([...declared.manifestKeys, ...declared.marketplaceKeys]);
	const knownPaths = new Set(
		[...declared.readPaths, ...declared.marketplaceFiles, ...declared.unsupportedSurfaces].map((p) =>
			p.replace(/\\/g, "/").replace(/\/$/, ""),
		),
	);

	const unreachable: DriftReport["unreachable"] = [];
	const findings: DriftFinding[] = [];
	const checkedSources: string[] = [];
	const seen = new Set<string>();

	for (const source of sources) {
		if (!source.text) {
			unreachable.push({ label: source.label, error: source.error ?? "not fetched" });
			continue;
		}
		checkedSources.push(source.label);
		for (const key of manifestKeysIn(source.text)) {
			if (knownKeys.has(key) || seen.has(`k:${key}`)) continue;
			seen.add(`k:${key}`);
			findings.push({ kind: "manifest-key", value: key, source: source.label });
		}
		for (const p of pathsIn(source.text)) {
			if (isDeclaredPath(p, knownPaths) || seen.has(`p:${p}`)) continue;
			seen.add(`p:${p}`);
			findings.push({ kind: "path", value: p, source: source.label });
		}
	}

	// Incomplete is never clean. A partial sweep that reports "no drift" is the
	// failure this tier exists to prevent, one level up.
	return { clean: unreachable.length === 0 && findings.length === 0, unreachable, findings, checkedSources };
}

/** Render a report for a CI log or a human-opened issue. */
export function formatDriftReport(report: DriftReport): string {
	const lines: string[] = [];
	if (report.checkedSources.length > 0) lines.push(`Checked: ${report.checkedSources.join(", ")}`);
	for (const u of report.unreachable) {
		lines.push(`UNREACHABLE ${u.label}: ${u.error} — this report is incomplete.`);
	}
	if (report.findings.length === 0) {
		lines.push(report.unreachable.length > 0 ? "No drift in what could be read." : "No drift.");
		return lines.join("\n");
	}
	lines.push("", `${report.findings.length} candidate(s) documented upstream but not declared here:`);
	for (const f of report.findings) {
		lines.push(`  [${f.kind}] ${f.value}  (${f.source})`);
	}
	lines.push(
		"",
		"These are leads, not verdicts — path extraction reads prose and will surface",
		"examples alongside conventions. Confirm against the reference, then either",
		"model the surface in the adapter or add it to the knowingly-unsupported list.",
	);
	return lines.join("\n");
}
