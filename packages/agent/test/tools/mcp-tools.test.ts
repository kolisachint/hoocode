import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { closeMcpTools, loadMcpTools } from "../../src/tools/mcp-tools.js";

const STUB_SERVER_PATH = join(dirname(fileURLToPath(import.meta.url)), "stub-mcp-server.mjs");

describe("loadMcpTools", () => {
	let dir: string;

	beforeAll(async () => {
		dir = await mkdtemp(join(tmpdir(), "mcp-tools-"));
	});

	afterAll(async () => {
		closeMcpTools();
		await rm(dir, { recursive: true, force: true });
	});

	it("returns [] for an empty config", async () => {
		const emptyPath = join(dir, "empty.json");
		await writeFile(emptyPath, JSON.stringify({}));
		expect(await loadMcpTools(emptyPath)).toEqual([]);

		const noServersPath = join(dir, "no-servers.json");
		await writeFile(noServersPath, JSON.stringify({ mcpServers: {} }));
		expect(await loadMcpTools(noServersPath)).toEqual([]);
	});

	it("rejects for a missing config file", async () => {
		await expect(loadMcpTools(join(dir, "does-not-exist.json"))).rejects.toThrow();
	});

	it("lists and executes tools from a stub server", async () => {
		const configPath = join(dir, "mcp.json");
		await writeFile(
			configPath,
			JSON.stringify({
				mcpServers: {
					stub: { command: process.execPath, args: [STUB_SERVER_PATH] },
				},
			}),
		);

		const tools = await loadMcpTools(configPath);
		expect(tools).toHaveLength(1);
		const echo = tools[0];
		expect(echo.name).toBe("mcp_stub_echo");
		expect(echo.description).toBe("Echo the given text back");

		const result = await echo.execute("test-call", { text: "hi there" });
		const first = result.content[0];
		expect(first?.type).toBe("text");
		expect(first?.type === "text" ? first.text : "").toContain("echo: hi there");
	});
});
