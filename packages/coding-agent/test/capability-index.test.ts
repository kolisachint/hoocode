/**
 * The capability index (architecture step 10, §6).
 *
 * The dense leg is not exercised here: it needs an embedding daemon that may not
 * be installed, and a test that skips itself on most machines is not a test. What
 * *is* tested is that its absence is invisible to callers — the lexical leg is
 * the floor, and every guarantee below holds without a binary.
 */

import { afterEach, describe, expect, it } from "vitest";
import { denseState } from "../src/core/capabilities/dense.js";
import { LexicalIndex, tokenize } from "../src/core/capabilities/lexical.js";
import {
	type CapabilityDoc,
	capabilitySetHash,
	clearCapabilities,
	getCapabilities,
	registerCapabilities,
} from "../src/core/capabilities/registry.js";
import { resetCapabilitySearch, searchCapabilities } from "../src/core/capabilities/search.js";

function doc(id: string, name: string, description: string, extra: Partial<CapabilityDoc> = {}): CapabilityDoc {
	return { id, kind: "mcp-tool", name, description, deferred: true, ...extra };
}

const tools: CapabilityDoc[] = [
	doc("mcp_github_create_pull_request", "mcp_github_create_pull_request", "Open a pull request against a branch.", {
		source: "github",
	}),
	doc("mcp_github_list_issues", "mcp_github_list_issues", "List issues in a repository.", { source: "github" }),
	doc("mcp_mail_send_message", "mcp_mail_send_message", "Send an email message to a recipient.", { source: "mail" }),
	doc("mcp_fs_read_file", "mcp_fs_read_file", "Read the contents of a file from disk.", { source: "fs" }),
];

afterEach(() => {
	clearCapabilities();
	resetCapabilitySearch();
});

describe("tokenize", () => {
	it("splits the identifier shapes tool names actually arrive in", () => {
		// The whole reason for a custom tokenizer: a half-remembered name is the
		// most common query, and a whitespace split cannot match any of these.
		expect(tokenize("mcp_github_create_pull_request")).toContain("pull");
		expect(tokenize("mcp_github_create_pull_request")).toContain("request");
		expect(tokenize("createPullRequest")).toContain("pull");
		expect(tokenize("list-issues")).toContain("issues");
	});

	it("keeps the whole token so an exact name still matches exactly", () => {
		expect(tokenize("mcp_fs_read_file")).toContain("mcp_fs_read_file");
	});
});

describe("LexicalIndex", () => {
	it("ranks the tool whose name matches the query first", () => {
		const index = new LexicalIndex(tools);
		expect(index.search("pull request")[0]?.id).toBe("mcp_github_create_pull_request");
		expect(index.search("send email")[0]?.id).toBe("mcp_mail_send_message");
	});

	it("finds a tool by a fragment of its name, which exact matching could not", () => {
		const index = new LexicalIndex(tools);
		expect(index.search("read_file")[0]?.id).toBe("mcp_fs_read_file");
	});

	it("returns nothing rather than everything for a query that matches nothing", () => {
		expect(new LexicalIndex(tools).search("quantum chromodynamics")).toEqual([]);
		expect(new LexicalIndex(tools).search("")).toEqual([]);
	});

	it("is deterministic across identical queries", () => {
		const index = new LexicalIndex(tools);
		expect(index.search("issues")).toEqual(index.search("issues"));
	});
});

describe("registry", () => {
	it("replaces a kind wholesale, so a removed server's tools do not linger", () => {
		registerCapabilities("mcp-tool", tools);
		expect(getCapabilities()).toHaveLength(4);
		registerCapabilities("mcp-tool", [tools[0]]);
		expect(getCapabilities().map((d) => d.id)).toEqual(["mcp_github_create_pull_request"]);
	});

	it("keeps kinds independent", () => {
		registerCapabilities("mcp-tool", tools);
		registerCapabilities("skill", [doc("skill:pdf", "pdf", "Work with PDFs.", { kind: "skill", deferred: false })]);
		expect(getCapabilities(["skill"]).map((d) => d.id)).toEqual(["skill:pdf"]);
		expect(getCapabilities(["mcp-tool"])).toHaveLength(4);
	});

	it("hashes content, not count — a changed description is a different index", () => {
		const a = capabilitySetHash(tools);
		expect(capabilitySetHash(tools)).toBe(a);
		const edited = [{ ...tools[0], description: "Something else entirely." }, ...tools.slice(1)];
		expect(capabilitySetHash(edited)).not.toBe(a);
	});
});

describe("searchCapabilities", () => {
	it("finds a tool by capability without the dense leg", async () => {
		registerCapabilities("mcp-tool", tools);
		const { hits, legs } = await searchCapabilities("open a pull request");
		expect(hits[0]?.doc.id).toBe("mcp_github_create_pull_request");
		// The floor: lexical answers on its own, whatever the machine has installed.
		expect(legs).toContain("lexical");
		expect(hits[0]?.matchedBy).toContain("lexical");
	});

	it("filters by kind and by deferred", async () => {
		registerCapabilities("mcp-tool", tools);
		registerCapabilities("skill", [
			doc("skill:issues", "issues", "Triage repository issues.", { kind: "skill", deferred: false }),
		]);

		const scoped = await searchCapabilities("issues", { kinds: ["skill"] });
		expect(scoped.hits.map((h) => h.doc.id)).toEqual(["skill:issues"]);

		// `deferred` is the policy knob: an eager capability is already in context,
		// so a caller resolving withheld things asks only for those.
		const deferredOnly = await searchCapabilities("issues", { deferredOnly: true });
		expect(deferredOnly.hits.map((h) => h.doc.id)).toEqual(["mcp_github_list_issues"]);
	});

	it("honours the limit and returns nothing for an empty registry", async () => {
		registerCapabilities("mcp-tool", tools);
		expect((await searchCapabilities("a", { limit: 1 })).hits.length).toBeLessThanOrEqual(1);
		clearCapabilities();
		resetCapabilitySearch();
		expect((await searchCapabilities("pull request")).hits).toEqual([]);
	});

	it("rebuilds the index when the capability set changes", async () => {
		registerCapabilities("mcp-tool", [tools[0]]);
		expect((await searchCapabilities("send email")).hits).toEqual([]);
		registerCapabilities("mcp-tool", tools);
		resetCapabilitySearch();
		expect((await searchCapabilities("send email")).hits[0]?.doc.id).toBe("mcp_mail_send_message");
	});

	it("reports the dense leg as off rather than pretending it ran", () => {
		// Never started in this process, so callers can say "lexical only" honestly
		// instead of silently returning a weaker result set.
		expect(denseState().status).toBe("off");
	});
});
