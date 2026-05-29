import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { FooterDataProvider } from "../src/core/footer-data-provider.js";
import { taskStore } from "../src/core/task-store.js";
import { FooterComponent } from "../src/modes/interactive/components/footer.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";
import { createHarness, type Harness } from "./suite/harness.js";

describe("footer task list rendering", () => {
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

	test("renders nothing task-related when the store is empty", () => {
		const out = renderFooter();
		expect(out).not.toContain("[subagent:");
		expect(out).not.toContain("#");
	});

	test("renders tasks with status icons, ids, titles, and subagent tags", () => {
		const explore = taskStore.create("SSE watch endpoint", { subagentMode: "explore" });
		const edit = taskStore.create("Auth refactor", { subagentMode: "edit" });
		const plain = taskStore.create("Init project");

		taskStore.update(explore.id, { status: "pending" });
		taskStore.update(edit.id, { status: "in_progress" });
		taskStore.update(plain.id, { status: "done" });

		const out = renderFooter();

		// Titles and ids
		expect(out).toContain(`#${explore.id}`);
		expect(out).toContain("SSE watch endpoint");
		expect(out).toContain("Auth refactor");
		expect(out).toContain("Init project");

		// Subagent tags appear only for subagent-owned tasks
		expect(out).toContain("[subagent:explore]");
		expect(out).toContain("[subagent:edit]");

		// Status glyphs
		expect(out).toContain("●"); // pending
		expect(out).toContain("◐"); // in_progress
		expect(out).toContain("✓"); // done
	});

	test("a non-subagent task shows no subagent tag on its line", () => {
		const out = renderFooter();
		const initLine = out.split("\n").find((line) => line.includes("Init project"));
		expect(initLine).toBeDefined();
		expect(initLine).not.toContain("[subagent:");
	});
});
