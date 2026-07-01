import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
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
