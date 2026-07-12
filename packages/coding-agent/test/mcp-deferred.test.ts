import { describe, expect, it } from "vitest";
import { deferMcpSchemas } from "../src/core/subagent-depth.js";
import {
	connectAllInOrder,
	type DeferredMcpToolEntry,
	formatDeferredCatalog,
	selectResolvable,
} from "../src/extensions/core/mcp-deferred.js";

const catalog: DeferredMcpToolEntry[] = [
	{ toolName: "mcp_github_create_pr", server: "github", description: "Create a pull request" },
	{ toolName: "mcp_github_list_issues", server: "github", description: "List issues\n(second line ignored)" },
	{ toolName: "mcp_fs_read_file", server: "fs", description: "Read a file" },
];

describe("deferMcpSchemas env predicate", () => {
	it("is true only when the env flag is exactly '1'", () => {
		expect(deferMcpSchemas({ HOOCODE_DEFER_MCP_SCHEMAS: "1" })).toBe(true);
		expect(deferMcpSchemas({ HOOCODE_DEFER_MCP_SCHEMAS: "0" })).toBe(false);
		expect(deferMcpSchemas({})).toBe(false);
	});
});

describe("formatDeferredCatalog", () => {
	it("groups by server and shows names + first-line descriptions", () => {
		const text = formatDeferredCatalog(catalog);
		expect(text).toContain("github:");
		expect(text).toContain("  mcp_github_create_pr — Create a pull request");
		expect(text).toContain("fs:");
		expect(text).toContain("  mcp_fs_read_file — Read a file");
		// Only the first line of a multi-line description is used.
		expect(text).not.toContain("second line ignored");
	});

	it("handles an empty catalog", () => {
		expect(formatDeferredCatalog([])).toBe("(no MCP tools available)");
	});

	it("annotates a server's header line with its configured snippet", () => {
		const text = formatDeferredCatalog(
			catalog,
			new Map([["github", "Use these for GitHub operations instead of bash git"]]),
		);
		expect(text).toContain("github: Use these for GitHub operations instead of bash git");
		// Servers without a snippet keep the bare header.
		expect(text).toContain("fs:\n");
	});
});

describe("selectResolvable", () => {
	it("matches by full registered name", () => {
		expect(selectResolvable(catalog, ["mcp_github_create_pr"]).map((e) => e.toolName)).toEqual([
			"mcp_github_create_pr",
		]);
	});

	it("matches by bare tool name", () => {
		expect(selectResolvable(catalog, ["read_file"]).map((e) => e.toolName)).toEqual(["mcp_fs_read_file"]);
	});

	it("returns nothing for unknown names or an empty request", () => {
		expect(selectResolvable(catalog, ["nope"])).toEqual([]);
		expect(selectResolvable(catalog, [])).toEqual([]);
	});

	it("dedupes repeated requests", () => {
		expect(selectResolvable(catalog, ["mcp_github_create_pr", "create_pr"]).map((e) => e.toolName)).toEqual([
			"mcp_github_create_pr",
		]);
	});
});

describe("connectAllInOrder", () => {
	it("starts every connect before any completes (concurrent, not sequential)", async () => {
		const started: string[] = [];
		const resolvers = new Map<string, () => void>();
		const promise = connectAllInOrder(["a", "b", "c"], (name) => {
			started.push(name);
			return new Promise<string>((resolve) => {
				resolvers.set(name, () => resolve(`conn-${name}`));
			});
		});
		// All connects were kicked off synchronously, before any resolved.
		expect(started).toEqual(["a", "b", "c"]);
		for (const resolve of resolvers.values()) resolve();
		await promise;
	});

	it("returns outcomes in config order even when completion order differs", async () => {
		const resolvers = new Map<string, (v: string) => void>();
		const promise = connectAllInOrder(
			["slow", "fast"],
			(name) => new Promise<string>((resolve) => resolvers.set(name, resolve)),
		);
		resolvers.get("fast")?.("conn-fast");
		resolvers.get("slow")?.("conn-slow");
		const outcomes = await promise;
		expect(outcomes.map((o) => o.config)).toEqual(["slow", "fast"]);
		expect(outcomes.map((o) => (o.result.status === "fulfilled" ? o.result.value : "?"))).toEqual([
			"conn-slow",
			"conn-fast",
		]);
	});

	it("isolates failures: one rejected connect does not affect the others", async () => {
		const outcomes = await connectAllInOrder(["ok", "dead", "ok2"], (name) =>
			name === "dead" ? Promise.reject(new Error("handshake timeout")) : Promise.resolve(`conn-${name}`),
		);
		expect(outcomes.map((o) => o.result.status)).toEqual(["fulfilled", "rejected", "fulfilled"]);
		const dead = outcomes[1]?.result;
		expect(dead?.status === "rejected" && String(dead.reason)).toContain("handshake timeout");
	});

	it("handles an empty config list", async () => {
		expect(await connectAllInOrder([], () => Promise.resolve(1))).toEqual([]);
	});
});
