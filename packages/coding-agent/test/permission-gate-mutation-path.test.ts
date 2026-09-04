/**
 * The permission gate's view of which file an edit or write targets.
 *
 * Both tools declare the argument as `path`. The gate read `file_path`, which
 * neither schema emits, so it saw nothing: the confirmation prompt asked the
 * user to approve "edit (unknown)", and `allowed_write_paths` compared an empty
 * string against its patterns and blocked every write in any mode that set one.
 */

import { mkdir, mkdtemp, writeFile } from "fs/promises";
import { tmpdir } from "os";
import { join } from "path";
import { beforeEach, describe, expect, it } from "vitest";
import { setupPermissionGate } from "../src/extensions/core/permission-gate.js";

type Handler = (event: unknown, ctx: unknown) => Promise<unknown>;

/** Capture the gate's tool_call handler from a minimal ExtensionAPI. */
function gateHandler(): Handler {
	let handler: Handler | undefined;
	setupPermissionGate({
		on: (_e: string, h: Handler) => {
			handler = h;
		},
	} as never);
	if (!handler) throw new Error("permission gate registered no tool_call handler");
	return handler;
}

async function projectWith(config: unknown): Promise<string> {
	const cwd = await mkdtemp(join(tmpdir(), "permission-gate-"));
	await mkdir(join(cwd, ".hoocode"), { recursive: true });
	await writeFile(join(cwd, ".hoocode", "hoo-config.json"), JSON.stringify(config));
	return cwd;
}

describe("permission gate: which file a mutation names", () => {
	let handler: Handler;
	beforeEach(() => {
		handler = gateHandler();
	});

	it("names the file in the approval prompt instead of asking to approve (unknown)", async () => {
		const cwd = await projectWith({ active_mode: "build", modes: { build: {} } });
		const prompts: string[] = [];
		const ctx = {
			cwd,
			hasUI: true,
			ui: {
				select: async (prompt: string) => {
					prompts.push(prompt);
					return "No (block)";
				},
			},
		};

		for (const toolName of ["edit", "write"] as const) {
			await handler({ type: "tool_call", toolName, toolCallId: "c1", input: { path: "src/app.ts" } }, ctx);
		}

		expect(prompts).toEqual(["Allow: edit src/app.ts", "Allow: write src/app.ts"]);
		for (const p of prompts) expect(p).not.toContain("(unknown)");
	});

	it("still names the file when a model sends file_path instead of path", async () => {
		const cwd = await projectWith({ active_mode: "build", modes: { build: {} } });
		const prompts: string[] = [];
		const ctx = {
			cwd,
			hasUI: true,
			ui: {
				select: async (p: string) => {
					prompts.push(p);
					return "No (block)";
				},
			},
		};

		await handler({ type: "tool_call", toolName: "edit", toolCallId: "c1", input: { file_path: "src/app.ts" } }, ctx);
		expect(prompts).toEqual(["Allow: edit src/app.ts"]);
	});

	it("lets allowed_write_paths permit a matching write rather than blocking everything", async () => {
		const cwd = await projectWith({
			active_mode: "docs",
			modes: { docs: { allowed_write_paths: ["docs/*"], auto_allow: ["edit", "write"] } },
		});
		const ctx = { cwd, hasUI: true, ui: { select: async () => "No (block)" } };

		const allowed = await handler(
			{ type: "tool_call", toolName: "edit", toolCallId: "c1", input: { path: "docs/guide.md" } },
			ctx,
		);
		expect(allowed).toBeUndefined();
	});

	it("still blocks a write outside allowed_write_paths, and says which file", async () => {
		const cwd = await projectWith({
			active_mode: "docs",
			modes: { docs: { allowed_write_paths: ["docs/*"], auto_allow: ["edit", "write"] } },
		});
		const ctx = { cwd, hasUI: true, ui: { select: async () => "No (block)" } };

		const blocked = (await handler(
			{ type: "tool_call", toolName: "edit", toolCallId: "c1", input: { path: "src/app.ts" } },
			ctx,
		)) as { block?: boolean; reason?: string };
		expect(blocked?.block).toBe(true);
		expect(blocked?.reason).toContain("src/app.ts");
	});

	it("accepts a Windows-style relative path against a forward-slash pattern", async () => {
		// `allowed_write_paths` is checked BEFORE `auto_allow`, so a non-match is a
		// hard block, not a prompt. Plan mode hands the model a path from
		// `relative()`, which is backslash-separated on Windows — matching it
		// literally against a `/` pattern would make plan mode unable to write its
		// own plan file there.
		const cwd = await projectWith({
			active_mode: "plan",
			modes: { plan: { allowed_write_paths: [".hoocode/plans/*"], auto_allow: ["write"] } },
		});
		const ctx = { cwd, hasUI: true, ui: { select: async () => "No (block)" } };

		const allowed = await handler(
			{ type: "tool_call", toolName: "write", toolCallId: "c1", input: { path: ".hoocode\\plans\\s1.md" } },
			ctx,
		);
		expect(allowed).toBeUndefined();
	});

	it("accepts an absolute path that resolves inside an allowed relative pattern", async () => {
		const cwd = await projectWith({
			active_mode: "plan",
			modes: { plan: { allowed_write_paths: [".hoocode/plans/*"], auto_allow: ["write"] } },
		});
		const ctx = { cwd, hasUI: true, ui: { select: async () => "No (block)" } };

		const allowed = await handler(
			{
				type: "tool_call",
				toolName: "write",
				toolCallId: "c1",
				input: { path: join(cwd, ".hoocode", "plans", "s1.md") },
			},
			ctx,
		);
		expect(allowed).toBeUndefined();
	});

	it("still blocks an absolute path that escapes the project root", async () => {
		const cwd = await projectWith({
			active_mode: "plan",
			modes: { plan: { allowed_write_paths: [".hoocode/plans/*"], auto_allow: ["write"] } },
		});
		const ctx = { cwd, hasUI: true, ui: { select: async () => "No (block)" } };

		const blocked = (await handler(
			{ type: "tool_call", toolName: "write", toolCallId: "c1", input: { path: "/etc/passwd" } },
			ctx,
		)) as { block?: boolean };
		expect(blocked?.block).toBe(true);
	});

	it("does not let a pattern's dots match arbitrary characters", async () => {
		// `.hoocode/plans/*` compiled to `^.hoocode/plans/.*$`, where the leading
		// `.` matched any character — so `Xhoocode/plans/evil.md` passed.
		const cwd = await projectWith({
			active_mode: "plan",
			modes: { plan: { allowed_write_paths: [".hoocode/plans/*"], auto_allow: ["write"] } },
		});
		const ctx = { cwd, hasUI: true, ui: { select: async () => "No (block)" } };

		const blocked = (await handler(
			{ type: "tool_call", toolName: "write", toolCallId: "c1", input: { path: "Xhoocode/plans/evil.md" } },
			ctx,
		)) as { block?: boolean };
		expect(blocked?.block).toBe(true);
	});

	it("blocks a mutation whose path cannot be identified at all", async () => {
		const cwd = await projectWith({
			active_mode: "docs",
			modes: { docs: { allowed_write_paths: ["docs/*"], auto_allow: ["edit", "write"] } },
		});
		const ctx = { cwd, hasUI: true, ui: { select: async () => "No (block)" } };

		const blocked = (await handler({ type: "tool_call", toolName: "edit", toolCallId: "c1", input: {} }, ctx)) as {
			block?: boolean;
		};
		expect(blocked?.block).toBe(true);
	});
});
