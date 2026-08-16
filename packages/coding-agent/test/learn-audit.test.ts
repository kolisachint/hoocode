import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { auditContextFiles, classifyReferent, resolutionRoots, staleTokens } from "../src/core/learn/audit.js";
import { renderAuditReport } from "../src/core/learn/digest.js";

let tempDir: string;

afterEach(() => {
	if (tempDir) {
		rmSync(tempDir, { recursive: true, force: true });
		tempDir = "";
	}
});

/** A repo with a root manifest, one package, and an AGENTS.md holding `content`. */
function repo(content: string, options?: { files?: string[]; scripts?: string[] }): { cwd: string; agents: string } {
	tempDir = mkdtempSync(join(tmpdir(), "learn-audit-"));
	const scripts: Record<string, string> = {};
	for (const name of options?.scripts ?? []) scripts[name] = "true";
	writeFileSync(join(tempDir, "package.json"), JSON.stringify({ name: "root", scripts }));

	for (const file of options?.files ?? []) {
		const full = join(tempDir, file);
		mkdirSync(join(full, ".."), { recursive: true });
		writeFileSync(full, "");
	}

	const agents = join(tempDir, "AGENTS.md");
	writeFileSync(agents, content);
	return { cwd: tempDir, agents };
}

function audit(content: string, options?: { files?: string[]; scripts?: string[] }) {
	const { cwd, agents } = repo(content, options);
	return auditContextFiles({ cwd, files: [{ path: agents }] });
}

describe("classifyReferent", () => {
	it("checks a path that names a location", () => {
		expect(classifyReferent("docs/providers.md").kind).toBe("path");
		expect(classifyReferent("src/core/learn/").kind).toBe("path");
	});

	it("checks a run script", () => {
		expect(classifyReferent("bun run check")).toEqual({ kind: "script", script: "check" });
		expect(classifyReferent("npm run release:patch")).toEqual({ kind: "script", script: "release:patch" });
	});

	it("skips a bare filename, which names no particular location", () => {
		// A monorepo has four of these; "not at the repo root" is not staleness.
		expect(classifyReferent("stream.test.ts")).toEqual({ kind: "skip", reason: "ambiguous" });
	});

	it("skips placeholders and globs rather than reporting them missing", () => {
		expect(classifyReferent("packages/*/CHANGELOG.md")).toEqual({ kind: "skip", reason: "placeholder" });
		expect(classifyReferent("dist/providers/<provider>.js")).toEqual({ kind: "skip", reason: "placeholder" });
		expect(classifyReferent("src/core/tools/{browser,doc}/")).toEqual({ kind: "skip", reason: "placeholder" });
	});

	it("skips runtime locations, which are not repo artifacts", () => {
		expect(classifyReferent("~/.agents/AGENTS.md")).toEqual({ kind: "skip", reason: "runtime" });
		expect(classifyReferent("/etc/hosts")).toEqual({ kind: "skip", reason: "runtime" });
	});

	it("skips urls and git refs", () => {
		expect(classifyReferent("https://example.com/x.md")).toEqual({ kind: "skip", reason: "external" });
		expect(classifyReferent("origin/main")).toEqual({ kind: "skip", reason: "external" });
	});

	it("skips a command, which is prose with a slash in it", () => {
		expect(classifyReferent("git add -A")).toEqual({ kind: "skip", reason: "ambiguous" });
	});
});

describe("resolutionRoots", () => {
	it("treats every package directory as a root, so a monorepo path resolves", () => {
		tempDir = mkdtempSync(join(tmpdir(), "learn-audit-"));
		writeFileSync(join(tempDir, "package.json"), "{}");
		mkdirSync(join(tempDir, "packages", "ai"), { recursive: true });
		writeFileSync(join(tempDir, "packages", "ai", "package.json"), "{}");
		mkdirSync(join(tempDir, "node_modules", "dep"), { recursive: true });
		writeFileSync(join(tempDir, "node_modules", "dep", "package.json"), "{}");

		const roots = resolutionRoots(tempDir);
		expect(roots).toContain(join(tempDir, "packages", "ai"));
		expect(roots.some((r) => r.includes("node_modules"))).toBe(false);
	});
});

