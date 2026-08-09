/**
 * Plugin eval gates G1 and G2 — the green signal an authored or installed plugin
 * must produce before anything activates it.
 *
 * Nothing used to run between "the model writes a plugin" and "the plugin is
 * live". These gates fill that gap, and they run against the **draft dir**, not
 * a production home: a failure discards the draft, so a plugin that does not
 * pass never reaches `~/.claude/skills/` where Claude Code would load it.
 *
 *   G1  structural + conformant — does it parse, is the id legal, is the content
 *       portable, and does the *target platform* accept it
 *   G2  static safety — can the executable content plausibly run, and is it
 *       obviously dangerous
 *
 * G1 measures conformance against the vendor where it can. Round-tripping
 * through our own parser only proves the emitter and the reader agree with each
 * other; it says nothing about whether Claude Code will accept the artifact. So
 * when the `claude` CLI is present, its own `plugin validate` is the authority —
 * it is the exact check the vendor's review pipeline runs, and inventing a
 * second opinion where the vendor ships one would just be a different set of
 * bugs. Copilot has no equivalent, so `github` targets get round-trip + schema.
 *
 * G3 (sandboxed behavioral smoke) and G4 (trigger eval) are separate; see
 * docs/plugin-system-architecture.md §4.
 */

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import * as path from "node:path";
import { classifyAllowlist } from "./authoring.js";
import type { PluginPlatform } from "./formats/platform-targets.js";
import { type NormalizedPlugin, parsePluginDir } from "./manifest.js";

export interface GateFinding {
	gate: "G1" | "G2" | "G3" | "G4";
	/**
	 * `error` fails the gate. `warning` is reported and passes, unless `strict`.
	 * `info` always passes — it exists so a *successful* check can be shown to the
	 * human in the confirmation prompt without `strict` treating "the hook ran
	 * fine" as a reason to refuse.
	 */
	severity: "error" | "warning" | "info";
	message: string;
}

export interface GateResult {
	ok: boolean;
	findings: GateFinding[];
	/** How conformance was established, for the record and for the confirm prompt. */
	conformance: "claude-plugin-validate" | "round-trip";
	plugin: NormalizedPlugin | null;
}

export interface GateOptions {
	/** Target platform, which decides whether a vendor validator applies. */
	platform?: PluginPlatform;
	/** Treat warnings as errors. Used on the publish path. */
	strict?: boolean;
	/** Skip the vendor validator (tests, and callers that must stay hermetic). */
	skipVendorValidator?: boolean;
}

/**
 * Opt out of the vendor validator process-wide.
 *
 * It costs a subprocess of a few hundred milliseconds per plugin — fine once per
 * authored plugin, ruinous across a test suite that authors dozens. Also the
 * escape hatch for a CI image that has the `claude` binary but should not spend
 * the time, or a sandbox where spawning it is undesirable. The gate falls back to
 * round-trip, and says so in `conformance` rather than pretending it validated.
 */
const ENV_SKIP_VENDOR_VALIDATE = "HOOCODE_PLUGIN_SKIP_VENDOR_VALIDATE";

function vendorValidatorDisabled(): boolean {
	const v = process.env[ENV_SKIP_VENDOR_VALIDATE];
	return v === "1" || v === "true";
}

const MAX_ID_LENGTH = 64;
const VALID_ID = /^[a-z0-9][a-z0-9-]*$/;

/** Files worth scanning for portability and secret problems. Binary content is skipped. */
const TEXT_EXTENSIONS = new Set([".md", ".json", ".txt", ".yaml", ".yml", ".sh", ".js", ".ts", ".toml"]);

/**
 * Absolute paths that only exist on the author's machine. A plugin carrying one
 * works exactly once — for whoever wrote it — which is the failure mode
 * portability guidance was meant to prevent and could not, being only advice.
 */
