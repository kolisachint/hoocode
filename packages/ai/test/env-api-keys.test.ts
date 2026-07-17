import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { findEnvKeys, getEnvApiKey } from "../src/env-api-keys.js";

/**
 * github-copilot must NOT be auto-detected from ambient GitHub tokens
 * (GH_TOKEN / GITHUB_TOKEN), which exist for repository access in CI and
 * GitHub-integrated environments. Only the explicit COPILOT_GITHUB_TOKEN
 * opts a GitHub token into Copilot inference.
 */
describe("Copilot key detection excludes ambient GitHub tokens", () => {
	const COPILOT_VARS = ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"] as const;
	let saved: Record<string, string | undefined>;

	beforeEach(() => {
		saved = {};
		for (const key of COPILOT_VARS) {
			saved[key] = process.env[key];
			delete process.env[key];
		}
	});

	afterEach(() => {
		for (const key of COPILOT_VARS) {
			if (saved[key] === undefined) delete process.env[key];
			else process.env[key] = saved[key];
		}
	});

	it("does not detect Copilot from GH_TOKEN alone", () => {
		process.env.GH_TOKEN = "gh-repo-token";
		expect(findEnvKeys("github-copilot")).toBeUndefined();
		expect(getEnvApiKey("github-copilot")).toBeUndefined();
	});

	it("does not detect Copilot from GITHUB_TOKEN alone", () => {
		process.env.GITHUB_TOKEN = "ci-token";
		expect(findEnvKeys("github-copilot")).toBeUndefined();
	});

	it("detects Copilot from the explicit COPILOT_GITHUB_TOKEN", () => {
		process.env.COPILOT_GITHUB_TOKEN = "copilot-token";
		expect(findEnvKeys("github-copilot")).toEqual(["COPILOT_GITHUB_TOKEN"]);
		expect(getEnvApiKey("github-copilot")).toBe("copilot-token");
	});
});
