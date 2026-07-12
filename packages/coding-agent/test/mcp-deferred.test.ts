import { describe, expect, it } from "vitest";
import { deferMcpSchemas } from "../src/core/subagent-depth.js";
import {
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
