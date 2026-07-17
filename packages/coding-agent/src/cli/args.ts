/**
 * CLI argument parsing and help display
 */

import type { ThinkingLevel } from "@kolisachint/hoocode-agent-core";
import chalk from "chalk";
import { APP_NAME, CONFIG_DIR_NAME, ENV_AGENT_DIR, ENV_SESSION_DIR } from "../config.js";
import type { ExtensionFlag } from "../core/extensions/types.js";

export type Mode = "text" | "json" | "rpc";

export interface Args {
	provider?: string;
	model?: string;
	apiKey?: string;
	systemPrompt?: string;
	thinking?: ThinkingLevel;
	continue?: boolean;
	resume?: boolean;
	help?: boolean;
	version?: boolean;
	mode?: Mode;
	noSession?: boolean;
	/** Internal: task id assigned by SubagentPool when this process is a spawned subagent. */
	taskId?: string;
	/** Hard cap on assistant turns. Near the cap the agent is asked to wrap up; at the cap it is stopped. */
	maxTurns?: number;
	session?: string;
	/** Base URL of a hooteams server to bridge into the task panel's teams view, or "auto" to discover a config and spawn one locally. */
	team?: string;
	fork?: string;
	sessionDir?: string;
	models?: string[];
	tools?: string[];
	disallowedTools?: string[];
	noTools?: boolean;
	noBuiltinTools?: boolean;
	/** Tri-state override of the `enableSubagent` setting (default true): --enable-subagents → true, --no-subagents → false, unset → setting. */
	subagent?: boolean;
	/** Dispatch eligible subagents on reused warm RPC workers (overrides the warmSubagents setting). */
	warmSubagents?: boolean;
	/** Tree-wide subagent nesting cap (overrides the maxSubagentDepth setting). */
	maxSubagentDepth?: number;
	/** Internal: restricts which subagent types this process may delegate to. */
	delegateAllow?: string[];
	todoWrite?: boolean;
	/** Enable the webfetch + websearch tools (off by default). */
	enableWebTools?: boolean;
	/** Enable the browser_run + browser_continue tools (browsertools engine, off by default). */
	enableBrowserTools?: boolean;
	/** Default the streamed live viewer on for browser_run runs and auto-open it (off by default). */
	enableBrowserLivePreview?: boolean;
	/** Enable the document tools — DocRead/DocEdit/DocWrite + DocScan/DocGrep/DocPeek (off by default). */
	enableFileTools?: boolean;
	/** Enable the autonomous plugin system — plugin lifecycle tools (SearchPlugins, InstallPlugin, ...) and ProposePlugin (off by default). */
	enablePluginTools?: boolean;
	/** Minimal low-token preset for small/local models: read/write/edit/bash only, terse prompt, no subagents/todo/skills/context files/mode appendix. */
	light?: boolean;
	/** Print the fixed per-turn surface (system prompt + serialized tool schema token estimate) and exit. */
	printTokenSurface?: boolean;
	/**
	 * Platform layout(s) hoocode targets when it WRITES artifacts (authored
	 * plugins, /new-skill //new-agent //new-command scaffolds). Raw tokens as
	 * given (comma-separated and/or repeated); normalized downstream:
	 * agents|native, claude, github|copilot|gh.
	 */
	supportPlatform?: string[];
	/** Path to an explicit PEM CA bundle to trust additively for hoocode's own TLS traffic. */
	caCert?: string;
	/** Trust the OS/system CA store additively (opt-in) for hoocode's own TLS traffic. */
	useSystemCa?: boolean;
	extensions?: string[];
	noExtensions?: boolean;
	print?: boolean;
	export?: string;
	noSkills?: boolean;
	skills?: string[];
	agents?: string[];
	promptTemplates?: string[];
	noPromptTemplates?: boolean;
	slashCommands?: string[];
	noSlashCommands?: boolean;
	themes?: string[];
	noThemes?: boolean;
	modePaths?: string[];
	noContextFiles?: boolean;
	listModels?: string | true;
	offline?: boolean;
	verbose?: boolean;
	messages: string[];
	fileArgs: string[];
	/** Unknown flags (potentially extension flags) - map of flag name to value */
	unknownFlags: Map<string, boolean | string>;
	diagnostics: Array<{ type: "warning" | "error"; message: string }>;
}

