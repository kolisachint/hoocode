import { describe, expect, test } from "vitest";
import { buildAgentCard, resolveActiveTools } from "../src/core/a2a/agent-card.js";
import { A2A_PROTOCOL_VERSION } from "../src/core/a2a/types.js";

describe("resolveActiveTools", () => {
	test("defaults to the core coding bundle", () => {
		expect(resolveActiveTools()).toEqual(["read", "bash", "edit", "write", "grep", "find", "ls"]);
	});

	test("adds opt-in bundles when enabled", () => {
		const tools = resolveActiveTools({
			enableWebTools: true,
			enableBrowserTools: true,
			enableFileTools: true,
		});
		expect(tools).toContain("webfetch");
		expect(tools).toContain("websearch");
		expect(tools).toContain("browser_run");
		expect(tools).toContain("browser_continue");
		expect(tools).toContain("DocRead");
	});

	test("does not add opt-in bundles by default", () => {
		const tools = resolveActiveTools();
		expect(tools).not.toContain("webfetch");
		expect(tools).not.toContain("browser_run");
		expect(tools).not.toContain("DocRead");
	});
});

describe("buildAgentCard", () => {
	const baseOptions = {
		url: "http://127.0.0.1:41411",
		version: "1.2.3",
		activeTools: resolveActiveTools(),
	};

	test("produces a spec-shaped card", () => {
		const card = buildAgentCard(baseOptions);
		expect(card.protocolVersion).toBe(A2A_PROTOCOL_VERSION);
		expect(card.name).toBe("HooCode");
		expect(card.url).toBe("http://127.0.0.1:41411");
		expect(card.version).toBe("1.2.3");
		expect(card.defaultInputModes).toContain("text/plain");
		expect(card.defaultOutputModes).toContain("text/plain");
	});

	test("advertises discovery-only capabilities honestly", () => {
		const card = buildAgentCard(baseOptions);
		expect(card.capabilities).toEqual({
			streaming: false,
			pushNotifications: false,
			stateTransitionHistory: false,
		});
	});

	test("maps the default bundle to shell/edit/navigation skills", () => {
		const card = buildAgentCard(baseOptions);
		const ids = card.skills.map((s) => s.id);
		expect(ids).toContain("shell-execution");
		expect(ids).toContain("file-editing");
		expect(ids).toContain("code-navigation");
		// Opt-in bundles are absent when their tools are not active.
		expect(ids).not.toContain("web-retrieval");
		expect(ids).not.toContain("browser-automation");
		expect(ids).not.toContain("document-editing");
	});

	test("advertises browser and web skills when those tools are active", () => {
		const card = buildAgentCard({
			...baseOptions,
			activeTools: resolveActiveTools({ enableBrowserTools: true, enableWebTools: true }),
		});
		const ids = card.skills.map((s) => s.id);
		expect(ids).toContain("browser-automation");
		expect(ids).toContain("web-retrieval");
	});

	test("includes discovered SKILL.md files as skills", () => {
		const card = buildAgentCard({
			...baseOptions,
			skills: [
				{ name: "rag-search", description: "Retrieve relevant snippets\nfrom the knowledge base." },
				{ name: "pdf-report", description: "Generate a PDF report" },
			],
		});
		const rag = card.skills.find((s) => s.name === "rag-search");
		expect(rag).toBeDefined();
		expect(rag?.id).toBe("skill-rag-search");
		// Multi-line descriptions are collapsed to one line.
		expect(rag?.description).toBe("Retrieve relevant snippets from the knowledge base.");
		expect(rag?.tags).toContain("skill");
	});

	test("de-duplicates skill ids that collide with a built-in group", () => {
		const card = buildAgentCard({
			...baseOptions,
			// A user skill literally named to collide with the shell group id.
			skills: [{ name: "shell execution", description: "custom" }],
		});
		const ids = card.skills.map((s) => s.id);
		const unique = new Set(ids);
		expect(unique.size).toBe(ids.length);
	});

	test("attaches provider and documentation metadata when given", () => {
		const card = buildAgentCard({
			...baseOptions,
			provider: { organization: "HooCode", url: "https://example.com" },
			documentationUrl: "https://example.com/docs",
		});
		expect(card.provider).toEqual({ organization: "HooCode", url: "https://example.com" });
		expect(card.documentationUrl).toBe("https://example.com/docs");
	});
});
