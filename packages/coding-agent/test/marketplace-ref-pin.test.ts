/**
 * Ref-pinned marketplaces: cache discipline when the pin moves, or vanishes.
 *
 * Why this exists at all: `github/awesome-copilot` serves *stubs* on its default
 * branch — a `plugins/<name>/` directory there holds only `plugin.json` and a
 * README, and CI materializes the real content onto a separate distribution
 * branch, which is what the vendor's own installer reads. So hoocode pins that
 * marketplace to a branch it does not control, whose name lives in someone
 * else's workflow env var. Three behaviours follow, and all three are failure
 * modes rather than features, which is exactly why they are tested:
 *
 *   1. a cache sitting on the wrong branch is discarded, not faithfully pulled;
 *   2. a pin that has been retired falls back to the default branch, loudly,
 *      instead of dropping the marketplace entirely;
 *   3. those two do not fight — a fallback clone must not be discarded and
 *      re-downloaded on every subsequent run.
 *
 * Run against local bare repositories whose branches this test moves and
 * deletes, because no real marketplace will do that on demand. Nothing here
 * touches the network.
 */

import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ensureWellKnownMarketplaces } from "../src/core/extensions/plugins/install.js";
import { marketplaceCacheDir } from "../src/core/extensions/plugins/locations.js";

const MARKETPLACE_JSON = (label: string) =>
	JSON.stringify({ name: "fixture", plugins: [{ name: label, source: `plugins/${label}` }] }, null, 2);

describe("ref-pinned marketplace caches", () => {
	let home: string;
	let agentDir: string;
	let remote: string;
	let url: string;

	const git = (cwd: string, ...args: string[]) =>
		execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });

	beforeEach(() => {
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-refpin-home-"));
		agentDir = path.join(home, ".hoocode");
		fs.mkdirSync(agentDir, { recursive: true });

		// A remote with two branches whose marketplace.json differs, so which branch
		// the cache is on is observable from its content alone.
		const work = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-refpin-work-"));
		git(work, "init", "--quiet", "--initial-branch", "main", ".");
		git(work, "config", "user.email", "t@example.com");
		git(work, "config", "user.name", "T");
		fs.mkdirSync(path.join(work, ".github", "plugin"), { recursive: true });
		const indexFile = path.join(work, ".github", "plugin", "marketplace.json");
		fs.writeFileSync(indexFile, MARKETPLACE_JSON("from-main"), "utf8");
		git(work, "add", "-A");
		git(work, "commit", "--quiet", "-m", "main");
		git(work, "checkout", "--quiet", "-b", "dist");
		fs.writeFileSync(indexFile, MARKETPLACE_JSON("from-dist"), "utf8");
		git(work, "add", "-A");
		git(work, "commit", "--quiet", "-m", "dist");
		git(work, "checkout", "--quiet", "main");

		remote = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-refpin-remote-"));
		git(remote, "clone", "--quiet", "--bare", work, "origin.git");
		remote = path.join(remote, "origin.git");
		url = remote;
		fs.rmSync(work, { recursive: true, force: true });
	});

	afterEach(() => {
		fs.rmSync(home, { recursive: true, force: true });
		fs.rmSync(path.dirname(remote), { recursive: true, force: true });
	});

	const ensure = (ref?: string) =>
		ensureWellKnownMarketplaces(agentDir, {
			force: true,
			marketplaces: [{ name: "fixture", url, ...(ref ? { ref } : {}) }],
		});

	/** Which branch the cache is on, read from the content rather than from git. */
	const cachedLabel = (): string => {
		const file = path.join(marketplaceCacheDir(url, agentDir), ".github", "plugin", "marketplace.json");
		return JSON.parse(fs.readFileSync(file, "utf8")).plugins[0].name;
	};

	it("clones the pinned branch, not the default one", async () => {
		expect(await ensure("dist")).toEqual([]);
		expect(cachedLabel()).toBe("from-dist");
	});

	it("discards a cache left on the wrong branch instead of pulling it forever", async () => {
		// The upgrade path: a cache created before the pin existed.
		await ensure();
		expect(cachedLabel()).toBe("from-main");

		expect(await ensure("dist")).toEqual([]);
		expect(cachedLabel()).toBe("from-dist");
	});

	it("falls back to the default branch when the pin has been retired, and says so", async () => {
		git(remote, "branch", "-D", "dist");

		const notes = await ensure("dist");
		expect(cachedLabel()).toBe("from-main");
		expect(notes.join("\n")).toContain('branch "dist" is unavailable');
		// The consequence is the part a user cannot otherwise work out.
		expect(notes.join("\n")).toContain("may be unbuilt");
	});

	it("keeps a fallback clone put rather than re-downloading it every run", async () => {
		git(remote, "branch", "-D", "dist");
		await ensure("dist");

		// The cache is now on `main` while the pin still names `dist`. An unguarded
		// mismatch check would discard it here — and again on every later run.
		const dir = marketplaceCacheDir(url, agentDir);
		const marker = path.join(dir, ".not-rebuilt");
		fs.writeFileSync(marker, "x", "utf8");

		await ensure("dist");
		expect(fs.existsSync(marker)).toBe(true);
		expect(cachedLabel()).toBe("from-main");
	});

	it("re-pins once the branch comes back", async () => {
		git(remote, "branch", "-D", "dist");
		await ensure("dist");
		expect(cachedLabel()).toBe("from-main");

		git(remote, "branch", "dist", "refs/heads/main");
		git(remote, "symbolic-ref", "HEAD", "refs/heads/main");
		expect(await ensure("dist")).toEqual([]);
		// Back on the pin: the cache was rebuilt rather than left on the fallback.
		expect(fs.existsSync(path.join(marketplaceCacheDir(url, agentDir), ".not-rebuilt"))).toBe(false);
	});

	it("an unpinned marketplace is untouched by any of this", async () => {
		expect(await ensure()).toEqual([]);
		expect(cachedLabel()).toBe("from-main");
		expect(await ensure()).toEqual([]);
		expect(cachedLabel()).toBe("from-main");
	});
});