describe("auditContextFiles", () => {
	it("reports a path that no longer exists, with the line to fix", () => {
		const report = audit("- Defined in `.agents/commands/push.md`.\n");
		expect(report.stale).toHaveLength(1);
		expect(report.stale[0]?.referent).toBe(".agents/commands/push.md");
		expect(report.stale[0]?.line).toBe(1);
		expect(report.stale[0]?.kind).toBe("path");
	});

	it("says nothing about a path that resolves", () => {
		const report = audit("- See `docs/providers.md` for setup.\n", { files: ["docs/providers.md"] });
		expect(report.stale).toHaveLength(0);
		expect(report.checked).toBe(1);
	});

	it("resolves a path relative to a package root, not just the repo root", () => {
		const report = audit("- Add the id to `src/core/model-resolver.ts`.\n", {
			files: ["packages/ai/package.json", "packages/ai/src/core/model-resolver.ts"],
		});
		expect(report.stale).toHaveLength(0);
	});

	it("reports a run script that no package declares", () => {
		const report = audit("- After changes run `bun run check`.\n", { scripts: ["build"] });
		expect(report.stale).toHaveLength(1);
		expect(report.stale[0]?.kind).toBe("script");
	});

	it("accepts a run script that any package declares", () => {
		const report = audit("- After changes run `bun run check`.\n", { scripts: ["check"] });
		expect(report.stale).toHaveLength(0);
	});

	it("ignores a line that asserts the referent is gone", () => {
		// The line is correct precisely because the file is missing.
		const report = audit("- The `src/core/tools/browser/index.ts` entry was removed.\n");
		expect(report.stale).toHaveLength(0);
		expect(report.skipped.assertsAbsence).toBe(1);
	});

	it("ignores an example, which names a shape rather than a file", () => {
		const report = audit("- For non-standard auth, create a utility (e.g. `src/utils/bedrock-utils.ts`).\n");
		expect(report.stale).toHaveLength(0);
	});

	it("ignores fenced code, where a plausible path is illustration", () => {
		const report = audit("```bash\ngit add `docs/gone.md`\n```\n\n- Real rule with no referent.\n");
		expect(report.checked).toBe(0);
		expect(report.stale).toHaveLength(0);
	});

	it("audits the repo's context file when run from a package subdirectory", () => {
		// The normal way to work in a monorepo, and the case that anchoring on cwd
		// got wrong: AGENTS.md sits above the package, so the audit skipped the only
		// file with any claims in it and reported a clean run.
		const { cwd, agents } = repo("- Defined in `.agents/commands/push.md`.\n", {
			files: ["packages/thing/package.json"],
		});
		mkdirSync(join(cwd, ".git"), { recursive: true });

		const report = auditContextFiles({ cwd: join(cwd, "packages", "thing"), files: [{ path: agents }] });
		expect(report.files).toHaveLength(1);
		expect(report.stale).toHaveLength(1);
	});

	it("resolves a repo-relative path from a subdirectory too", () => {
		const { cwd, agents } = repo("- See `docs/providers.md`.\n", {
			files: ["docs/providers.md", "packages/thing/package.json"],
		});
		mkdirSync(join(cwd, ".git"), { recursive: true });

		const report = auditContextFiles({ cwd: join(cwd, "packages", "thing"), files: [{ path: agents }] });
		expect(report.stale).toHaveLength(0);
		expect(report.checked).toBe(1);
	});

	it("does not audit a context file outside the project", () => {
		const { cwd } = repo("- nothing here\n");
		const outside = mkdtempSync(join(tmpdir(), "learn-audit-user-"));
		try {
			const user = join(outside, "AGENTS.md");
			writeFileSync(user, "- See `src/index.ts`.\n");
			const report = auditContextFiles({ cwd, files: [{ path: user }] });
			expect(report.stale).toHaveLength(0);
			expect(report.skippedFiles).toEqual([user]);
		} finally {
			rmSync(outside, { recursive: true, force: true });
		}
	});

	it("prices the lines it flags, since deleting them is the payoff", () => {
		const report = audit("- Auto-closed by `.github/workflows/issue-gate.yml` on every new issue.\n");
		expect(report.stale).toHaveLength(1);
		expect(staleTokens(report)).toBeGreaterThan(0);
	});

	it("counts one line once, however many referents on it are stale", () => {
		const report = audit("- Handled by `.github/workflows/a.yml` and `.github/workflows/b.yml`.\n");
		expect(report.stale).toHaveLength(2);
		expect(staleTokens(report)).toBe(report.stale[0]?.tokens);
	});
});

describe("renderAuditReport", () => {
	it("presents findings as candidates to verify, not as a licence to delete", () => {
		const report = audit("- Defined in `.agents/commands/push.md`.\n");
		const text = renderAuditReport(report);
		expect(text).toContain(".agents/commands/push.md");
		expect(text).toContain("candidate, not a verdict");
		// The subtractive half must not turn into another proposal engine.
		expect(text).toContain("Do not add anything");
	});
});
