import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { FooterDataProvider } from "../src/core/footer-data-provider.js";
import { FooterComponent } from "../src/modes/interactive/components/footer.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";
import { createHarness, type Harness } from "./suite/harness.js";

describe("footer rendering", () => {
	let harness: Harness;
	let provider: FooterDataProvider;
	let footer: FooterComponent;

	beforeAll(async () => {
		initTheme("dark");
		harness = await createHarness();
		provider = new FooterDataProvider(harness.tempDir);
		footer = new FooterComponent(harness.session, provider);
	});

	afterAll(() => {
		provider.dispose();
		harness.cleanup();
	});

	function renderFooter(): string {
		return footer.render(120).join("\n");
	}

	test("renders without task-related content", () => {
		const out = renderFooter();
		expect(out).not.toContain("[subagent:");
		expect(out).not.toContain("#1 "); // no task id lines
	});
});
