/**
 * hoo-core — HooCode built-in core extension
 *
 * A. Permission Gate    — prompts before bash/write/edit; checks modes.{mode}.auto_allow
 *                         from the merged (global + project) config; persists "always"
 *                         choices back to the global config
 * B. MCP Server Loader  — discovers ~/.hoocode/mcp-servers and ./.hoocode/mcp-servers JSON
 *                         configs, connects via JSON-RPC 2.0, registers server tools
 * C. Mode + Profile     — resolves active mode (ask/plan/build/agent/debug) and profile
 *                         (default/data/devops/…), merges system prompt from three template
 *                         layers, filters active tools, and exposes /mode, /profile,
 *                         /plan, and /approve commands
 *
 * Config merge order (lowest → highest priority):
 *   1. ~/.hoocode/agent/hoo-config.json   (global defaults)
 *   2. ./.hoocode/config.json             (project overrides — scalars win; arrays union)
 *   3. profile_detectors from project prepend global list (project markers checked first)
 */
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { readdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { Type } from "typebox";
import { isToolCallEventType } from "../../core/extensions/types.js";
// ============================================================================
// Shared paths
// ============================================================================
const HOOCODE_DIR = join(homedir(), ".hoocode");
const GLOBAL_CONFIG_PATH = join(HOOCODE_DIR, "agent", "hoo-config.json");
// ============================================================================
// Config I/O and merging
// ============================================================================
function readConfig() {
    try {
        return JSON.parse(readFileSync(GLOBAL_CONFIG_PATH, "utf8"));
    }
    catch {
        return {};
    }
}
function writeConfig(config) {
    const dir = join(HOOCODE_DIR, "agent");
    if (!existsSync(dir))
        mkdirSync(dir, { recursive: true });
    writeFileSync(GLOBAL_CONFIG_PATH, `${JSON.stringify(config, null, 2)}\n`, "utf8");
}
/**
 * Deep-merges a project-local config on top of the global config.
 *
 * Merge rules:
 * - Scalars (active_mode, active_profile): project wins if set
 * - modes[x].auto_allow: union of global + project arrays
 * - modes[x].allowed_write_paths: union of global + project arrays
 * - modes[x].enabled_tools: project wins if set, else falls back to global
 * - profiles[x].enabled_tools: project wins if set, else falls back to global
 * - profile_detectors: project list is prepended so project markers are checked first
 */
export function mergeConfigs(global, project) {
    const merged = { ...global };
    if (project.active_mode !== undefined)
        merged.active_mode = project.active_mode;
    if (project.active_profile !== undefined)
        merged.active_profile = project.active_profile;
    if (project.modes) {
        merged.modes = { ...(global.modes ?? {}) };
        for (const [mode, projectCfg] of Object.entries(project.modes)) {
            const globalCfg = global.modes?.[mode] ?? {};
            merged.modes[mode] = {
                ...globalCfg,
                ...projectCfg,
                // Union both auto_allow lists so project can extend, not just replace
                auto_allow: Array.from(new Set([...(globalCfg.auto_allow ?? []), ...(projectCfg.auto_allow ?? [])])),
                // Union allowed_write_paths so project can extend
                allowed_write_paths: Array.from(new Set([...(globalCfg.allowed_write_paths ?? []), ...(projectCfg.allowed_write_paths ?? [])])),
                // enabled_tools: project wins if set, else falls back to global
                enabled_tools: projectCfg.enabled_tools ?? globalCfg.enabled_tools,
            };
        }
    }
    if (project.profiles) {
        merged.profiles = { ...(global.profiles ?? {}) };
        for (const [profile, projectCfg] of Object.entries(project.profiles)) {
            merged.profiles[profile] = {
                ...(global.profiles?.[profile] ?? {}),
                ...projectCfg,
            };
        }
    }
    if (project.profile_detectors) {
        // Project detectors are prepended: project-specific markers are checked first
        merged.profile_detectors = [...project.profile_detectors, ...(global.profile_detectors ?? [])];
    }
    return merged;
}
/**
 * Reads the global config and optionally overlays the project-local config at
 * `./.hoocode/config.json`. Project values win on all scalar fields; arrays are
 * unioned (see mergeConfigs for full rules).
 */
export function readMergedConfig(cwd) {
    const global = readConfig();
    const projectPath = join(cwd, ".hoocode", "config.json");
    if (!existsSync(projectPath))
        return global;
    try {
        const project = JSON.parse(readFileSync(projectPath, "utf8"));
        return mergeConfigs(global, project);
    }
    catch {
        return global;
    }
}
// ============================================================================
// A. Permission Gate
// ============================================================================
const GATED_TOOLS = new Set(["bash", "write", "edit"]);
/**
 * Checks if a file path matches any of the allowed patterns.
 * Supports glob patterns with * and exact paths.
 */
function matchesAllowedPath(filePath, allowedPatterns) {
    if (allowedPatterns.length === 0)
        return true;
    for (const pattern of allowedPatterns) {
        // Exact match
        if (pattern === filePath)
            return true;
        // Glob pattern matching for *
        if (pattern.includes("*")) {
            const regex = new RegExp(`^${pattern.replace(/\*/g, ".*")}$`);
            if (regex.test(filePath))
                return true;
        }
    }
    return false;
}
function describeTool(event) {
    if (isToolCallEventType("bash", event)) {
        return `$ ${event.input.command.replace(/\s+/g, " ").slice(0, 100)}`;
    }
    if (isToolCallEventType("edit", event)) {
        const p = event.input.file_path ?? "(unknown)";
        return `edit ${p}`;
    }
    if (isToolCallEventType("write", event)) {
        const p = event.input.file_path ?? "(unknown)";
        return `write ${p}`;
    }
    return event.toolName;
}
export function setupPermissionGate(pi) {
    pi.on("tool_call", async (event, ctx) => {
        if (!GATED_TOOLS.has(event.toolName) || !ctx.hasUI)
            return;
        // Use the merged config so project-local auto_allow entries are respected
        const config = readMergedConfig(ctx.cwd);
        const mode = config.active_mode ?? "build";
        const modeCfg = config.modes?.[mode];
        const autoAllow = modeCfg?.auto_allow ?? [];
        // Check allowed_write_paths for write/edit operations
        if ((event.toolName === "write" || event.toolName === "edit") && modeCfg?.allowed_write_paths) {
            const filePath = event.input.file_path ?? "";
            if (!matchesAllowedPath(filePath, modeCfg.allowed_write_paths)) {
                return {
                    block: true,
                    reason: `Mode "${mode}" only allows writes to: ${modeCfg.allowed_write_paths.join(", ")}. ` +
                        `Attempted to ${event.toolName}: ${filePath}. ` +
                        `Switch to "/mode build" or "/mode agent" to modify source files.`,
                };
            }
        }
        if (autoAllow.includes(event.toolName))
            return;
        const choice = await ctx.ui.select(`Allow: ${describeTool(event)}`, [
            "Yes (once)",
            "No (block)",
            "Always (add to auto-allow for this mode)",
        ]);
        if (!choice || choice.startsWith("No")) {
            return { block: true, reason: "Denied by permission gate" };
        }
        if (choice.startsWith("Always")) {
            // Write "always" choices to the global config only
            const latest = readConfig();
            const currentMode = latest.active_mode ?? "build";
            latest.modes ??= {};
            latest.modes[currentMode] ??= {};
            latest.modes[currentMode].auto_allow = Array.from(new Set([...(latest.modes[currentMode].auto_allow ?? []), event.toolName]));
            writeConfig(latest);
            ctx.ui.notify(`"${event.toolName}" added to auto-allow for mode "${currentMode}"`, "info");
        }
    });
}
const mcpConnections = new Map();
function spawnMcpServer(config) {
    const proc = spawn(config.command, config.args ?? [], {
        env: { ...process.env, ...(config.env ?? {}) },
        stdio: ["pipe", "pipe", "pipe"],
    });
    let nextId = 1;
    const pending = new Map();
    const rl = createInterface({ input: proc.stdout });
    rl.on("line", (line) => {
        if (!line.trim())
            return;
        try {
            const msg = JSON.parse(line);
            if (msg.id === undefined)
                return;
            const cb = pending.get(msg.id);
            if (!cb)
                return;
            pending.delete(msg.id);
            if (msg.error)
                cb.reject(new Error(msg.error.message));
            else
                cb.resolve(msg.result);
        }
        catch {
            // ignore non-JSON server startup output
        }
    });
    proc.on("exit", () => {
        for (const cb of pending.values())
            cb.reject(new Error(`MCP server "${config.name}" exited unexpectedly`));
        pending.clear();
        mcpConnections.delete(config.name);
    });
    function rpc(method, params) {
        const id = nextId++;
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject });
            proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
        });
    }
    return {
        rpc,
        terminate: () => {
            rl.close();
            proc.kill();
        },
    };
}
async function connectMcpServer(config) {
    mcpConnections.get(config.name)?.terminate();
    const conn = spawnMcpServer(config);
    mcpConnections.set(config.name, conn);
    await conn.rpc("initialize", {
        protocolVersion: "2024-11-05",
        capabilities: { tools: {} },
        clientInfo: { name: "hoocode", version: "1.0.0" },
    });
    const toolsResult = (await conn.rpc("tools/list", {}));
    return { conn, tools: toolsResult.tools ?? [] };
}
function buildMcpSchema(tool) {
    const props = tool.inputSchema?.properties ?? {};
    const required = new Set(tool.inputSchema?.required ?? []);
    const shape = {};
    for (const [key, prop] of Object.entries(props)) {
        let field;
        switch (prop.type) {
            case "number":
            case "integer":
                field = Type.Number({ description: prop.description });
                break;
            case "boolean":
                field = Type.Boolean({ description: prop.description });
                break;
            default:
                field = Type.String({ description: prop.description });
        }
        shape[key] = required.has(key) ? field : Type.Optional(field);
    }
    return Type.Object(shape);
}
export function setupMcpLoader(pi) {
    pi.on("session_start", async (_event, ctx) => {
        const searchDirs = [join(HOOCODE_DIR, "mcp-servers"), join(ctx.cwd, ".hoocode", "mcp-servers")];
        for (const dir of searchDirs) {
            if (!existsSync(dir))
                continue;
            let files;
            try {
                files = (await readdir(dir)).filter((f) => f.endsWith(".json"));
            }
            catch {
                continue;
            }
            for (const file of files) {
                const cfgPath = join(dir, file);
                let serverConfig;
                try {
                    serverConfig = JSON.parse(readFileSync(cfgPath, "utf8"));
                    if (!serverConfig.name || !serverConfig.command) {
                        ctx.ui.notify(`MCP: config "${file}" is missing required "name" or "command"`, "warning");
                        continue;
                    }
                }
                catch (err) {
                    ctx.ui.notify(`MCP: failed to parse "${file}": ${String(err)}`, "error");
                    continue;
                }
                try {
                    const { tools } = await connectMcpServer(serverConfig);
                    for (const tool of tools) {
                        const toolName = `mcp_${serverConfig.name}_${tool.name}`;
                        const schema = buildMcpSchema(tool);
                        const capturedServer = serverConfig.name;
                        const capturedTool = tool.name;
                        pi.registerTool({
                            name: toolName,
                            label: `[MCP] ${serverConfig.name} › ${tool.name}`,
                            description: tool.description,
                            parameters: schema,
                            async execute(_toolCallId, params, signal, _onUpdate) {
                                const activeConn = mcpConnections.get(capturedServer);
                                if (!activeConn) {
                                    return {
                                        content: [
                                            {
                                                type: "text",
                                                text: `MCP server "${capturedServer}" is not connected`,
                                            },
                                        ],
                                        details: undefined,
                                    };
                                }
                                const abortPromise = new Promise((_, reject) => {
                                    signal.addEventListener("abort", () => reject(new Error("Aborted")));
                                });
                                const result = await Promise.race([
                                    activeConn.rpc("tools/call", {
                                        name: capturedTool,
                                        arguments: params,
                                    }),
                                    abortPromise,
                                ]);
                                return {
                                    content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
                                    details: undefined,
                                };
                            },
                        });
                    }
                    ctx.ui.notify(`MCP: connected "${serverConfig.name}" (${tools.length} tool${tools.length === 1 ? "" : "s"})`, "info");
                }
                catch (err) {
                    ctx.ui.notify(`MCP: failed to connect "${serverConfig.name}": ${String(err)}`, "error");
                }
            }
        }
    });
}
// ============================================================================
// C. Mode + Profile System
// ============================================================================
const DEFAULT_MODE = "build";
const DEFAULT_PROFILE = "default";
/**
 * Returns true if `marker` matches something in `cwd`.
 * Plain markers use existsSync. Glob markers (containing `*`) scan the
 * immediate directory entries — only one level, no recursion needed for
 * common cases like `*.sql` or `k8s/`.
 */