const VALID_THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;

export function isValidThinkingLevel(level: string): level is ThinkingLevel {
	return VALID_THINKING_LEVELS.includes(level as ThinkingLevel);
}

export function parseArgs(args: string[]): Args {
	const result: Args = {
		messages: [],
		fileArgs: [],
		unknownFlags: new Map(),
		diagnostics: [],
	};

	for (let i = 0; i < args.length; i++) {
		const arg = args[i];

		if (arg === "--help" || arg === "-h") {
			result.help = true;
		} else if (arg === "--version" || arg === "-v") {
			result.version = true;
		} else if (arg === "--mode" && i + 1 < args.length) {
			const mode = args[++i];
			if (mode === "text" || mode === "json" || mode === "rpc") {
				result.mode = mode;
			}
		} else if (arg === "--continue" || arg === "-c") {
			result.continue = true;
		} else if (arg === "--resume" || arg === "-r") {
			result.resume = true;
		} else if (arg === "--provider" && i + 1 < args.length) {
			result.provider = args[++i];
		} else if (arg === "--model" && i + 1 < args.length) {
			result.model = args[++i];
		} else if (arg === "--api-key" && i + 1 < args.length) {
			result.apiKey = args[++i];
		} else if (arg === "--system-prompt" && i + 1 < args.length) {
			result.systemPrompt = args[++i];
		} else if (arg === "--no-session") {
			result.noSession = true;
		} else if (arg === "--task-id" && i + 1 < args.length) {
			result.taskId = args[++i];
		} else if (arg === "--max-turns" && i + 1 < args.length) {
			const n = Number.parseInt(args[++i], 10);
			if (Number.isInteger(n) && n > 0) {
				result.maxTurns = n;
			}
		} else if (arg === "--session" && i + 1 < args.length) {
			result.session = args[++i];
		} else if (arg === "--team" && i + 1 < args.length) {
			result.team = args[++i];
		} else if (arg === "--fork" && i + 1 < args.length) {
			result.fork = args[++i];
		} else if (arg === "--session-dir" && i + 1 < args.length) {
			result.sessionDir = args[++i];
		} else if (arg === "--models" && i + 1 < args.length) {
			result.models = args[++i].split(",").map((s) => s.trim());
		} else if (arg === "--no-tools" || arg === "-nt") {
			result.noTools = true;
		} else if (arg === "--no-builtin-tools" || arg === "-nbt") {
			result.noBuiltinTools = true;
		} else if (arg === "--enable-subagents") {
			result.subagent = true;
		} else if (arg === "--no-subagents" || arg === "--disable-subagents") {
			result.subagent = false;
		} else if (arg === "--warm-subagents") {
			result.warmSubagents = true;
		} else if (arg === "--max-subagent-depth" && i + 1 < args.length) {
			const n = Number.parseInt(args[++i], 10);
			if (Number.isInteger(n) && n >= 1) {
				result.maxSubagentDepth = n;
			}
		} else if (arg === "--delegate-allow" && i + 1 < args.length) {
			result.delegateAllow = args[++i]
				.split(",")
				.map((s) => s.trim())
				.filter((name) => name.length > 0);
		} else if (arg === "--enable-todowrite") {
			result.todoWrite = true;
		} else if (arg === "--enable-webtools") {
			result.enableWebTools = true;
		} else if (arg === "--enable-browsertools") {
			result.enableBrowserTools = true;
		} else if (arg === "--enable-browser-live-preview") {
			result.enableBrowserLivePreview = true;
		} else if (arg === "--enable-filetools") {
			result.enableFileTools = true;
		} else if (arg === "--enable-plugintools") {
			result.enablePluginTools = true;
		} else if (arg === "--light") {
			result.light = true;
		} else if (arg === "--print-token-surface") {
			result.printTokenSurface = true;
		} else if (arg === "--support-platform" && i + 1 < args.length) {
			result.supportPlatform = [
				...(result.supportPlatform ?? []),
				...args[++i]
					.split(",")
					.map((s) => s.trim())
					.filter(Boolean),
			];
		} else if (arg === "--ca-cert" && i + 1 < args.length) {
			result.caCert = args[++i];
		} else if (arg === "--use-system-ca") {
			result.useSystemCa = true;
		} else if ((arg === "--tools" || arg === "-t") && i + 1 < args.length) {
			result.tools = args[++i]
				.split(",")
				.map((s) => s.trim())
				.filter((name) => name.length > 0);
		} else if (arg === "--disallowed-tools" && i + 1 < args.length) {
			result.disallowedTools = args[++i]
				.split(",")
				.map((s) => s.trim())
				.filter((name) => name.length > 0);
		} else if (arg === "--thinking" && i + 1 < args.length) {
			const level = args[++i];
			if (isValidThinkingLevel(level)) {
				result.thinking = level;
			} else {
				result.diagnostics.push({
					type: "warning",
					message: `Invalid thinking level "${level}". Valid values: ${VALID_THINKING_LEVELS.join(", ")}`,
				});
			}
		} else if (arg === "--print" || arg === "-p") {
			result.print = true;
			const next = args[i + 1];
			if (next !== undefined && !next.startsWith("@") && (!next.startsWith("-") || next.startsWith("---"))) {
				result.messages.push(next);
				i++;
			}
		} else if (arg === "--export" && i + 1 < args.length) {
			result.export = args[++i];
		} else if ((arg === "--extension" || arg === "-e") && i + 1 < args.length) {
			result.extensions = result.extensions ?? [];
			result.extensions.push(args[++i]);
		} else if (arg === "--no-extensions" || arg === "-ne") {
			result.noExtensions = true;
		} else if (arg === "--skill" && i + 1 < args.length) {
			result.skills = result.skills ?? [];
			result.skills.push(args[++i]);
		} else if (arg === "--agent" && i + 1 < args.length) {
			result.agents = result.agents ?? [];
			result.agents.push(args[++i]);
		} else if (arg === "--prompt-template" && i + 1 < args.length) {
			result.promptTemplates = result.promptTemplates ?? [];
			result.promptTemplates.push(args[++i]);
		} else if (arg === "--slash-command" && i + 1 < args.length) {
			result.slashCommands = result.slashCommands ?? [];
			result.slashCommands.push(args[++i]);
		} else if (arg === "--theme" && i + 1 < args.length) {
			result.themes = result.themes ?? [];
			result.themes.push(args[++i]);
		} else if (arg === "--mode-path" && i + 1 < args.length) {
			result.modePaths = result.modePaths ?? [];
			result.modePaths.push(args[++i]);
		} else if (arg === "--no-skills" || arg === "-ns") {
			result.noSkills = true;
		} else if (arg === "--no-prompt-templates" || arg === "-np") {
			result.noPromptTemplates = true;
		} else if (arg === "--no-slash-commands" || arg === "-nsc") {
			result.noSlashCommands = true;
		} else if (arg === "--no-themes") {
			result.noThemes = true;
		} else if (arg === "--no-context-files" || arg === "-nc") {
			result.noContextFiles = true;
		} else if (arg === "--list-models") {
			// Check if next arg is a search pattern (not a flag or file arg)
			if (i + 1 < args.length && !args[i + 1].startsWith("-") && !args[i + 1].startsWith("@")) {
				result.listModels = args[++i];
			} else {
				result.listModels = true;
			}
		} else if (arg === "--verbose") {
			result.verbose = true;
		} else if (arg === "--offline") {
			result.offline = true;
		} else if (arg.startsWith("@")) {
			result.fileArgs.push(arg.slice(1)); // Remove @ prefix
		} else if (arg.startsWith("--")) {
			const eqIndex = arg.indexOf("=");
			if (eqIndex !== -1) {
				result.unknownFlags.set(arg.slice(2, eqIndex), arg.slice(eqIndex + 1));
			} else {
				const flagName = arg.slice(2);
				const next = args[i + 1];
				if (next !== undefined && !next.startsWith("-") && !next.startsWith("@")) {
					result.unknownFlags.set(flagName, next);
					i++;
				} else {
					result.unknownFlags.set(flagName, true);
				}
			}
		} else if (arg.startsWith("-") && !arg.startsWith("--")) {
			result.diagnostics.push({ type: "error", message: `Unknown option: ${arg}` });
		} else if (!arg.startsWith("-")) {
			result.messages.push(arg);
		}
	}

	return result;
}

