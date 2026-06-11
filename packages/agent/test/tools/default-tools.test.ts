import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { getDefaultTools } from "../../src/tools/default-tools.js";
import type { AgentTool } from "../../src/types.js";

function getTool(tools: AgentTool<any>[], name: string): AgentTool<any> {
	const tool = tools.find((t) => t.name === name);
	if (!tool) throw new Error(`Tool ${name} not found`);
	return tool;
}

async function executeText(tool: AgentTool<any>, params: Record<string, unknown>): Promise<string> {
	const result = await tool.execute("test-call", params);
	const first = result.content[0];
	if (first?.type !== "text") throw new Error("Expected text content");
	return first.text;
}

describe("getDefaultTools", () => {
	let cwd: string;
	let tools: AgentTool<any>[];

	beforeAll(async () => {
		cwd = await mkdtemp(join(tmpdir(), "default-tools-"));
		tools = getDefaultTools({ cwd });
	});

	afterAll(async () => {
		await rm(cwd, { recursive: true, force: true });
	});

	it("returns the built-in tool bundle", () => {
		expect(tools.length).toBeGreaterThanOrEqual(1);
		const names = tools.map((tool) => tool.name);
		expect(names).toEqual(["bash", "read", "edit", "write", "grep", "find", "ls"]);
		for (const tool of tools) {
			expect(typeof tool.execute).toBe("function");
			expect(tool.description.length).toBeGreaterThan(0);
		}
	});

	it("bash executes commands in the configured cwd", async () => {
		const output = await executeText(getTool(tools, "bash"), { command: "echo hello-from-bash && pwd" });
		expect(output).toContain("hello-from-bash");
		expect(output).toContain(cwd.split("/").pop() as string);
	});

	it("bash reports non-zero exit codes", async () => {
		const output = await executeText(getTool(tools, "bash"), { command: "exit 3" });
		expect(output).toContain("Exit code: 3");
	});

	it("write/read/edit roundtrip", async () => {
		await executeText(getTool(tools, "write"), { path: "notes/hello.txt", content: "alpha\nbeta\ngamma\n" });
		const read = await executeText(getTool(tools, "read"), { path: "notes/hello.txt" });
		expect(read).toContain("beta");

		await executeText(getTool(tools, "edit"), {
			path: "notes/hello.txt",
			edits: [{ oldText: "beta", newText: "delta" }],
		});
		const after = await executeText(getTool(tools, "read"), { path: "notes/hello.txt" });
		expect(after).toContain("delta");
		expect(after).not.toContain("beta");
	});

	it("edit rejects ambiguous oldText", async () => {
		await executeText(getTool(tools, "write"), { path: "dup.txt", content: "same\nsame\n" });
		await expect(
			executeText(getTool(tools, "edit"), { path: "dup.txt", edits: [{ oldText: "same", newText: "other" }] }),
		).rejects.toThrow(/matches 2 locations/);
	});

	it("read honors offset and limit", async () => {
		await executeText(getTool(tools, "write"), { path: "lines.txt", content: "one\ntwo\nthree\nfour\n" });
		const output = await executeText(getTool(tools, "read"), { path: "lines.txt", offset: 2, limit: 2 });
		expect(output).toBe("two\nthree");
	});

	it("grep finds matches with file and line info", async () => {
		await executeText(getTool(tools, "write"), {
			path: "src/code.ts",
			content: "const needle = 1;\nconst hay = 2;\n",
		});
		const output = await executeText(getTool(tools, "grep"), { pattern: "needle", glob: "*.ts" });
		expect(output).toContain("src/code.ts:1:");
		expect(output).toContain("needle");
	});

	it("grep respects the root .gitignore", async () => {
		await writeFile(join(cwd, ".gitignore"), "ignored/\n");
		await executeText(getTool(tools, "write"), { path: "ignored/secret.ts", content: "const needle = 3;\n" });
		const output = await executeText(getTool(tools, "grep"), { pattern: "needle" });
		expect(output).not.toContain("ignored/secret.ts");
	});

	it("find matches glob patterns", async () => {
		const output = await executeText(getTool(tools, "find"), { pattern: "**/*.ts" });
		expect(output).toContain("src/code.ts");
		expect(output).not.toContain("hello.txt");
	});

	it("ls lists directories with trailing slash", async () => {
		const output = await executeText(getTool(tools, "ls"), {});
		expect(output).toContain("notes/");
		expect(output).toContain("dup.txt");
	});
});