const MACHINE_PATHS = [/\/home\/[a-z0-9._-]+\//i, /\/Users\/[a-z0-9._-]+\//i, /\b[A-Z]:\\Users\\/];

/** Credential shapes worth refusing outright rather than shipping to a marketplace. */
const SECRET_PATTERNS: ReadonlyArray<{ re: RegExp; what: string }> = [
	{ re: /\bsk-[A-Za-z0-9]{16,}/, what: "an API key (sk-…)" },
	{ re: /\bghp_[A-Za-z0-9]{20,}/, what: "a GitHub token (ghp_…)" },
	{ re: /\bAKIA[0-9A-Z]{12,}/, what: "an AWS access key id" },
	{ re: /\bAWS_SECRET_ACCESS_KEY\s*[=:]\s*\S+/, what: "an AWS secret" },
	{ re: /-----BEGIN [A-Z ]*PRIVATE KEY-----/, what: "a private key" },
];

/**
 * Shell constructs refused in a hook or MCP command. Not a sandbox and not a
 * general shell analysis — a determined command can evade any of these. It
 * catches the accident and the obvious, which is what a static gate can honestly
 * claim; G3 is where behavior gets checked.
 */
const DANGEROUS_COMMANDS: ReadonlyArray<{ re: RegExp; what: string }> = [
	{ re: /\brm\s+(-[a-zA-Z]*\s+)*-[a-zA-Z]*[rR][a-zA-Z]*f|\brm\s+-fr\b/, what: "a recursive force delete" },
	{ re: /\b(curl|wget)\b[^|]*\|\s*(ba)?sh\b/, what: "piping a download into a shell" },
	{ re: /\bmkfs(\.[a-z0-9]+)?\b|\bdd\s+[^|]*of=\/dev\//, what: "a raw device write" },
	{ re: />\s*\/dev\/(sd|nvme|disk)/, what: "a raw device write" },
	{ re: /\bchmod\s+(-[a-zA-Z]+\s+)*777\b/, what: "a world-writable chmod" },
	{ re: /\bsudo\b/, what: "a privilege escalation" },
];

function walkFiles(root: string, out: string[] = []): string[] {
	let entries: import("node:fs").Dirent[];
	try {
		entries = readdirSync(root, { withFileTypes: true });
	} catch {
		return out;
	}
	for (const entry of entries) {
		const full = path.join(root, entry.name);
		if (entry.isDirectory()) {
			walkFiles(full, out);
		} else if (entry.isFile() && TEXT_EXTENSIONS.has(path.extname(entry.name))) {
			out.push(full);
		}
	}
	return out;
}

/** First executable token of a shell command, ignoring `VAR=x` prefixes. */
export function commandBinary(command: string): string | undefined {
	for (const token of command.trim().split(/\s+/)) {
		if (!token || /^[A-Za-z_][A-Za-z0-9_]*=/.test(token)) continue;
		return token;
	}
	return undefined;
}

/** Whether `binary` resolves — on PATH, or as a path relative to the plugin. */
function binaryResolves(binary: string, root: string): boolean {
	// A template variable is resolved at run time, so it cannot be checked here.
	if (binary.includes("${")) return true;
	if (binary.includes("/") || binary.includes("\\")) {
		return existsSync(path.isAbsolute(binary) ? binary : path.resolve(root, binary));
	}
	// Shell builtins never appear on PATH.
	if (["echo", "cd", "true", "false", "test", "set", "export", ":"].includes(binary)) return true;
	try {
		execFileSync(process.platform === "win32" ? "where" : "which", [binary], {
			stdio: "ignore",
			timeout: 5_000,
		});
		return true;
	} catch {
		return false;
	}
}

/** G1: parses, legal id, portable content, and accepted by the target platform. */
function structural(
	dir: string,
	opts: GateOptions,
): { findings: GateFinding[]; conformance: GateResult["conformance"]; plugin: NormalizedPlugin | null } {
	const findings: GateFinding[] = [];
	const err = (message: string) => findings.push({ gate: "G1", severity: "error", message });
	const warn = (message: string) => findings.push({ gate: "G1", severity: "warning", message });

	const plugin = parsePluginDir(dir);
	if (!plugin) {
		err(`No recognizable plugin manifest at ${dir}.`);
		return { findings, conformance: "round-trip", plugin: null };
	}

	if (plugin.id.length > MAX_ID_LENGTH) err(`Plugin id "${plugin.id}" exceeds ${MAX_ID_LENGTH} characters.`);
	if (!VALID_ID.test(plugin.id)) {
		err(`Plugin id "${plugin.id}" must be lowercase letters, digits and hyphens, and start with one.`);
	}

	for (const file of walkFiles(dir)) {
		let content: string;
		try {
			if (statSync(file).size > 512 * 1024) continue;
			content = readFileSync(file, "utf8");
		} catch {
			continue;
		}
		const rel = path.relative(dir, file);
		for (const re of MACHINE_PATHS) {
			const hit = re.exec(content);
			if (hit) {
				err(`${rel} contains a machine-specific path ("${hit[0]}"); the plugin would only work for its author.`);
				break;
			}
		}
		for (const { re, what } of SECRET_PATTERNS) {
			if (re.test(content)) {
				err(`${rel} appears to contain ${what}.`);
				break;
			}
		}
	}

	// Vendor conformance where one exists. Round-trip alone proves only that our
	// emitter and our reader agree.
	let conformance: GateResult["conformance"] = "round-trip";
	if (opts.platform === "claude" && !opts.skipVendorValidator && !vendorValidatorDisabled()) {
		const vendor = runClaudeValidate(dir, opts.strict === true);
		if (vendor) {
			conformance = "claude-plugin-validate";
			if (!vendor.ok) err(`claude plugin validate rejected the plugin:\n${vendor.output}`);
			else if (vendor.output.trim()) warn(`claude plugin validate: ${vendor.output.trim()}`);
		}
	}

	return { findings, conformance, plugin };
}

/** Run the vendor validator, or undefined when the CLI is not installed. */
function runClaudeValidate(dir: string, strict: boolean): { ok: boolean; output: string } | undefined {
	const args = ["plugin", "validate", dir, ...(strict ? ["--strict"] : [])];
	try {
		const output = execFileSync("claude", args, { encoding: "utf8", timeout: 60_000, stdio: "pipe" });
		return { ok: true, output };
	} catch (error) {
		const e = error as { code?: string; status?: number; stdout?: string; stderr?: string };
		// No CLI on this machine, or it predates `plugin validate` — not a failure
		// of the plugin, so the gate falls back to round-trip rather than blocking.
		if (e.code === "ENOENT") return undefined;
		const output = `${e.stdout ?? ""}${e.stderr ?? ""}`;
		if (/unknown command|unrecognized|not a valid/i.test(output)) return undefined;
		return { ok: false, output };
	}
}

/** G2: the executable content can plausibly run and is not obviously dangerous. */
function safety(plugin: NormalizedPlugin | null): GateFinding[] {
	const findings: GateFinding[] = [];
	if (!plugin) return findings;
	const err = (message: string) => findings.push({ gate: "G2", severity: "error", message });
	const warn = (message: string) => findings.push({ gate: "G2", severity: "warning", message });

	const checkCommand = (label: string, command: string) => {
		for (const { re, what } of DANGEROUS_COMMANDS) {
			if (re.test(command)) err(`${label} performs ${what}: ${command}`);
		}
		const binary = commandBinary(command);
		if (!binary) {
			err(`${label} has no runnable command.`);
			return;
		}
		if (!binaryResolves(binary, plugin.root)) {
			// A warning, not an error: the binary may be installed later, or by the
			// plugin's own prerequisites. Refusing outright would block a plugin that
			// documents its dependency honestly.
			warn(`${label} runs "${binary}", which is not on PATH or in the plugin.`);
		}
	};

	for (const [event, groups] of Object.entries(plugin.hooks ?? {})) {
		for (const group of groups) {
			for (const cmd of group.hooks) checkCommand(`hook ${event}`, cmd.command);
		}
	}
	for (const [name, raw] of Object.entries(plugin.mcpServers ?? {})) {
		const cfg = raw as { command?: unknown };
		if (typeof cfg.command === "string") checkCommand(`mcp server "${name}"`, cfg.command);
	}

	// Subagent grants are classified rather than judged: the confirm gate already
	// decides on them, and recording the classification is what lets the human see
	// *why* they were asked.
	for (const agentFile of listAgentFiles(plugin)) {
		const tools = readToolsFrontmatter(agentFile);
		if (tools === undefined) continue;
		const cls = classifyAllowlist(tools);
		if (cls.pluginTools.length > 0) {
			err(
				`subagent "${path.basename(agentFile)}" grants capability-acquisition tools: ${cls.pluginTools.join(", ")}`,
			);
		} else if (cls.risk === "mutating") {
			warn(`subagent "${path.basename(agentFile)}" holds a mutating grant (${cls.reason}).`);
		}
	}

	return findings;
}

function listAgentFiles(plugin: NormalizedPlugin): string[] {
	if (!plugin.agentsDir || !existsSync(plugin.agentsDir)) return [];
	try {
		return readdirSync(plugin.agentsDir)
			.filter((f) => f.endsWith(".md"))
			.map((f) => path.join(plugin.agentsDir as string, f));
	} catch {
		return [];
	}
}

function readToolsFrontmatter(file: string): string | undefined {
	try {
		const match = /^---\r?\n([\s\S]*?)\r?\n---/.exec(readFileSync(file, "utf8"));
		return /^tools:\s*(.+)$/m.exec(match?.[1] ?? "")?.[1]?.trim();
	} catch {
		return undefined;
	}
}

/** Whether a set of findings fails the gate. `info` never does. */
export function findingsFail(findings: readonly GateFinding[], strict = false): boolean {
	return findings.some((f) => f.severity === "error" || (strict && f.severity === "warning"));
}

/** Run G1 and G2 over a plugin directory. */
export function runStaticGates(dir: string, opts: GateOptions = {}): GateResult {
	const { findings: g1, conformance, plugin } = structural(dir, opts);
	const findings = [...g1, ...safety(plugin)];
	return { ok: !findingsFail(findings, opts.strict === true), findings, conformance, plugin };
}

/** Fold additional findings (G3) into an existing result. */
export function withFindings(result: GateResult, extra: readonly GateFinding[], strict = false): GateResult {
	const findings = [...result.findings, ...extra];
	return { ...result, findings, ok: !findingsFail(findings, strict) };
}

/** Render findings for a tool result or a confirmation prompt. */
export function formatGateFindings(result: GateResult): string {
	if (result.findings.length === 0) {
		return `Checks passed (${result.conformance}).`;
	}
	const lines = result.findings.map((f) => `  [${f.gate}/${f.severity}] ${f.message}`);
	return `${result.ok ? "Checks passed with warnings" : "Checks failed"} (${result.conformance}):\n${lines.join("\n")}`;
}
