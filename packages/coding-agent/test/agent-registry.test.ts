import { mkdirSync, mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, describe, expect, test } from "vitest";
import type { AgentDefinition } from "../src/core/agent-frontmatter.js";
import { AgentRegistry, loadAgentRegistry } from "../src/core/agent-registry.js";

const tmpDirs: string[] = [];

function makeTmp(): string {
	const dir = mkdtempSync(join(tmpdir(), "agent-registry-"));
	tmpDirs.push(dir);
	return dir;
}

function writeAgent(dir: string, name: string, body: string, extraFrontmatter = ""): void {
	mkdirSync(dir, { recursive: true });
	const content = `---\nname: ${name}\ndescription: ${name} agent for testing purposes.\n${extraFrontmatter}---\n${body}`;
	writeFileSync(join(dir, `${name}.md`), content);
}

afterEach(() => {
	// Best-effort cleanup; tmp dirs are removed by the OS regardless.
	tmpDirs.length = 0;
});

describe("AgentRegistry", () => {
	test("register/get/has/list", () => {
		const reg = new AgentRegistry();
		const def: AgentDefinition = {
			name: "custom",
			description: "x",
			prompt: "p",
			source: "project",
		};
		reg.register(def);
		expect(reg.has("custom")).toBe(true);
		expect(reg.get("custom")).toBe(def);
		expect(reg.list()).toHaveLength(1);
	});

	test("re-registering the same name records a collision and the later wins", () => {
		const reg = new AgentRegistry();
		reg.register({ name: "dup", description: "a", prompt: "1", source: "user" });
		reg.register({ name: "dup", description: "b", prompt: "2", source: "project" });
		expect(reg.get("dup")?.prompt).toBe("2");
		expect(reg.get("dup")?.source).toBe("project");
		expect(reg.getDiagnostics().some((d) => d.type === "collision")).toBe(true);
	});
});

describe("loadAgentRegistry", () => {
	test("loads embedded built-in agents by default", () => {
		const reg = loadAgentRegistry({ cwd: makeTmp(), agentDir: makeTmp(), includeClaude: false });
		expect(reg.has("explore")).toBe(true);
		expect(reg.has("edit")).toBe(true);
		expect(reg.list().length).toBeGreaterThanOrEqual(4);
	});

	test("project agents override user agents and built-ins", () => {
		const cwd = makeTmp();
		const agentDir = makeTmp();
		writeAgent(join(agentDir, "agents"), "explore", "USER explore prompt");
		writeAgent(join(cwd, ".hoocode", "agents"), "explore", "PROJECT explore prompt");

		const reg = loadAgentRegistry({ cwd, agentDir, includeClaude: false });
		const explore = reg.get("explore");
		expect(explore?.source).toBe("project");
		expect(explore?.prompt).toBe("PROJECT explore prompt");
	});

	test("imports .claude/agents natively (D7)", () => {
		const cwd = makeTmp();
		const agentDir = makeTmp();
		writeAgent(join(cwd, ".claude", "agents"), "claude-helper", "From .claude", "tools: Read, Glob\n");

		const reg = loadAgentRegistry({ cwd, agentDir, includeBuiltins: false });
		const helper = reg.get("claude-helper");
		expect(helper).toBeDefined();
		expect(helper?.source).toBe("claude-project");
		expect(helper?.tools).toEqual(["read", "find"]);
	});

	test("native project agents take precedence over imported .claude project agents", () => {
		const cwd = makeTmp();
		const agentDir = makeTmp();
		writeAgent(join(cwd, ".claude", "agents"), "shared", "CLAUDE version");
		writeAgent(join(cwd, ".hoocode", "agents"), "shared", "HOOCODE version");

		const reg = loadAgentRegistry({ cwd, agentDir, includeBuiltins: false });
		expect(reg.get("shared")?.source).toBe("project");
		expect(reg.get("shared")?.prompt).toBe("HOOCODE version");
	});

	test("ignores subdirectories and non-md files (runtime dispatch dirs)", () => {
		const cwd = makeTmp();
		const agentDir = makeTmp();
		const projectAgents = join(cwd, ".hoocode", "agents");
		writeAgent(projectAgents, "real", "real agent");
		// Simulate a runtime dispatch directory and a stray non-md file.
		mkdirSync(join(projectAgents, "dispatch-123"), { recursive: true });
		writeFileSync(join(projectAgents, "notes.txt"), "ignore me");

		const reg = loadAgentRegistry({ cwd, agentDir, includeBuiltins: false, includeClaude: false });
		expect(reg.list()).toHaveLength(1);
		expect(reg.get("real")).toBeDefined();
	});
});
