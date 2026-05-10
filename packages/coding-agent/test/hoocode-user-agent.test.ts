import { describe, expect, it } from "vitest";
import { getHooCodeUserAgent } from "../src/utils/hoocode-user-agent.js";

describe("getHooCodeUserAgent", () => {
	it("formats the user agent expected by pi.dev", () => {
		const runtime = process.versions.bun ? `bun/${process.versions.bun}` : `node/${process.version}`;
		const userAgent = getHooCodeUserAgent("1.2.3");

		expect(userAgent).toBe(`hoocode/1.2.3 (${process.platform}; ${runtime}; ${process.arch})`);
		expect(userAgent).toMatch(/^hoocode\/[^\s()]+ \([^;()]+;\s*[^;()]+;\s*[^()]+\)$/);
	});
});