function markerExists(cwd, marker) {
    if (!marker.includes("*"))
        return existsSync(join(cwd, marker));
    const suffix = marker.replace(/^\*/, "");
    try {
        return readdirSync(cwd).some((entry) => entry.endsWith(suffix));
    }
    catch {
        return false;
    }
}
/**
 * Resolves which profile should be active.
 * Priority: config override → file-marker detection → "default"
 */
export function resolveProfile(config, cwd) {
    if (config.active_profile)
        return config.active_profile;
    for (const detector of config.profile_detectors ?? []) {
        if (markerExists(cwd, detector.marker))
            return detector.profile;
    }
    return DEFAULT_PROFILE;
}
/**
 * Merges the system prompt from up to three layers (lowest → highest priority):
 *   1. ~/.hoocode/templates/modes/{mode}/system.md        (mode behaviour)
 *   2. ~/.hoocode/templates/profiles/{profile}/context.md (domain context; skipped for "default")
 *   3. ./.hoocode/agents.md                               (project-local override; appended last)
 *
 * Each present layer is joined with a `---` separator.
 */
export function buildSystemPrompt(mode, profile, cwd) {
    const layers = [];
    function tryRead(path) {
        if (!existsSync(path))
            return undefined;
        try {
            const text = readFileSync(path, "utf8").trim();
            return text || undefined;
        }
        catch {
            return undefined;
        }
    }
    // Layer 1: mode system prompt (~/.hoocode/modes/{mode}/system.md)
    const modePrompt = tryRead(join(HOOCODE_DIR, "modes", mode, "system.md"));
    if (modePrompt)
        layers.push(modePrompt);
    // Layer 2: profile context — omit for "default" (no extra domain constraints)
    // (~/.hoocode/profiles/{profile}/context.md)
    if (profile !== DEFAULT_PROFILE) {
        const profileContext = tryRead(join(HOOCODE_DIR, "profiles", profile, "context.md"));
        if (profileContext)
            layers.push(profileContext);
    }
    // Layer 3: project-local agents.md — appended after mode + profile so it can
    // extend or override them for this specific repo
    const projectOverride = tryRead(join(cwd, ".hoocode", "agents.md"));
    if (projectOverride)
        layers.push(projectOverride);
    return layers.length > 0 ? layers.join("\n\n---\n\n") : undefined;
}
/**
 * Parses `.hoocode/plan.md` into named sections.
 *
 * Recognises both ATX headings (`## Goal`) and bold labels (`**Goal**`).
 * Section names matched (case-insensitive): Goal, Files to modify, New files,
 * Tests, Verification.
 */
