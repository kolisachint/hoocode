import { setKeybindings, type TUI } from "@kolisachint/hoocode-tui";
import stripAnsi from "strip-ansi";
import { beforeAll, describe, expect, test } from "vitest";
import { KeybindingsManager } from "../src/core/keybindings.js";
import type { ToolOutputView } from "../src/core/tool-output-view.js";
import { createAllToolDefinitions } from "../src/core/tools/index.js";
import { ToolChainComponent } from "../src/modes/interactive/components/tool-chain.js";
import { ToolExecutionComponent } from "../src/modes/interactive/components/tool-execution.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

const cwd = process.cwd();
const fakeTui = { requestRender() {}, terminal: { columns: 100, rows: 40 } } as unknown as TUI;

interface Call {
	tool: string;
	args: Record<string, unknown>;
	out: string;
	isError?: boolean;
	pending?: boolean;
}

function chainOf(view: ToolOutputView, calls: Call[], id = "c"): ToolChainComponent {
	const defs = createAllToolDefinitions(cwd);
	const chain = new ToolChainComponent(view);
	calls.forEach((call, i) => {
		const block = new ToolExecutionComponent(
			call.tool,
			`${id}-${i}`,
			call.args,
			{ view },
			defs[call.tool as never],
			fakeTui,
			cwd,
		);
		if (!call.pending) {
			block.updateResult(
				{ content: [{ type: "text", text: call.out }], details: {}, isError: !!call.isError },
				false,
			);
		}
		chain.add(block);
	});
	return chain;
}

const render = (chain: ToolChainComponent) => stripAnsi(chain.render(100).join("\n"));

const RUN: Call[] = [
	{ tool: "grep", args: { pattern: "toolOutputView", path: "packages" }, out: "a\nb" },
	{ tool: "read", args: { file_path: `${cwd}/src/keys.ts` }, out: "x\ny\nz" },
	{ tool: "edit", args: { path: `${cwd}/src/keys.ts`, edits: [] }, out: "" },
];

describe("ToolChainComponent", () => {
	beforeAll(() => {
		initTheme("dark");
		setKeybindings(new KeybindingsManager());
	});

	test("collapses a run of calls to one line in radar", () => {
		const chain = chainOf("radar", RUN);
		const lines = render(chain)
			.split("\n")
			.filter((l) => l.trim());
		expect(lines).toHaveLength(1);
		expect(lines[0]).toContain("grep › read › edit");
	});

	test("shows the shape while running and what it amounted to once done", () => {
		const running = chainOf("radar", [...RUN, { tool: "bash", args: { command: "x" }, out: "", pending: true }], "r");
		expect(render(running)).toContain("grep › read › edit › bash");
		expect(render(running)).toContain("running");

		const done = chainOf("radar", RUN, "d");
		done.close("done");
		const settled = render(done);
		expect(settled).toContain("Edited");
		expect(settled).not.toContain("grep › read");
	});

	test("an interrupted chain keeps the shape and says so, claiming nothing", () => {
		// The settled phrase is a claim about what the run amounted to; a run cut
		// off partway through has no such claim to make.
		const chain = chainOf("radar", RUN, "i");
		chain.close("interrupted");
		const out = render(chain);
		expect(out).toContain("grep › read › edit");
		expect(out).toContain("interrupted");
		expect(out).not.toContain("Edited src");
	});

	test("a collapsed chain still shows why a call failed", () => {
		const chain = chainOf(
			"radar",
			[RUN[0], { tool: "bash", args: { command: "bun test" }, out: "ASSERTION FAILED", isError: true }],
			"e",
		);
		chain.close("done");
		const out = render(chain);
		expect(out).toContain("1 failed");
		expect(out).toContain("ASSERTION FAILED");
	});

	test("opening it turns the line back into the calls it stood for", () => {
		const chain = chainOf("radar", RUN, "o");
		chain.close("done");
		expect(render(chain)).not.toContain("grep");

		chain.setOpened(true);
		const opened = render(chain);
		expect(opened).toContain("grep");
		expect(opened).toContain("read");
		expect(opened).toContain("edit");
	});

	test("is a plain pass-through in glance and full", () => {
		for (const view of ["glance", "full"] as ToolOutputView[]) {
			const chain = chainOf(view, RUN, `p-${view}`);
			chain.close("done");
			const out = render(chain);
			expect(out, view).not.toContain("›");
			expect(out, view).toContain("grep");
		}
	});

	test("follows the dial when the view changes under it", () => {
		const chain = chainOf("glance", RUN, "v");
		chain.close("done");
		expect(render(chain)).not.toContain("›");
		chain.setView("radar");
		expect(render(chain)).toContain("Edited");
		chain.setView("glance");
		expect(render(chain)).not.toContain("›");
	});

	test("every tool's call line starts in the same column", () => {
		// `edit` renders its own framed diff and pads for the header band. With no
		// band to draw — a bare call line in a folded view — that padding was just
		// an indent putting it one column right of every other tool.
		const chain = chainOf(
			"glance",
			[
				{ tool: "ls", args: { path: `${cwd}/docs` }, out: "a" },
				{ tool: "read", args: { file_path: `${cwd}/a.ts` }, out: "a" },
				{ tool: "edit", args: { path: `${cwd}/a.ts`, edits: [] }, out: "" },
			],
			"align",
		);
		chain.close("done");
		const columns = render(chain)
			.split("\n")
			.filter((line) => line.includes("●"))
			.map((line) => line.indexOf("●"));
		expect(new Set(columns).size).toBe(1);
	});

	test("a failure's reason hangs off its radar row rather than starting a new column", () => {
		const chain = chainOf(
			"radar",
			[
				{ tool: "ls", args: { path: `${cwd}/docs` }, out: "a" },
				{ tool: "bash", args: { command: "bun test" }, out: "ASSERTION FAILED", isError: true },
			],
			"indent",
		);
		chain.close("done");
		chain.setOpened(true);
		const lines = render(chain).split("\n");
		const row = lines.findIndex((line) => line.includes("bash"));
		const body = lines.findIndex((line) => line.includes("ASSERTION FAILED"));
		expect(body).toBeGreaterThan(row);
		expect(lines[body].indexOf("ASSERTION")).toBeGreaterThan(lines[row].indexOf("●"));
	});

	test("reports whether it is still collecting calls", () => {
		const chain = chainOf("radar", RUN, "s");
		expect(chain.isOpen).toBe(true);
		chain.close("done");
		expect(chain.isOpen).toBe(false);
	});
});
