import { afterEach, describe, expect, test } from "vitest";
import { websearchApiKeyNotice } from "../src/modes/interactive/websearch-warning.js";

const CREDENTIAL_ENV = [
	"WEBTOOLS_SEARCH_PROVIDER",
	"WEBTOOLS_BRAVE_API_KEY",
	"BRAVE_API_KEY",
	"WEBTOOLS_TAVILY_API_KEY",
	"TAVILY_API_KEY",
	"WEBTOOLS_SEARXNG_URL",
] as const;

const savedEnv = new Map(CREDENTIAL_ENV.map((name) => [name, process.env[name]]));

afterEach(() => {
	for (const [name, value] of savedEnv) {
		if (value === undefined) delete process.env[name];
		else process.env[name] = value;
	}
});

/** Clear every credential var, so a key in the developer's own env cannot mask a case. */
function clearCredentialEnv(): void {
	for (const name of CREDENTIAL_ENV) delete process.env[name];
}

function notice(overrides: Partial<Parameters<typeof websearchApiKeyNotice>[0]> = {}) {
	return websearchApiKeyNotice({
		activeToolNames: ["read", "websearch"],
		warnings: {},
		...overrides,
	});
}

describe("websearchApiKeyNotice", () => {
	test("warns when websearch is active with no keyed provider", () => {
		clearCredentialEnv();

		const result = notice();

		expect(result).toBeDefined();
		expect(result?.title).toContain("API key");
		const body = result?.body.join(" ") ?? "";
		expect(body).toContain("DuckDuckGo");
		expect(body).toContain("BRAVE_API_KEY");
		expect(body).toContain("/settings");
	});

	test("stays quiet when websearch is not an active tool", () => {
		clearCredentialEnv();

		expect(notice({ activeToolNames: ["read", "bash", "webfetch"] })).toBeUndefined();
	});

	test("stays quiet when the warning is turned off", () => {
		clearCredentialEnv();

		expect(notice({ warnings: { websearchApiKey: false } })).toBeUndefined();
	});

	test.each(["WEBTOOLS_BRAVE_API_KEY", "BRAVE_API_KEY", "WEBTOOLS_TAVILY_API_KEY", "TAVILY_API_KEY"])(
		"stays quiet when %s is set",
		(name) => {
			clearCredentialEnv();
			process.env[name] = "key-from-env";

			expect(notice()).toBeUndefined();
		},
	);

	test("stays quiet for a self-hosted SearXNG endpoint, which needs no key", () => {
		clearCredentialEnv();
		process.env.WEBTOOLS_SEARXNG_URL = "https://searx.internal";

		expect(notice()).toBeUndefined();
	});

	test("stays quiet when a key lives in the settings file the binary reads", () => {
		clearCredentialEnv();

		expect(notice({ search: { providers: { tavily: { api_key: "key-from-settings" } } } })).toBeUndefined();
	});

	test("warns when the settings block exists but carries no credential", () => {
		clearCredentialEnv();

		expect(notice({ search: { providers: { brave: { api_key: "  " } } } })).toBeDefined();
	});

	test("stays quiet when the keyless backend is pinned by env", () => {
		clearCredentialEnv();
		process.env.WEBTOOLS_SEARCH_PROVIDER = "duckduckgo";

		expect(notice()).toBeUndefined();
	});

	test("stays quiet when the keyless backend is pinned in settings", () => {
		clearCredentialEnv();

		expect(notice({ search: { provider: "duckduckgo" } })).toBeUndefined();
	});
});
