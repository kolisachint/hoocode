import { describe, expect, it } from "vitest";
import { deferMcpSchemas } from "../src/core/subagent-depth.js";
import {
	CATALOG_EAGER_LIMIT,
	type DeferredMcpToolEntry,
	formatDeferredCatalog,
	selectResolvable,
	toCapabilityDocs,
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

	it("summarizes instead of listing once the catalog outgrows the budget", () => {
		const big: DeferredMcpToolEntry[] = Array.from({ length: 40 }, (_, i) => ({
			toolName: `mcp_big_tool_${i}`,
			server: "big",
			description: `Tool number ${i}`,
		}));
		const text = formatDeferredCatalog(big);

		expect(text).toContain("40 tools across 1 server(s)");
		expect(text).toContain("big — 40 tool(s)");
		// The tail must not be silently dropped: the model has to know the rest
		// exists and how to reach it, or a summarized catalog is just a shorter lie.
		expect(text).toContain("Search for the rest by capability");
		expect(text).not.toContain("mcp_big_tool_39 — Tool number 39");
	});

	it("still lists in full at the threshold", () => {
		const exact: DeferredMcpToolEntry[] = Array.from({ length: CATALOG_EAGER_LIMIT }, (_, i) => ({
			toolName: `mcp_s_t${i}`,
			server: "s",
			description: `d${i}`,
		}));
		const text = formatDeferredCatalog(exact);
		expect(text).toContain(`mcp_s_t${CATALOG_EAGER_LIMIT - 1} — d${CATALOG_EAGER_LIMIT - 1}`);
		expect(text).not.toContain("Search for the rest");
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

	it("stays exact — a near miss resolves nothing", () => {
		// Fuzziness belongs to the `query` parameter. A caller naming a tool it
		// already knows must not get a different one that happened to rank well.
		expect(selectResolvable(catalog, ["create_pull_request"])).toEqual([]);
	});
});

describe("toCapabilityDocs", () => {
	it("carries the server and the deferred flag into the index", () => {
		const docs = toCapabilityDocs(catalog, true);
		expect(docs).toHaveLength(3);
		expect(docs[0]).toMatchObject({
			id: "mcp_github_create_pr",
			kind: "mcp-tool",
			source: "github",
			deferred: true,
		});
		expect(toCapabilityDocs(catalog, false)[0].deferred).toBe(false);
	});
});
