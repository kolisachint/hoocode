/**
 * The publish lane (architecture step 9, §3.2–§3.3).
 *
 * Two things are under test, and the second matters more than the first:
 *  - packaging does its mechanical job (strict gates, README, index entry);
 *  - it stops where the plan says it stops. Nothing here contacts a remote, and
 *    staging goes no further than an uncommitted change in a local checkout.
 */

import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { productionPluginDir } from "../src/core/extensions/plugins/locations.js";
import { parsePluginDir } from "../src/core/extensions/plugins/manifest.js";
import {
	entryNeedsSource,
	PUBLISH_RECORD_FILE,
	packagePlugin,
	readPublishRecord,
	resolvePackagePlatform,
	stageIntoMarketplace,
} from "../src/core/extensions/plugins/packaging.js";

const ENV_AGENT_DIR = "HOOCODE_CODING_AGENT_DIR";

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2), "utf8");
}

describe("plugin packaging", () => {
	let dir: string;
	let priorAgentDir: string | undefined;

	beforeEach(() => {
		dir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-pkg-"));
		// Relocate every plugin home into the sandbox so `resolvePackagePlatform`
		// reads production homes that exist only for this test.
		priorAgentDir = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = path.join(dir, "home", ".hoocode");
	});

	afterEach(() => {
		if (priorAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = priorAgentDir;
		fs.rmSync(dir, { recursive: true, force: true });
	});

	/**
	 * A minimal, gate-clean plugin at `root`, in `platform`'s own layout.
	 *
	 * The layout is not cosmetic here: publishing to an ecosystem requires that
	 * ecosystem's manifest, so a fixture written in the wrong one is a plugin that
	 * genuinely cannot be published — which is its own test, below.
	 */
	function plugin(
		root: string,
		id: string,
		opts: { platform?: "claude" | "github" | "agents"; manifest?: Record<string, unknown> } = {},
	): string {
		const manifestRel =
			opts.platform === "github"
				? "plugin.json"
				: opts.platform === "agents"
					? path.join(".agents-plugin", "plugin.json")
					: path.join(".claude-plugin", "plugin.json");
		writeJson(path.join(root, manifestRel), {
			name: id,
			description: `The ${id} plugin.`,
			version: "1.2.3",
			...opts.manifest,
		});
		const skill = path.join(root, "skills", "s");
		fs.mkdirSync(skill, { recursive: true });
		fs.writeFileSync(path.join(skill, "SKILL.md"), "---\nname: s\ndescription: d\n---\n\nBody.\n");
		return root;
	}

	function parse(root: string) {
		const parsed = parsePluginDir(root);
		if (!parsed) throw new Error(`fixture at ${root} did not parse`);
		return parsed;
	}

	it("gates, writes a README and an index entry, and records what it produced", async () => {
		const root = plugin(path.join(dir, "loose"), "packme");
		const result = await packagePlugin(parse(root), "claude");

		expect(result.ok).toBe(true);
		expect(result.written).toContain("README.md");
		expect(result.written).toContain(PUBLISH_RECORD_FILE);

		const readme = fs.readFileSync(path.join(root, "README.md"), "utf8");
		expect(readme).toContain("# packme");
		expect(readme).toContain("The packme plugin.");
		expect(readme).toContain("- Skills");
		// The install section is the target platform's, not a generic one.
		expect(readme).toContain("claude plugin install packme@<marketplace>");
		expect(readme).not.toContain("copilot plugin install");

		expect(result.entry.name).toBe("packme");
		expect(result.entry.version).toBe("1.2.3");

		const record = readPublishRecord(root);
		expect(record?.platform).toBe("claude");
		expect(record?.ok).toBe(true);
		expect(record?.entry.name).toBe("packme");
	});

	it("writes the GitHub install path when packaging for github", async () => {
		const root = plugin(path.join(dir, "gh"), "ghplug", { platform: "github" });
		const result = await packagePlugin(parse(root), "github");
		expect(result.platform).toBe("github");
		const readme = fs.readFileSync(path.join(root, "README.md"), "utf8");
		expect(readme).toContain("copilot plugin install ghplug@<marketplace>");
		// GitHub has no drop-in, so the lane must not offer one.
		expect(readme).not.toContain(".claude/skills");
		expect(result.instructions).toContain("only route into the ecosystem");
	});

	it("never overwrites a hand-written README", async () => {
		const root = plugin(path.join(dir, "documented"), "documented");
		fs.writeFileSync(path.join(root, "README.md"), "# Mine\n\nHand written.\n");
		const result = await packagePlugin(parse(root), "claude");
		expect(result.written).not.toContain("README.md");
		expect(fs.readFileSync(path.join(root, "README.md"), "utf8")).toBe("# Mine\n\nHand written.\n");
	});

	it("refuses to green-light a plugin that fails the strict gates", async () => {
		// A warning, not an error: tolerable locally, not tolerable in something
		// other people install — which is exactly what strict is for.
		const root = plugin(path.join(dir, "risky"), "risky", {
			manifest: { mcpServers: { svc: { command: "definitely-not-a-real-binary-xyz" } } },
		});
		const result = await packagePlugin(parse(root), "claude");
		expect(result.ok).toBe(false);
		expect(result.instructions).toContain("NOT ready to publish");
		// Still records the attempt, so the state on disk matches the verdict.
		expect(readPublishRecord(root)?.ok).toBe(false);
	});

	it("refuses a plugin that has no manifest for the target ecosystem, and writes nothing", async () => {
		// The native layout belongs to no marketplace, so this plugin is not a
		// claude artifact and cannot become one by being packaged.
		const root = plugin(path.join(dir, "native"), "native", { platform: "agents" });
		const result = await packagePlugin(parse(root), "claude");
		expect(result.ok).toBe(false);
		expect(result.written).toEqual([]);
		expect(result.instructions).toContain("carries no claude manifest");
		expect(fs.existsSync(path.join(root, "README.md"))).toBe(false);
		expect(fs.existsSync(path.join(root, PUBLISH_RECORD_FILE))).toBe(false);
	});

	it("marks the source as needing a human when the plugin is not in a git checkout", async () => {
		const root = plugin(path.join(dir, "orphan"), "orphan");
		const result = await packagePlugin(parse(root), "claude");
		// The sandbox is a temp dir; if it somehow sits inside a repo, the origin is
		// a real URL and the placeholder branch does not apply.
		if (entryNeedsSource(result.entry)) {
			expect(result.instructions).toContain("Replace `source`");
		} else {
			expect(typeof result.entry.source).toBe("object");
		}
	});

	describe("platform resolution", () => {
		it("takes the platform from the production home the plugin lives in", () => {
			const ghDir = productionPluginDir("github", "homed");
			fs.mkdirSync(ghDir, { recursive: true });
			// Declares claude, but lives in the github home — where it lives wins.
			plugin(ghDir, "homed", { platform: "claude" });
			expect(resolvePackagePlatform(parse(ghDir))).toBe("github");
		});

		it("falls back to what a plugin outside any production home declares", () => {
			const root = plugin(path.join(dir, "declared"), "declared", { platform: "github" });
			expect(resolvePackagePlatform(parse(root))).toBe("github");
		});
	});

	describe("staging into a local marketplace", () => {
		function marketplace(name: string, index?: unknown): string {
			const root = path.join(dir, name);
			fs.mkdirSync(root, { recursive: true });
			if (index) writeJson(path.join(root, ".claude-plugin", "marketplace.json"), index);
			return root;
		}

		it("vendors the plugin and adds an entry pointing at it", async () => {
			const root = plugin(path.join(dir, "src-a"), "vendored");
			await packagePlugin(parse(root), "claude");
			const market = marketplace("market-a", { name: "m", plugins: [{ name: "other", source: "./plugins/other" }] });

			const staged = stageIntoMarketplace(parse(root), "claude", market);
			expect(staged.ok).toBe(true);
			expect(staged.createdIndex).toBe(false);
			expect(fs.existsSync(path.join(market, "plugins", "vendored", "skills", "s", "SKILL.md"))).toBe(true);

			const index = JSON.parse(fs.readFileSync(path.join(market, ".claude-plugin", "marketplace.json"), "utf8"));
			expect(index.plugins).toHaveLength(2);
			const entry = index.plugins.find((p: { name: string }) => p.name === "vendored");
			// Vendored in the repo, so the source is a path within it — not wherever
			// the plugin happened to be authored.
			expect(entry.source).toBe("./plugins/vendored");
			expect(entry.version).toBe("1.2.3");
		});

		it("leaves hoocode's internal files out of the published artifact", async () => {
			const root = plugin(path.join(dir, "src-b"), "clean");
			fs.writeFileSync(path.join(root, ".authored.json"), '{"authored":true}\n');
			await packagePlugin(parse(root), "claude");
			const market = marketplace("market-b", { name: "m", plugins: [] });

			stageIntoMarketplace(parse(root), "claude", market);
			const dest = path.join(market, "plugins", "clean");
			expect(fs.existsSync(path.join(dest, PUBLISH_RECORD_FILE))).toBe(false);
			expect(fs.existsSync(path.join(dest, ".authored.json"))).toBe(false);
			expect(fs.existsSync(path.join(dest, ".claude-plugin", "plugin.json"))).toBe(true);
			expect(fs.existsSync(path.join(dest, "skills", "s", "SKILL.md"))).toBe(true);
		});

		it("replaces an existing entry rather than duplicating it", async () => {
			const root = plugin(path.join(dir, "src-c"), "again");
			const market = marketplace("market-c", { name: "m", plugins: [{ name: "again", source: "./old/path" }] });

			stageIntoMarketplace(parse(root), "claude", market);
			const staged = stageIntoMarketplace(parse(root), "claude", market);
			expect(staged.message).toContain("Updated");

			const index = JSON.parse(fs.readFileSync(path.join(market, ".claude-plugin", "marketplace.json"), "utf8"));
			expect(index.plugins.filter((p: { name: string }) => p.name === "again")).toHaveLength(1);
			expect(index.plugins[0].source).toBe("./plugins/again");
		});

		it("creates the index when the marketplace repo has none yet", () => {
			const root = plugin(path.join(dir, "src-d"), "first", { platform: "github" });
			const market = marketplace("market-d");
			const staged = stageIntoMarketplace(parse(root), "github", market);
			expect(staged.createdIndex).toBe(true);
			// Per platform: a github staging writes the Copilot index, not Claude's.
			expect(staged.indexPath).toBe(path.join(".github", "marketplace.json"));
			expect(fs.existsSync(path.join(market, ".github", "marketplace.json"))).toBe(true);
		});

		it("refuses a missing directory, bad JSON, and a plugin already inside the marketplace", () => {
			const root = plugin(path.join(dir, "src-e"), "guarded");

			expect(stageIntoMarketplace(parse(root), "claude", path.join(dir, "nope")).ok).toBe(false);

			const broken = marketplace("market-e");
			fs.mkdirSync(path.join(broken, ".claude-plugin"), { recursive: true });
			fs.writeFileSync(path.join(broken, ".claude-plugin", "marketplace.json"), "{ not json", "utf8");
			const badJson = stageIntoMarketplace(parse(root), "claude", broken);
			expect(badJson.ok).toBe(false);
			expect(badJson.message).toContain("not valid JSON");

			const nested = marketplace("market-f", { name: "m", plugins: [] });
			const inside = plugin(path.join(nested, "plugins", "already"), "already");
			const selfStage = stageIntoMarketplace(parse(inside), "claude", nested);
			expect(selfStage.ok).toBe(false);
			expect(selfStage.message).toContain("already lives inside");
		});

		it("stages without committing — the artifact stays a reviewable diff", () => {
			const root = plugin(path.join(dir, "src-g"), "uncommitted");
			const market = marketplace("market-g", { name: "m", plugins: [] });
			execFileSync("git", ["init", "-q"], { cwd: market });

			stageIntoMarketplace(parse(root), "claude", market);

			// The whole trust argument rests on this: the human reviews and commits.
			const status = execFileSync("git", ["status", "--porcelain"], { cwd: market, encoding: "utf8" });
			expect(status.trim().length).toBeGreaterThan(0);
			const log = execFileSync("git", ["rev-list", "--count", "--all"], { cwd: market, encoding: "utf8" });
			expect(log.trim()).toBe("0");
		});
	});
});
