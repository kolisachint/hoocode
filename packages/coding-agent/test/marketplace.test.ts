import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
	normalizePlatforms,
	parseMarketplaceDir,
	readMarketplaceStore,
	resolvePluginSource,
	writeMarketplaceStore,
} from "../src/core/extensions/plugins/marketplace.js";

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2));
}

describe("marketplace manifests", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-market-"));
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("parses a native .agents-plugin marketplace manifest", () => {
		const dir = path.join(tempDir, "agents-market");
		writeJson(path.join(dir, ".agents-plugin", "marketplace.json"), {
			name: "native",
			owner: "me",
			plugins: [{ name: "foo", source: "./plugins/foo", description: "Foo" }],
		});
		const market = parseMarketplaceDir(dir);
		expect(market?.format).toBe("agents");
		expect(market?.name).toBe("native");
		expect(market?.plugins).toEqual([{ name: "foo", source: "./plugins/foo", description: "Foo" }]);
	});

	// The three shapes below all appear in anthropics/claude-plugins-official and
	// github/awesome-copilot. Each was dropped or mis-resolved before; they are
	// pinned here because the only other place they are checked is the network.
	it("treats a url source carrying `path` as a git subdirectory", () => {
		const dir = path.join(tempDir, "url-with-path");
		writeJson(path.join(dir, ".claude-plugin", "marketplace.json"), {
			name: "official-like",
			plugins: [
				{
					name: "atomic-agents",
					source: {
						source: "url",
						url: "https://github.com/BrainBlend-AI/atomic-agents.git",
						path: "claude-plugin/atomic-agents",
						sha: "b15ca449a81278b1c92666bdf9a2e57a817dcacd",
					},
				},
			],
		});
		// Dropping `path` installed the whole repository instead of the plugin.
		expect(parseMarketplaceDir(dir)?.plugins[0].source).toEqual({
			source: "git-subdir",
			url: "https://github.com/BrainBlend-AI/atomic-agents.git",
			path: "claude-plugin/atomic-agents",
			sha: "b15ca449a81278b1c92666bdf9a2e57a817dcacd",
		});
	});

	it("accepts `commit` as the pin when `sha` is absent", () => {
		const dir = path.join(tempDir, "commit-pin");
		writeJson(path.join(dir, ".claude-plugin", "marketplace.json"), {
			name: "official-like",
			plugins: [
				{ name: "jfrog", source: { source: "github", repo: "jfrog/claude-plugin", commit: "259c8e71" } },
				{ name: "sha-wins", source: { source: "github", repo: "o/r", commit: "259c8e71", sha: "3ae8f772" } },
			],
		});
		const plugins = parseMarketplaceDir(dir)?.plugins ?? [];
		expect(plugins[0].source).toEqual({
			source: "url",
			url: "https://github.com/jfrog/claude-plugin.git",
			sha: "259c8e71",
		});
		// `sha` is the documented field, so it wins when a catalog ships both.
		expect(plugins[1].source).toEqual({ source: "url", url: "https://github.com/o/r.git", sha: "3ae8f772" });
	});

	it("normalizes the github shorthand with path, ref and sha", () => {
		const dir = path.join(tempDir, "gh-shorthand");
		writeJson(path.join(dir, ".github", "plugin", "marketplace.json"), {
			name: "awesome-copilot-like",
			plugins: [
				{ name: "figma", source: { source: "github", repo: "figma/mcp-server-guide" } },
				{
					name: "copilot-goal-skill",
					source: {
						source: "github",
						repo: "gsemet/copilot-goal-skill",
						path: "plugins/copilot-goal-skill",
						ref: "1.1.2",
						sha: "17d0452f",
					},
				},
			],
		});
		const plugins = parseMarketplaceDir(dir)?.plugins ?? [];
		expect(plugins[0].source).toEqual({ source: "url", url: "https://github.com/figma/mcp-server-guide.git" });
		expect(plugins[1].source).toEqual({
			source: "git-subdir",
			url: "https://github.com/gsemet/copilot-goal-skill.git",
			path: "plugins/copilot-goal-skill",
			ref: "1.1.2",
			sha: "17d0452f",
		});
	});

	it("prefers the native .agents-plugin manifest over Claude and Copilot", () => {
		const dir = path.join(tempDir, "triple");
		writeJson(path.join(dir, ".agents-plugin", "marketplace.json"), { name: "from-agents", plugins: [] });
		writeJson(path.join(dir, ".claude-plugin", "marketplace.json"), { name: "from-claude", plugins: [] });
		writeJson(path.join(dir, ".github", "marketplace.json"), { name: "from-copilot", plugins: [] });
		expect(parseMarketplaceDir(dir)?.name).toBe("from-agents");
		expect(parseMarketplaceDir(dir)?.format).toBe("agents");
	});

	it("parses a Claude marketplace manifest", () => {
		const dir = path.join(tempDir, "claude-market");
		writeJson(path.join(dir, ".claude-plugin", "marketplace.json"), {
			name: "demo",
			owner: "me",
			plugins: [{ name: "foo", source: "./plugins/foo", description: "Foo" }],
		});
		const market = parseMarketplaceDir(dir);
		expect(market?.format).toBe("claude");
		expect(market?.name).toBe("demo");
		expect(market?.plugins).toEqual([{ name: "foo", source: "./plugins/foo", description: "Foo" }]);
	});

	it("parses a Copilot-style .github marketplace manifest", () => {
		const dir = path.join(tempDir, "copilot-market");
		writeJson(path.join(dir, ".github", "marketplace.json"), {
			name: "gh",
			plugins: [{ name: "bar", source: "https://github.com/x/bar.git" }],
		});
		const market = parseMarketplaceDir(dir);
		expect(market?.format).toBe("copilot");
		expect(market?.plugins[0].name).toBe("bar");
	});

	it("parses the real-world Copilot index location (.github/plugin/marketplace.json) and the github source shape", () => {
		// Mirrors github/copilot-plugins: index under .github/plugin/, entries with
		// { source: "github", repo, path? } and metadata objects.
		const dir = path.join(tempDir, "copilot-plugin-market");
		writeJson(path.join(dir, ".github", "plugin", "marketplace.json"), {
			name: "copilot-plugins",
			metadata: { description: "GitHub Copilot plugins", version: "1.0.0" },
			owner: { name: "GitHub" },
			plugins: [
				{ name: "workiq", source: { source: "github", repo: "microsoft/work-iq", path: "plugins/workiq" } },
				{
					name: "advanced-security",
					source: { source: "github", repo: "github/copilot-advanced-security-plugin" },
				},
				{ name: "spark", source: "./plugins/spark" },
			],
		});
		const market = parseMarketplaceDir(dir);
		expect(market?.format).toBe("copilot");
		expect(market?.supportPlatform).toEqual(["github"]);
		expect(market?.plugins.map((p) => p.name)).toEqual(["workiq", "advanced-security", "spark"]);

		const workiq = resolvePluginSource(market!.plugins[0].source, market!.root);
		expect(workiq).toEqual({
			kind: "git-subdir",
			url: "https://github.com/microsoft/work-iq.git",
			path: "plugins/workiq",
		});
		const advSec = resolvePluginSource(market!.plugins[1].source, market!.root);
		expect(advSec).toEqual({ kind: "git", url: "https://github.com/github/copilot-advanced-security-plugin.git" });
	});

	it("parses the canonical Copilot CLI marketplace locations (root and .plugin/)", () => {
		// Copilot CLI probe order: marketplace.json, .plugin/marketplace.json,
		// .github/plugin/marketplace.json, .claude-plugin/marketplace.json.
		const rootDir = path.join(tempDir, "root-market");
		writeJson(path.join(rootDir, "marketplace.json"), {
			name: "root-market",
			owner: { name: "Octo", email: "octo@example.com" },
			plugins: [{ name: "foo", source: "./plugins/foo" }],
		});
		expect(parseMarketplaceDir(rootDir)?.format).toBe("copilot");
		expect(parseMarketplaceDir(rootDir)?.supportPlatform).toEqual(["github"]);

		const dotDir = path.join(tempDir, "dot-market");
		writeJson(path.join(dotDir, ".plugin", "marketplace.json"), { name: "dot-market", plugins: [] });
		expect(parseMarketplaceDir(dotDir)?.format).toBe("copilot");
	});

	it("applies metadata.pluginRoot to relative plugin sources (Claude and Copilot share the field)", () => {
		const dir = path.join(tempDir, "rooted");
		writeJson(path.join(dir, ".claude-plugin", "marketplace.json"), {
			name: "rooted",
			owner: { name: "me" },
			metadata: { pluginRoot: "./plugins" },
			plugins: [
				{ name: "formatter", source: "formatter" },
				{ name: "linter", source: "./linter" },
				{ name: "remote", source: "https://github.com/x/remote.git" },
			],
		});
		const market = parseMarketplaceDir(dir);
		expect(market?.plugins.find((p) => p.name === "formatter")?.source).toBe("./plugins/formatter");
		expect(market?.plugins.find((p) => p.name === "linter")?.source).toBe("./plugins/linter");
		// Non-path sources are untouched by pluginRoot.
		expect(market?.plugins.find((p) => p.name === "remote")?.source).toBe("https://github.com/x/remote.git");
	});

	it("prefers the Claude manifest when both are present", () => {
		const dir = path.join(tempDir, "both");
		writeJson(path.join(dir, ".claude-plugin", "marketplace.json"), { name: "from-claude", plugins: [] });
		writeJson(path.join(dir, ".github", "marketplace.json"), { name: "from-copilot", plugins: [] });
		expect(parseMarketplaceDir(dir)?.name).toBe("from-claude");
	});

	it("returns null when no manifest exists", () => {
		const dir = path.join(tempDir, "empty");
		fs.mkdirSync(dir);
		expect(parseMarketplaceDir(dir)).toBeNull();
	});

	it("sets supportPlatform to the single platform for a lone index", () => {
		const dir = path.join(tempDir, "solo");
		writeJson(path.join(dir, ".github", "marketplace.json"), { name: "gh", plugins: [] });
		expect(parseMarketplaceDir(dir)?.supportPlatform).toEqual(["github"]);
	});

	it("records every platform present on conflict (github surfaced alongside the winner)", () => {
		const dir = path.join(tempDir, "conflict");
		writeJson(path.join(dir, ".claude-plugin", "marketplace.json"), { name: "c", plugins: [] });
		writeJson(path.join(dir, ".github", "marketplace.json"), { name: "g", plugins: [] });
		const market = parseMarketplaceDir(dir);
		expect(market?.format).toBe("claude"); // precedence winner
		expect(market?.supportPlatform).toEqual(["claude", "github"]); // conflict recorded, not hidden
	});

	it("folds an authored top-level supportPlatform into the resolved list", () => {
		const dir = path.join(tempDir, "authored");
		writeJson(path.join(dir, ".agents-plugin", "marketplace.json"), {
			name: "a",
			supportPlatform: ["github", "copilot"], // copilot aliases to github
			plugins: [],
		});
		expect(parseMarketplaceDir(dir)?.supportPlatform).toEqual(["agents", "github"]);
	});

	it("carries an optional per-entry supportPlatform (normalized), and omits it when absent", () => {
		const dir = path.join(tempDir, "entries");
		writeJson(path.join(dir, ".agents-plugin", "marketplace.json"), {
			name: "a",
			plugins: [
				{ name: "gated", source: "./g", supportPlatform: "gh" },
				{ name: "plain", source: "./p" },
			],
		});
		const plugins = parseMarketplaceDir(dir)?.plugins ?? [];
		expect(plugins.find((p) => p.name === "gated")?.supportPlatform).toEqual(["github"]);
		expect(plugins.find((p) => p.name === "plain")).not.toHaveProperty("supportPlatform");
	});
});

