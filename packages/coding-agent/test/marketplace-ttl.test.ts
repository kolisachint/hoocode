/**
 * Marketplace index freshness (architecture step 8).
 *
 * Indices used to be fetched exactly once and never again, so a plugin added
 * upstream stayed permanently invisible — the inherit half of the system quietly
 * stopped inheriting. These cover the TTL and the explicit refresh, using a local
 * git remote so nothing here touches the network.
 */

import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import { listAvailablePlugins, refreshMarketplaces } from "../src/core/extensions/plugins/install.js";
import {
	marketplaceCacheDir,
	marketplaceCacheMetaPath,
	marketplaceStorePath,
} from "../src/core/extensions/plugins/locations.js";
import { writeMarketplaceStore } from "../src/core/extensions/plugins/marketplace.js";

function git(cwd: string, ...args: string[]): void {
	execFileSync("git", args, { cwd, stdio: "ignore" });
}

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2), "utf8");
}

describe("marketplace index freshness", () => {
	let home: string;
	let remote: string;
	let cwd: string;
	let prior: string | undefined;

	beforeEach(() => {
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-ttl-home-"));
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-ttl-cwd-"));
		prior = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = path.join(home, ".hoocode");

		// A real git repo standing in for a remote marketplace.
		remote = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-ttl-remote-"));
		git(remote, "init", "-q", "-b", "main");
		git(remote, "config", "user.email", "t@t.test");
		git(remote, "config", "user.name", "t");
		writeJson(path.join(remote, ".agents-plugin", "marketplace.json"), {
			name: "remote-market",
			plugins: [{ name: "first", source: "./plugins/first" }],
		});
		git(remote, "add", "-A");
		git(remote, "commit", "-qm", "one");
	});

	afterEach(() => {
		if (prior === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = prior;
		for (const d of [home, cwd, remote]) fs.rmSync(d, { recursive: true, force: true });
	});

	/** Register the local remote as a user-added marketplace, cloned into the cache. */
	function addAsUserMarketplace(): string {
		const dir = marketplaceCacheDir(remote);
		fs.mkdirSync(path.dirname(dir), { recursive: true });
		execFileSync("git", ["clone", "-q", remote, dir], { stdio: "ignore" });
		writeMarketplaceStore(marketplaceStorePath(), [{ location: remote, dir }]);
		return dir;
	}

	it("an added marketplace is visible, and a later upstream plugin is not until refresh", async () => {
		addAsUserMarketplace();
		expect(listAvailablePlugins(cwd).map((p) => p.name)).toContain("first");

		// Upstream gains a plugin.
		writeJson(path.join(remote, ".agents-plugin", "marketplace.json"), {
			name: "remote-market",
			plugins: [
				{ name: "first", source: "./plugins/first" },
				{ name: "second", source: "./plugins/second" },
			],
		});
		git(remote, "add", "-A");
		git(remote, "commit", "-qm", "two");

		// Still invisible: the cache is a clone, not a live view. This is exactly
		// the staleness the TTL exists to bound.
		expect(listAvailablePlugins(cwd).map((p) => p.name)).not.toContain("second");

		await refreshMarketplaces(cwd);
		expect(listAvailablePlugins(cwd).map((p) => p.name)).toContain("second");
	}, 30_000);

	it("records a fetch timestamp so staleness can be judged", async () => {
		addAsUserMarketplace();
		await refreshMarketplaces(cwd);
		const meta = JSON.parse(fs.readFileSync(marketplaceCacheMetaPath(), "utf8")) as Record<string, string>;
		expect(meta[remote]).toBeDefined();
		expect(Number.isFinite(Date.parse(meta[remote]))).toBe(true);
	}, 30_000);

	it("keeps the stale copy when a refresh fails, rather than leaving nothing", async () => {
		const dir = addAsUserMarketplace();
		expect(listAvailablePlugins(cwd).map((p) => p.name)).toContain("first");

		// Remote disappears — the offline case.
		fs.rmSync(remote, { recursive: true, force: true });
		const { errors } = await refreshMarketplaces(cwd);
		expect(errors.length).toBeGreaterThan(0);

		// An out-of-date index beats no index at all.
		expect(fs.existsSync(dir)).toBe(true);
		expect(listAvailablePlugins(cwd).map((p) => p.name)).toContain("first");
	}, 30_000);
});