export function parsePlanSections(planContent) {
    const result = { raw: planContent };
    // Match `## Heading text` or `**Heading text**` followed by content until
    // the next heading of the same style.
    const sectionPattern = /^(?:#{1,3}\s+(.+?)|(?:\*\*(.+?)\*\*))\s*\n([\s\S]*?)(?=(?:^#{1,3}\s+|\*\*[^*\n]+\*\*\s*\n)|$)/gm;
    for (const match of planContent.matchAll(sectionPattern)) {
        const heading = (match[1] ?? match[2] ?? "").toLowerCase().trim();
        const content = match[3].trim();
        if (!content)
            continue;
        if (/^goal/.test(heading)) {
            result.goal = content;
        }
        else if (/files?\s+to\s+modif|^modif/.test(heading)) {
            result.filesToModify = content;
        }
        else if (/new\s+files?/.test(heading)) {
            result.newFiles = content;
        }
        else if (/^tests?/.test(heading)) {
            result.tests = content;
        }
        else if (/^verif/.test(heading)) {
            result.verification = content;
        }
    }
    return result;
}
/**
 * Builds the user message sent to the agent when `/approve` is run.
 *
 * If the plan has recognisable sections, each is presented as a numbered step
 * so the agent works through them sequentially. Otherwise the raw plan is used.
 *
 * Execution order:
 *   1. Modify existing files
 *   2. Create new files
 *   3. Update / add tests
 *   4. Run verification commands
 */
export function buildApproveMessage(sections) {
    const steps = [];
    if (sections.goal) {
        steps.push(`**Goal:** ${sections.goal}`);
    }
    if (sections.filesToModify) {
        steps.push(`**Step 1 — Modify existing files:**\n${sections.filesToModify}`);
    }
    if (sections.newFiles) {
        steps.push(`**Step 2 — Create new files:**\n${sections.newFiles}`);
    }
    if (sections.tests) {
        steps.push(`**Step 3 — Update tests:**\n${sections.tests}`);
    }
    if (sections.verification) {
        steps.push(`**Step 4 — Verify:**\n${sections.verification}`);
    }
    if (steps.length === 0) {
        return `Execute the following plan:\n\n${sections.raw}`;
    }
    return `Execute this plan step by step. Complete each step fully before moving to the next.\n\n${steps.join("\n\n")}`;
}
// ============================================================================
// C. setupModeAndProfile
// ============================================================================
export function setupModeAndProfile(pi) {
    let cachedMode = DEFAULT_MODE;
    let cachedProfile = DEFAULT_PROFILE;
    let cachedSystemPrompt;
    // ── session_start ─────────────────────────────────────────────────────────
    // Config resolution order:
    //   1. Read global config  (~/.hoocode/agent/hoo-config.json)
    //   2. Read project config (./.hoocode/config.json) if present
    //   3. Merge — project scalars win; arrays are unioned; project detectors prepend
    //   4. Re-resolve active_mode and active_profile from the merged result
    pi.on("session_start", (_event, ctx) => {
        // Steps 1–3: merge global + project configs
        const config = readMergedConfig(ctx.cwd);
        // Step 4: resolve mode and profile from the merged config
        cachedMode = config.active_mode ?? DEFAULT_MODE;
        cachedProfile = resolveProfile(config, ctx.cwd);
        cachedSystemPrompt = buildSystemPrompt(cachedMode, cachedProfile, ctx.cwd);
        // Update footer with active mode/profile
        if (ctx.hasUI) {
            ctx.ui.setModeProfile(cachedMode, cachedProfile);
        }
        // Apply tool filter: mode enabled_tools takes priority, then profile
        const modeCfg = config.modes?.[cachedMode];
        const profileCfg = config.profiles?.[cachedProfile];
        if (modeCfg?.enabled_tools && modeCfg.enabled_tools.length > 0) {
            pi.setActiveTools(modeCfg.enabled_tools);
        }
        else if (profileCfg?.enabled_tools && profileCfg.enabled_tools.length > 0) {
            pi.setActiveTools(profileCfg.enabled_tools);
        }
    });
    // ── before_agent_start ────────────────────────────────────────────────────
    pi.on("before_agent_start", (event) => {
        if (!cachedSystemPrompt)
            return;
        return {
            systemPrompt: `${event.systemPrompt}\n\n` +
                `<!-- hoo-core: mode=${cachedMode} profile=${cachedProfile} -->\n` +
                cachedSystemPrompt,
        };
    });
    // ── /mode command ─────────────────────────────────────────────────────────
    const KNOWN_MODES = ["ask", "plan", "build", "agent", "debug"];
    pi.registerCommand("mode", {
        description: "Switch active mode. Usage: /mode <ask|plan|build|agent|debug>",
        getArgumentCompletions: (prefix) => KNOWN_MODES.filter((m) => m.startsWith(prefix)).map((m) => ({ value: m, label: m })),
        handler: async (args, ctx) => {
            const name = args.trim();
            if (!name) {
                ctx.ui.notify(`Active mode: ${cachedMode}`, "info");
                return;
            }
            const config = readConfig();
            config.active_mode = name === DEFAULT_MODE ? undefined : name;
            writeConfig(config);
            ctx.ui.notify(`Mode set to "${name}" — reloading…`, "info");
            await ctx.reload();
        },
    });
    // ── /profile command ──────────────────────────────────────────────────────
    pi.registerCommand("profile", {
        description: "Switch active profile. Usage: /profile <name>",
        getArgumentCompletions: (prefix) => {
            // Show profiles from the merged config so project-local profiles appear
            const config = readMergedConfig(".");
            const names = Object.keys(config.profiles ?? {});
            const suggestions = [DEFAULT_PROFILE, ...names.filter((n) => n !== DEFAULT_PROFILE)];
            return suggestions.filter((n) => n.startsWith(prefix)).map((n) => ({ value: n, label: n }));
        },
        handler: async (args, ctx) => {
            const name = args.trim();
            if (!name) {
                ctx.ui.notify(`Active profile: ${cachedProfile}`, "info");
                return;
            }
            const config = readConfig();
            config.active_profile = name === DEFAULT_PROFILE ? undefined : name;
            writeConfig(config);
            ctx.ui.notify(`Profile set to "${name}" — reloading…`, "info");
            await ctx.reload();
        },
    });
    // ── /plan command (shorthand for /mode plan) ──────────────────────────────
    pi.registerCommand("plan", {
        description: "Switch to plan mode. Shorthand for /mode plan.",
        getArgumentCompletions: () => [],
        handler: async (_args, ctx) => {
            const config = readConfig();
            config.active_mode = "plan";
            writeConfig(config);
            ctx.ui.notify(`Mode set to "plan" — reloading…`, "info");
            await ctx.reload();
        },
    });
    // ── /approve command ──────────────────────────────────────────────────────
    // Reads .hoocode/plan.md, parses it into named sections (Goal, Files to
    // modify, New files, Tests, Verification), switches to build mode, then
    // injects a step-by-step execution message into the new session.
    pi.registerCommand("approve", {
        description: "Approve the current plan and switch to build mode to execute it.",
        getArgumentCompletions: () => [],
        handler: async (_args, ctx) => {
            if (cachedMode !== "plan") {
                ctx.ui.notify(`/approve is only available in plan mode (current mode: "${cachedMode}")`, "warning");
                return;
            }
            // Read ./.hoocode/plan.md written by the agent during plan mode
            const planPath = join(ctx.cwd, ".hoocode", "plan.md");
            let approveMessage;
            if (existsSync(planPath)) {
                try {
                    const raw = readFileSync(planPath, "utf8").trim();
                    if (raw) {
                        const sections = parsePlanSections(raw);
                        approveMessage = buildApproveMessage(sections);
                    }
                }
                catch {
                    ctx.ui.notify(`Could not read .hoocode/plan.md`, "error");
                    return;
                }
            }
            // Switch global config to build mode
            const config = readConfig();
            config.active_mode = "build";
            writeConfig(config);
            if (approveMessage) {
                // Open a new build-mode session and deliver the parsed plan as the
                // first user message so the agent starts executing immediately
                await ctx.newSession({
                    withSession: async (replacedCtx) => {
                        await replacedCtx.sendUserMessage(approveMessage, { deliverAs: "followUp" });
                    },
                });
            }
            else {
                ctx.ui.notify(`Switched to build mode. No .hoocode/plan.md found — describe what to build.`, "info");
                await ctx.reload();
            }
        },
    });
}
// ============================================================================
// Extension entry point
// ============================================================================
export default function hooCore(pi) {
    setupPermissionGate(pi);
    setupMcpLoader(pi);
    setupModeAndProfile(pi);
}
//# sourceMappingURL=hoo-core.js.map