describe("normalizePlatforms", () => {
	it("normalizes strings, arrays, and aliases; drops unknowns and dedupes", () => {
		expect(normalizePlatforms("github")).toEqual(["github"]);
		expect(normalizePlatforms("copilot")).toEqual(["github"]);
		expect(normalizePlatforms("native")).toEqual(["agents"]);
		expect(normalizePlatforms(["Claude", "gh", "github", "bogus"])).toEqual(["claude", "github"]);
		expect(normalizePlatforms(undefined)).toEqual([]);
	});
});

describe("resolvePluginSource", () => {
	it("classifies git, npm, and local sources", () => {
		const root = "/market";
		expect(resolvePluginSource("https://github.com/x/y.git", root)).toEqual({
			kind: "git",
			url: "https://github.com/x/y.git",
		});
		expect(resolvePluginSource("git@github.com:x/y.git", root)).toEqual({
			kind: "git",
			url: "git@github.com:x/y.git",
		});
		expect(resolvePluginSource("npm:my-plugin", root)).toEqual({ kind: "npm", spec: "my-plugin" });
		expect(resolvePluginSource("./plugins/foo", root)).toEqual({
			kind: "local",
			path: path.resolve(root, "./plugins/foo"),
		});
	});
});

describe("marketplace store", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-market-store-"));
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("round-trips marketplace records", () => {
		const storePath = path.join(tempDir, "marketplaces.json");
		expect(readMarketplaceStore(storePath)).toEqual([]);
		writeMarketplaceStore(storePath, [{ location: "https://x/y.git", dir: "/cache/y" }]);
		expect(readMarketplaceStore(storePath)).toEqual([{ location: "https://x/y.git", dir: "/cache/y" }]);
	});
});
