import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { loadProjectContextFiles } from "../src/core/context-files.js";

let tempDir: string;

afterEach(() => {
	if (tempDir) {
		rmSync(tempDir, { recursive: true, force: true });
		tempDir = "";
	}
});

interface Fixture {
	cwd: string;
	agentDir: string;
	userAgentsDir: string;
}

function createFixture(): Fixture {
	tempDir = mkdtempSync(join(tmpdir(), "hoo-context-files-"));
	const fixture = {
		cwd: join(tempDir, "repo"),
		agentDir: join(tempDir, ".hoocode"),
		userAgentsDir: join(tempDir, ".agents"),
	};
	for (const dir of Object.values(fixture)) mkdirSync(dir, { recursive: true });
	return fixture;
}

function load(fixture: Fixture) {
	return loadProjectContextFiles(fixture);
}

describe("user-scope context files", () => {
	it("reads ~/.agents/AGENTS.md as instructions", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.userAgentsDir, "AGENTS.md"), "prefer functional typescript");

		const { agentsFiles } = load(fixture);
		expect(agentsFiles.map((f) => f.path)).toEqual([join(fixture.userAgentsDir, "AGENTS.md")]);
		expect(agentsFiles[0]!.content).toBe("prefer functional typescript");
	});

	it("loads both user scopes additively, cross-vendor first", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.userAgentsDir, "AGENTS.md"), "cross-vendor rule");
		writeFileSync(join(fixture.agentDir, "AGENTS.md"), "native rule");

		const { agentsFiles } = load(fixture);
		expect(agentsFiles.map((f) => f.content)).toEqual(["cross-vendor rule", "native rule"]);
	});

	it("orders user scopes before the repo, least specific first", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.userAgentsDir, "AGENTS.md"), "cross-vendor rule");
		writeFileSync(join(fixture.agentDir, "AGENTS.md"), "native rule");
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "repo rule");

		const { agentsFiles } = load(fixture);
		expect(agentsFiles.map((f) => f.content)).toEqual(["cross-vendor rule", "native rule", "repo rule"]);
	});

	it("does not double-count when both user scopes point at one directory", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.agentDir, "AGENTS.md"), "only once");

		const { agentsFiles } = load({ ...fixture, userAgentsDir: fixture.agentDir });
		expect(agentsFiles).toHaveLength(1);
	});

	it("reads nothing when the cross-vendor scope is absent", () => {
		const fixture = createFixture();
		rmSync(fixture.userAgentsDir, { recursive: true, force: true });
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "repo rule");

		const { agentsFiles } = load(fixture);
		expect(agentsFiles.map((f) => f.content)).toEqual(["repo rule"]);
	});
});

describe("aggregate context-file budget", () => {
	it("stays silent under the soft cap", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "x".repeat(4 * 1024));

		const { agentsFiles, warnings } = load(fixture);
		expect(warnings).toEqual([]);
		expect(agentsFiles[0]!.size).toBeUndefined();
	});

	it("warns without trimming between the soft and hard caps", () => {
		const fixture = createFixture();
		// Two files, each under the 40KB per-file cap, together over the 24KB total.
		writeFileSync(join(fixture.userAgentsDir, "AGENTS.md"), "u".repeat(16 * 1024));
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "r".repeat(16 * 1024));

		const { agentsFiles, warnings } = load(fixture);
		expect(warnings.some((w) => w.includes("re-sent on every request"))).toBe(true);
		expect(agentsFiles.every((f) => f.size !== "truncated")).toBe(true);
		expect(agentsFiles.map((f) => f.content.length)).toEqual([16 * 1024, 16 * 1024]);
	});

	it("trims the least specific scope first past the hard cap", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.userAgentsDir, "AGENTS.md"), "u".repeat(39 * 1024));
		writeFileSync(join(fixture.agentDir, "AGENTS.md"), "n".repeat(39 * 1024));
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "r".repeat(39 * 1024));

		const { agentsFiles, warnings } = load(fixture);
		const [crossVendor, native, repo] = agentsFiles;

		// The file nearest the work keeps its budget; the outer scopes pay. It is
		// still flagged `large` by the per-file rule, which is a separate concern.
		expect(repo!.size).toBe("large");
		expect(repo!.content).toBe("r".repeat(39 * 1024));
		expect(native!.size).toBe("truncated");
		expect(crossVendor!.size).toBe("truncated");
		expect(warnings.some((w) => w.includes("total budget"))).toBe(true);
	});

	it("keeps a trimmed file visible instead of dropping it", () => {
		const fixture = createFixture();
		writeFileSync(join(fixture.userAgentsDir, "AGENTS.md"), "u".repeat(39 * 1024));
		writeFileSync(join(fixture.agentDir, "AGENTS.md"), "n".repeat(39 * 1024));
		writeFileSync(join(fixture.cwd, "AGENTS.md"), "r".repeat(39 * 1024));

		const { agentsFiles } = load(fixture);
		expect(agentsFiles).toHaveLength(3);
		for (const file of agentsFiles.slice(0, 2)) {
			expect(file.content).toContain("[trimmed:");
			expect(file.tokens).toBe(Math.round(Buffer.byteLength(file.content, "utf-8") / 4));
		}
	});
});