export function printHelp(extensionFlags?: ExtensionFlag[]): void {
	const extensionFlagsText =
		extensionFlags && extensionFlags.length > 0
			? `\n${chalk.bold("Extension CLI Flags:")}\n${extensionFlags
					.map((flag) => {
						const value = flag.type === "string" ? " <value>" : "";
						const description = flag.description ?? `Registered by ${flag.extensionPath}`;
						return `  --${flag.name}${value}`.padEnd(30) + description;
					})
					.join("\n")}\n`
			: "";
	console.log(`${chalk.bold(APP_NAME)} - AI coding assistant with read, bash, edit, write tools

${chalk.bold("Usage:")}
  ${APP_NAME} [options] [@files...] [messages...]

${chalk.bold("Commands:")}
  ${APP_NAME} install <source> [-l]     Install extension source and add to settings
  ${APP_NAME} remove <source> [-l]      Remove extension source from settings
  ${APP_NAME} uninstall <source> [-l]   Alias for remove
  ${APP_NAME} update [source|self|pi]   Update hoocode and installed extensions
  ${APP_NAME} list                      List installed extensions from settings
  ${APP_NAME} config                    Open TUI to enable/disable package resources
  ${APP_NAME} resources                 List discovered skills, subagents, slash commands, and MCP servers
  ${APP_NAME} a2a [--serve]             Publish an A2A AgentCard of active tools/skills for agent discovery
  ${APP_NAME} <command> --help          Show help for install/remove/uninstall/update/list

${chalk.bold("Options:")}
  --provider <name>              Provider name (default: google)
  --model <pattern>              Model pattern or ID (supports "provider/id" and optional ":<thinking>")
  --api-key <key>                API key (defaults to env vars)
  --system-prompt <text>         System prompt (default: coding assistant prompt)
  --mode <mode>                  Output mode: text (default), json, or rpc
  --print, -p                    Non-interactive mode: process prompt and exit
  --continue, -c                 Continue previous session
  --resume, -r                   Select a session to resume
  --session <path|id>            Use specific session file or partial UUID
  --fork <path|id>               Fork specific session file or partial UUID into a new session
  --session-dir <dir>            Directory for session storage and lookup
  --no-session                   Don't save session (ephemeral)
  --team <url|auto>              Bridge a hooteams server into the task panel's teams view
                                 (focus roles, nudge, attach, answer approval gates);
                                 "auto" finds .agents/teams/default.json
                                 or hooteams.config.json upward from cwd and spawns hooteams locally
  --models <patterns>            Comma-separated model patterns for Ctrl+P cycling
                                 Supports globs (anthropic/*, *sonnet*) and fuzzy matching
  --no-tools, -nt                Disable all tools by default (built-in and extension)
  --no-builtin-tools, -nbt       Disable built-in tools by default but keep extension/custom tools enabled
  --tools, -t <tools>            Comma-separated allowlist of tool names to enable
                                 Applies to built-in, extension, and custom tools
  --disallowed-tools <tools>     Comma-separated denylist of tool names to disable
                                 (subtracted from the allowlist or default set)
  --max-turns <n>                Hard cap on assistant turns; the agent is asked to wrap up near the
                                 cap and stopped at it (mainly used for spawned subagents)
  --enable-subagents             Enable the subagent tool (delegate tasks to isolated agent loops).
                                 On by default (the "enableSubagent" setting defaults to true)
  --no-subagents                 Disable the subagent tool for this session (overrides the setting)
  --warm-subagents               Dispatch eligible subagents on reused warm RPC workers (experimental)
  --max-subagent-depth <n>       Tree-wide subagent nesting cap (default 2 = one level of nesting;
                                 1 = no nesting). Overrides the "maxSubagentDepth" setting
                                 Can also be enabled via the "enableSubagent" setting
  --enable-todowrite             Enable the TodoWrite tool (maintain a live todo list in the task panel)
                                  Can also be enabled via the "enableTodoWrite" setting
  --enable-webtools              Enable the webfetch + websearch tools (network access, off by default)
                                  Can also be enabled via the "enableWebTools" setting
                                  Block hosts with a .webtoolsignore file (gitignore syntax)
  --enable-browsertools          Enable the browser_run + browser_continue tools (off by default)
                                  Deterministic browser automation via the browsertools engine,
                                  pausing for LLM decisions (NeedsParent) mid-flow
                                  Can also be enabled via the "enableBrowserTools" setting
  --enable-browser-live-preview  Default the live viewer on for browser_run runs and auto-open it
                                  Streams the page + the agent's tool-call log over a local WebSocket
                                  Set HOOCODE_BROWSERTOOLS_NO_OPEN=1 to print the URL without opening
                                  Can also be enabled via the "enableBrowserLivePreview" setting
  --enable-filetools             Enable the document tools (off by default)
                                  DocRead/DocEdit/DocWrite (extract + lossless id-based edit) and
                                  DocScan/DocGrep/DocPeek (cheap outline/search/partial read) for
                                  XML, drawio, docx/xlsx/pptx, PDF via the filetools binary
                                  Can also be enabled via the "enableFileTools" setting
  --enable-plugintools           Enable the autonomous plugin system (off by default)
                                  Plugin lifecycle tools (SearchPlugins, InstallPlugin, ...) and
                                  ProposePlugin, plus the runtime plugin-reuse nudge
                                  Can also be enabled via the "enablePluginTools" setting
  --light                        Minimal low-token preset for small/local models:
                                  only the read/write/edit/bash tools (short descriptions,
                                  stripped schemas; search via bash), a terse system prompt, and
                                  no subagents/TodoWrite/skills/context files/mode appendix
                                  Can also be enabled via the "light" setting
  --print-token-surface          Print the fixed per-turn surface (system prompt + serialized
                                  tool schema token estimate) and exit
  --support-platform <list>      Platform layout(s) hoocode targets when it writes artifacts:
                                  authored plugins (ProposePlugin) and the /new-skill /new-agent
                                  /new-command scaffolds. Comma-separated and/or repeated.
                                  Tokens: claude, copilot (alias: github, gh), agents (alias: native)
                                  e.g. --support-platform copilot writes .github/skills/<name>/SKILL.md,
                                  .github/agents/<name>.agent.md, .github/prompts/<name>.prompt.md and
                                  a .github/plugin/plugin.json manifest for authored plugins
                                  Default: claude + copilot for authored plugins; .hoocode/ for scaffolds
                                  Can also be set via the "supportPlatform" setting
  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh
  --extension, -e <path>         Load an extension file (can be used multiple times)
  --no-extensions, -ne           Disable extension discovery (explicit -e paths still work)
  --skill <path>                 Load a skill file or directory (can be used multiple times)
  --no-skills, -ns               Disable skills discovery and loading
  --agent <path>                 Load an agent file or directory (can be used multiple times)
  --prompt-template <path>       Load a prompt template / slash command file or directory (repeatable)
  --slash-command <path>         Alias of --prompt-template (prompts and slash commands are one feature)
  --no-prompt-templates, -np     Disable prompt template / slash command discovery (same as --no-slash-commands)
  --no-slash-commands, -nsc      Disable prompt template / slash command discovery (same as --no-prompt-templates)
  --theme <path>                 Load a theme file or directory (can be used multiple times)
  --no-themes                    Disable theme discovery and loading
  --mode-path <dir>              Add a directory to search for {name}/system.md mode files (can be used multiple times)
  --no-context-files, -nc        Disable AGENTS.md and CLAUDE.md discovery and loading
  --export <file>                Export session file to HTML and exit
  --list-models [search]         List available models (with optional fuzzy search)
  --verbose                      Force verbose startup (overrides quietStartup setting)
  --offline                      Disable startup network operations (same as HOOCODE_OFFLINE=1)
  --ca-cert <path>               Trust an extra PEM CA bundle for hoocode's own TLS traffic (provider
                                 calls, GitHub API, downloads). Additive to the bundled roots;
                                 verification stays ON. Also: HOOCODE_CA_CERT / NODE_EXTRA_CA_CERTS.
                                 Does not affect the webfetch/websearch binary.
  --use-system-ca                Also trust the OS/system CA store (opt-in). Same as
                                 HOOCODE_USE_SYSTEM_CA=1
  --help, -h                     Show this help
  --version, -v                  Show version number

Extensions can register additional flags (e.g., --plan from plan-mode extension).${extensionFlagsText}

${chalk.bold("Examples:")}
  # Interactive mode
  ${APP_NAME}

  # Interactive mode with initial prompt
  ${APP_NAME} "List all .ts files in src/"

  # Include files in initial message
  ${APP_NAME} @prompt.md @image.png "What color is the sky?"

  # Non-interactive mode (process and exit)
  ${APP_NAME} -p "List all .ts files in src/"

  # Multiple messages (interactive)
  ${APP_NAME} "Read package.json" "What dependencies do we have?"

  # Continue previous session
  ${APP_NAME} --continue "What did we discuss?"

  # Use different model
  ${APP_NAME} --provider openai --model gpt-4o-mini "Help me refactor this code"

  # Use model with provider prefix (no --provider needed)
  ${APP_NAME} --model openai/gpt-4o "Help me refactor this code"

  # Use model with thinking level shorthand
  ${APP_NAME} --model sonnet:high "Solve this complex problem"

  # Limit model cycling to specific models
  ${APP_NAME} --models claude-sonnet,claude-haiku,gpt-4o

  # Limit to a specific provider with glob pattern
  ${APP_NAME} --models "github-copilot/*"

  # Cycle models with fixed thinking levels
  ${APP_NAME} --models sonnet:high,haiku:low

  # Start with a specific thinking level
  ${APP_NAME} --thinking high "Solve this complex problem"

  # Read-only mode (no file modifications possible)
  ${APP_NAME} --tools read,grep,find,ls -p "Review the code in src/"

  # Export a session file to HTML
  ${APP_NAME} --export ~/${CONFIG_DIR_NAME}/agent/sessions/--path--/session.jsonl
  ${APP_NAME} --export session.jsonl output.html

${chalk.bold("Environment Variables:")}
  ANTHROPIC_API_KEY                - Anthropic Claude API key
  ANTHROPIC_OAUTH_TOKEN            - Anthropic OAuth token (alternative to API key)
  OPENAI_API_KEY                   - OpenAI GPT API key
  AZURE_OPENAI_API_KEY             - Azure OpenAI API key
  AZURE_OPENAI_BASE_URL            - Azure OpenAI/Cognitive Services base URL (e.g. https://{resource}.openai.azure.com)
  AZURE_OPENAI_RESOURCE_NAME       - Azure OpenAI resource name (alternative to base URL)
  AZURE_OPENAI_API_VERSION         - Azure OpenAI API version (default: v1)
  AZURE_OPENAI_DEPLOYMENT_NAME_MAP - Azure OpenAI model=deployment map (comma-separated)
  DEEPSEEK_API_KEY                 - DeepSeek API key
  GEMINI_API_KEY                   - Google Gemini API key
  GROQ_API_KEY                     - Groq API key
  CEREBRAS_API_KEY                 - Cerebras API key
  XAI_API_KEY                      - xAI Grok API key
  FIREWORKS_API_KEY                - Fireworks API key
  TOGETHER_API_KEY                 - Together AI API key
  OPENROUTER_API_KEY               - OpenRouter API key
  AI_GATEWAY_API_KEY               - Vercel AI Gateway API key
  ZAI_API_KEY                      - ZAI API key
  MISTRAL_API_KEY                  - Mistral API key
  MINIMAX_API_KEY                  - MiniMax API key
  MOONSHOT_API_KEY                 - Moonshot AI API key
  OPENCODE_API_KEY                 - OpenCode Zen/OpenCode Go API key
  KIMI_API_KEY                     - Kimi For Coding API key
  CLOUDFLARE_API_KEY               - Cloudflare API token (Workers AI and AI Gateway)
  CLOUDFLARE_ACCOUNT_ID            - Cloudflare account id (required for both)
  CLOUDFLARE_GATEWAY_ID            - Cloudflare AI Gateway slug (required for AI Gateway)
  XIAOMI_API_KEY                   - Xiaomi MiMo API key (api.xiaomimimo.com billing)
  XIAOMI_TOKEN_PLAN_CN_API_KEY     - Xiaomi MiMo Token Plan API key (China region)
  XIAOMI_TOKEN_PLAN_AMS_API_KEY    - Xiaomi MiMo Token Plan API key (Amsterdam region)
  XIAOMI_TOKEN_PLAN_SGP_API_KEY    - Xiaomi MiMo Token Plan API key (Singapore region)
  NVIDIA_API_KEY                   - NVIDIA API key (integrate.api.nvidia.com)
  AWS_PROFILE                      - AWS profile for Amazon Bedrock
  AWS_ACCESS_KEY_ID                - AWS access key for Amazon Bedrock
  AWS_SECRET_ACCESS_KEY            - AWS secret key for Amazon Bedrock
  AWS_BEARER_TOKEN_BEDROCK         - Bedrock API key (bearer token)
  AWS_REGION                       - AWS region for Amazon Bedrock (e.g., us-east-1)
  ${ENV_AGENT_DIR.padEnd(32)} - Config directory (default: ~/${CONFIG_DIR_NAME}/agent)
  ${ENV_SESSION_DIR.padEnd(32)} - Session storage directory (overridden by --session-dir)
  HOOCODE_PACKAGE_DIR              - Override package directory (for Nix/Guix store paths).
  HOOCODE_OFFLINE                  - Disable startup network operations when set to 1/true/yes.
  HOOCODE_NATIVE_SEARCH            - Force the pure-JS find/grep fallback instead of the fd/rg binaries
                                     when set to 1 (also used automatically when fd/rg are unavailable).
  HOOCODE_CA_CERT                  - Path to an extra PEM CA bundle to trust for hoocode's own TLS traffic
                                     (additive; verification stays on). Precedence: --ca-cert > this > NODE_EXTRA_CA_CERTS
  HOOCODE_USE_SYSTEM_CA            - Also trust the OS/system CA store when set to 1/true/yes (same as --use-system-ca)
  NODE_EXTRA_CA_CERTS              - Standard Node path to an extra PEM CA bundle (used as the lowest-precedence CA source)
  HOOCODE_WEBTOOLS_CA_CERT         - Path to a PEM CA bundle forwarded to the webtools binary (webfetch/websearch) as --ca-cert
  HOOCODE_WEBTOOLS_INSECURE        - Forward --insecure to the webtools binary (disables its TLS verification) when set to 1/true/yes
  HOOCODE_TELEMETRY                - Override install telemetry when set to 1/true/yes or 0/false/no.
  HOOCODE_SHARE_VIEWER_URL         - Base URL for /share command (default: https://pi.dev/session/).

${chalk.bold("Built-in Tool Names:")}
  read   - Read file contents
  bash   - Execute bash commands
  edit   - Edit files with find/replace
  write  - Write files (creates/overwrites)
  grep   - Search file contents (read-only, off by default)
  find   - Find files by glob pattern (read-only, off by default)
  ls     - List directory contents (read-only, off by default)
`);
}
