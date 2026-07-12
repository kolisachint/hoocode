/**
 * Startup/reload resource listing: path and source-label formatting, scope
 * grouping, diagnostics rendering, and the [Resources]/[Skills]/… sections
 * shown in the chat. Extracted from interactive-mode.ts; everything here is
 * either pure formatting or rendering into a container passed via deps.
 */

import * as os from "node:os";
import * as path from "node:path";
import { type Container, Spacer, Text, visibleWidth } from "@kolisachint/hoocode-tui";
import { getExtensionMcpServers } from "../../core/extension-mcp-servers.js";
import type { ExtensionRunner } from "../../core/extensions/index.js";
import type { PromptTemplate } from "../../core/prompt-templates.js";
import type { ResourceDiagnostic, ResourceLoader } from "../../core/resource-loader.js";
import type { SourceInfo } from "../../core/source-info.js";
import { parseGitUrl } from "../../utils/git.js";
import { getCwdRelativePath } from "../../utils/paths.js";
import { CATEGORY_GLYPH, type CategoryKey } from "./brand.js";
import { appKeyLabel } from "./components/keybinding-hints.js";
import { type ThemeColor, theme } from "./theme/theme.js";

// ============================================================================
// Expandable sections
// ============================================================================

export interface Expandable {
	setExpanded(expanded: boolean): void;
}

export function isExpandable(obj: unknown): obj is Expandable {
	return typeof obj === "object" && obj !== null && "setExpanded" in obj;
}

export class ExpandableText extends Text implements Expandable {
	constructor(
		private collapsed: () => string,
		private expanded: () => string,
		initiallyExpanded: boolean,
		paddingX: number,
		paddingY: number,
	) {
		super(initiallyExpanded ? expanded() : collapsed(), paddingX, paddingY);
	}

	setExpanded(expanded: boolean): void {
		this.setText(expanded ? this.expanded() : this.collapsed());
	}
}

// ============================================================================
// Path and source-label formatting
// ============================================================================

export function formatDisplayPath(p: string): string {
	const home = os.homedir();
	let result = p;

	// Replace home directory with ~
	if (result.startsWith(home)) {
		result = `~${result.slice(home.length)}`;
	}

	return result;
}

export function formatExtensionDisplayPath(extensionPath: string): string {
	let result = formatDisplayPath(extensionPath);
	result = result.replace(/\/index\.ts$/, "").replace(/\/index\.js$/, "");
	return result;
}

export function formatContextPath(p: string, cwd: string): string {
	const resolvedCwd = path.resolve(cwd);
	const absolutePath = path.isAbsolute(p) ? path.resolve(p) : path.resolve(resolvedCwd, p);
	const relativePath = getCwdRelativePath(absolutePath, resolvedCwd);
	if (relativePath !== undefined) {
		return relativePath;
	}

	return formatDisplayPath(absolutePath);
}

/**
 * Get a short path relative to the package root for display.
 */
export function getShortPath(fullPath: string, sourceInfo?: SourceInfo): string {
	const baseDir = sourceInfo?.baseDir;
	if (baseDir && isPackageSource(sourceInfo)) {
		const relativePath = path.relative(path.resolve(baseDir), path.resolve(fullPath));
		if (
			relativePath &&
			relativePath !== "." &&
			!relativePath.startsWith("..") &&
			!relativePath.startsWith(`..${path.sep}`) &&
			!path.isAbsolute(relativePath)
		) {
			return relativePath.replace(/\\/g, "/");
		}
	}

	const source = sourceInfo?.source ?? "";
	const npmMatch = fullPath.match(/node_modules\/(@?[^/]+(?:\/[^/]+)?)\/(.*)/);
	if (npmMatch && source.startsWith("npm:")) {
		return npmMatch[2];
	}

	const gitMatch = fullPath.match(/git\/[^/]+\/[^/]+\/(.*)/);
	if (gitMatch && source.startsWith("git:")) {
		return gitMatch[1];
	}

	return formatDisplayPath(fullPath);
}

export function getCompactPathLabel(resourcePath: string, sourceInfo?: SourceInfo): string {
	const shortPath = getShortPath(resourcePath, sourceInfo);
	const normalizedPath = shortPath.replace(/\\/g, "/");
	const segments = normalizedPath.split("/").filter((segment) => segment.length > 0 && segment !== "~");
	if (segments.length > 0) {
		return segments[segments.length - 1]!;
	}
	return shortPath;
}

function getCompactPackageSourceLabel(sourceInfo?: SourceInfo): string {
	const source = sourceInfo?.source ?? "";
	if (source.startsWith("npm:")) {
		return source.slice("npm:".length) || source;
	}

	const gitSource = parseGitUrl(source);
	if (gitSource) {
		return gitSource.path || source;
	}

	return source;
}

function getCompactExtensionLabel(resourcePath: string, sourceInfo?: SourceInfo): string {
	if (!isPackageSource(sourceInfo)) {
		return getCompactPathLabel(resourcePath, sourceInfo);
	}

	const sourceLabel = getCompactPackageSourceLabel(sourceInfo);
	if (!sourceLabel) {
		return getCompactPathLabel(resourcePath, sourceInfo);
	}

	const shortPath = getShortPath(resourcePath, sourceInfo).replace(/\\/g, "/");
	const packagePath = shortPath.startsWith("extensions/") ? shortPath.slice("extensions/".length) : shortPath;
	const parsedPath = path.posix.parse(packagePath);

	if (parsedPath.name === "index") {
		return !parsedPath.dir || parsedPath.dir === "." ? sourceLabel : `${sourceLabel}:${parsedPath.dir}`;
	}

	return `${sourceLabel}:${packagePath}`;
}

function getCompactDisplayPathSegments(resourcePath: string): string[] {
	return formatDisplayPath(resourcePath)
		.replace(/\\/g, "/")
		.split("/")
		.filter((segment) => segment.length > 0 && segment !== "~");
}

function getCompactNonPackageExtensionLabel(
	resourcePath: string,
	index: number,
	allPaths: Array<{ path: string; segments: string[] }>,
): string {
	const segments = allPaths[index]?.segments;
	if (!segments || segments.length === 0) {
		return getCompactPathLabel(resourcePath);
	}

	for (let segmentCount = 1; segmentCount <= segments.length; segmentCount += 1) {
		const candidate = segments.slice(-segmentCount).join("/");
		const isUnique = allPaths.every((item, itemIndex) => {
			if (itemIndex === index) {
				return true;
			}
			return item.segments.slice(-segmentCount).join("/") !== candidate;
		});

		if (isUnique) {
			return candidate;
		}
	}

	return segments.join("/");
}

export function getCompactExtensionLabels(
	extensions: Array<{ path: string; sourceInfo?: SourceInfo; displayName?: string }>,
): string[] {
	const nonPackageExtensions = extensions
		.map((extension) => {
			const segments = getCompactDisplayPathSegments(extension.path);
			const lastSegment = segments[segments.length - 1];
			if (segments.length > 1 && (lastSegment === "index.ts" || lastSegment === "index.js")) {
				segments.pop();
			}
			return {
				path: extension.path,
				sourceInfo: extension.sourceInfo,
				segments,
			};
		})
		.filter((extension) => !isPackageSource(extension.sourceInfo));

	return extensions.map((extension) => {
		if (extension.displayName) {
			return extension.displayName;
		}

		if (isPackageSource(extension.sourceInfo)) {
			return getCompactExtensionLabel(extension.path, extension.sourceInfo);
		}

		const nonPackageIndex = nonPackageExtensions.findIndex((item) => item.path === extension.path);
		if (nonPackageIndex === -1) {
			return getCompactPathLabel(extension.path, extension.sourceInfo);
		}

		return getCompactNonPackageExtensionLabel(extension.path, nonPackageIndex, nonPackageExtensions);
	});
}

function getDisplaySourceInfo(sourceInfo?: SourceInfo): {
	label: string;
	scopeLabel?: string;
	color: "accent" | "muted";
} {
	const source = sourceInfo?.source ?? "local";
	const scope = sourceInfo?.scope ?? "project";
	if (source === "local") {
		if (scope === "user") {
			return { label: "user", color: "muted" };
		}
		if (scope === "project") {
			return { label: "project", color: "muted" };
		}
		if (scope === "temporary") {
			return { label: "path", scopeLabel: "temp", color: "muted" };
		}
		return { label: "path", color: "muted" };
	}

	if (source === "cli") {
		return { label: "path", scopeLabel: scope === "temporary" ? "temp" : undefined, color: "muted" };
	}

	const scopeLabel =
		scope === "user" ? "user" : scope === "project" ? "project" : scope === "temporary" ? "temp" : undefined;
	return { label: source, scopeLabel, color: "accent" };
}

function getScopeGroup(sourceInfo?: SourceInfo): "user" | "project" | "path" {
	const source = sourceInfo?.source ?? "local";
	const scope = sourceInfo?.scope ?? "project";
	if (source === "cli" || scope === "temporary") return "path";
	if (scope === "user") return "user";
	if (scope === "project") return "project";
	return "path";
}

function isPackageSource(sourceInfo?: SourceInfo): boolean {
	const source = sourceInfo?.source ?? "";
	return source.startsWith("npm:") || source.startsWith("git:");
}

// ============================================================================
// Scope grouping and diagnostics
// ============================================================================

interface ScopedItem {
	path: string;
	sourceInfo?: SourceInfo;
	displayName?: string;
}

interface ScopeGroup {
	scope: "user" | "project" | "path";
	paths: ScopedItem[];
	packages: Map<string, ScopedItem[]>;
}

function buildScopeGroups(items: ScopedItem[]): ScopeGroup[] {
	const groups: Record<"user" | "project" | "path", ScopeGroup> = {
		user: { scope: "user", paths: [], packages: new Map() },
		project: { scope: "project", paths: [], packages: new Map() },
		path: { scope: "path", paths: [], packages: new Map() },
	};

	for (const item of items) {
		const groupKey = getScopeGroup(item.sourceInfo);
		const group = groups[groupKey];
		const source = item.sourceInfo?.source ?? "local";

		if (isPackageSource(item.sourceInfo)) {
			const list = group.packages.get(source) ?? [];
			list.push(item);
			group.packages.set(source, list);
		} else {
			group.paths.push(item);
		}
	}

	return [groups.project, groups.user, groups.path].filter(
		(group) => group.paths.length > 0 || group.packages.size > 0,
	);
}

function formatScopeGroups(
	groups: ScopeGroup[],
	options: {
		formatPath: (item: ScopedItem) => string;
		formatPackagePath: (item: ScopedItem, source: string) => string;
	},
): string {
	const lines: string[] = [];

	for (const group of groups) {
		lines.push(`  ${theme.fg("accent", group.scope)}`);

		const sortedPaths = [...group.paths].sort((a, b) => a.path.localeCompare(b.path));
		for (const item of sortedPaths) {
			lines.push(theme.fg("dim", `    ${options.formatPath(item)}`));
		}

		const sortedPackages = Array.from(group.packages.entries()).sort(([a], [b]) => a.localeCompare(b));
		for (const [source, items] of sortedPackages) {
			lines.push(`    ${theme.fg("mdLink", source)}`);
			const sortedPackagePaths = [...items].sort((a, b) => a.path.localeCompare(b.path));
			for (const item of sortedPackagePaths) {
				lines.push(theme.fg("dim", `      ${options.formatPackagePath(item, source)}`));
			}
		}
	}

	return lines.join("\n");
}

function findSourceInfoForPath(p: string, sourceInfos: Map<string, SourceInfo>): SourceInfo | undefined {
	const exact = sourceInfos.get(p);
	if (exact) return exact;

	let current = p;
	while (current.includes("/")) {
		current = current.substring(0, current.lastIndexOf("/"));
		const parent = sourceInfos.get(current);
		if (parent) return parent;
	}

	return undefined;
}

function formatPathWithSource(p: string, sourceInfo?: SourceInfo): string {
	if (sourceInfo) {
		const shortPath = getShortPath(p, sourceInfo);
		const { label, scopeLabel } = getDisplaySourceInfo(sourceInfo);
		const labelText = scopeLabel ? `${label} (${scopeLabel})` : label;
		return `${labelText} ${shortPath}`;
	}
	return formatDisplayPath(p);
}

export function formatDiagnostics(
	diagnostics: readonly ResourceDiagnostic[],
	sourceInfos: Map<string, SourceInfo>,
): string {
	const lines: string[] = [];

	// Group collision diagnostics by name
	const collisions = new Map<string, ResourceDiagnostic[]>();
	const otherDiagnostics: ResourceDiagnostic[] = [];

	for (const d of diagnostics) {
		if (d.type === "collision" && d.collision) {
			const list = collisions.get(d.collision.name) ?? [];
			list.push(d);
			collisions.set(d.collision.name, list);
		} else {
			otherDiagnostics.push(d);
		}
	}

	// Format collision diagnostics grouped by name
	for (const [name, collisionList] of collisions) {
		const first = collisionList[0]?.collision;
		if (!first) continue;
		lines.push(theme.fg("warning", `  "${name}" collision:`));
		lines.push(
			theme.fg(
				"dim",
				`    ${theme.fg("success", "✓")} ${formatPathWithSource(first.winnerPath, findSourceInfoForPath(first.winnerPath, sourceInfos))}`,
			),
		);
		for (const d of collisionList) {
			if (d.collision) {
				lines.push(
					theme.fg(
						"dim",
						`    ${theme.fg("warning", "✗")} ${formatPathWithSource(d.collision.loserPath, findSourceInfoForPath(d.collision.loserPath, sourceInfos))} (skipped)`,
					),
				);
			}
		}
	}

	for (const d of otherDiagnostics) {
		if (d.path) {
			const formattedPath = formatPathWithSource(d.path, findSourceInfoForPath(d.path, sourceInfos));
			lines.push(theme.fg(d.type === "error" ? "error" : "warning", `  ${formattedPath}`));
			lines.push(theme.fg(d.type === "error" ? "error" : "warning", `    ${d.message}`));
		} else {
			lines.push(theme.fg(d.type === "error" ? "error" : "warning", `  ${d.message}`));
		}
	}

	return lines.join("\n");
}

// ============================================================================
// Loaded-resources listing
// ============================================================================

/** The slice of the interactive mode the resource listing renders into. */
export interface ResourceDisplayDeps {
	chatContainer: Container;
	getCwd(): string;
	getResourceLoader(): ResourceLoader;
	getPromptTemplates(): ReadonlyArray<PromptTemplate>;
	getExtensionRunner(): ExtensionRunner;
	getActiveMode(): string;
	getSubagentEnabled(): boolean;
	quietStartup(): boolean;
	verbose: boolean;
	/** Startup expansion state for the collapsible sections. */
	isExpanded(): boolean;
	getBuiltInCommandConflictDiagnostics(extensionRunner: ExtensionRunner): ResourceDiagnostic[];
}

export function showLoadedResources(
	deps: ResourceDisplayDeps,
	options?: {
		extensions?: Array<{ path: string; sourceInfo?: SourceInfo }>;
		force?: boolean;
		showDiagnosticsWhenQuiet?: boolean;
	},
): void {
	const showListing = options?.force || deps.verbose || !deps.quietStartup();
	const showDiagnostics = showListing || options?.showDiagnosticsWhenQuiet === true;
	if (!showListing && !showDiagnostics) {
		return;
	}

	const resourceLoader = deps.getResourceLoader();
	const chatContainer = deps.chatContainer;

	const sectionHeader = (name: string, color: ThemeColor = "mdHeading") => theme.fg(color, `[${name}]`);
	const formatCompactList = (items: string[], listOptions?: { sort?: boolean }): string => {
		const labels = items.map((item) => item.trim()).filter((item) => item.length > 0);
		if (listOptions?.sort !== false) {
			labels.sort((a, b) => a.localeCompare(b));
		}
		return theme.fg("dim", `  ${labels.join(", ")}`);
	};

	const skillsResult = resourceLoader.getSkills();
	const promptsResult = resourceLoader.getPrompts();
	const themesResult = resourceLoader.getThemes();
	const extensions =
		options?.extensions ??
		resourceLoader
			.getExtensions()
			.extensions.filter((extension) => !extension.internal)
			.map((extension) => ({
				path: extension.path,
				sourceInfo: extension.sourceInfo,
				displayName: extension.displayName,
			}));
	const sourceInfos = new Map<string, SourceInfo>();
	for (const extension of extensions) {
		if (extension.sourceInfo) {
			sourceInfos.set(extension.path, extension.sourceInfo);
		}
	}
	for (const skill of skillsResult.skills) {
		if (skill.sourceInfo) {
			sourceInfos.set(skill.filePath, skill.sourceInfo);
		}
	}
	for (const prompt of promptsResult.prompts) {
		if (prompt.sourceInfo) {
			sourceInfos.set(prompt.filePath, prompt.sourceInfo);
		}
	}
	for (const loadedTheme of themesResult.themes) {
		if (loadedTheme.sourcePath && loadedTheme.sourceInfo) {
			sourceInfos.set(loadedTheme.sourcePath, loadedTheme.sourceInfo);
		}
	}

	if (showListing) {
		chatContainer.addChild(new Spacer(1));

		const { agentsFiles: contextFiles, warnings: contextWarnings } = resourceLoader.getAgentsFiles();
		const skills = skillsResult.skills;
		const templates = deps.getPromptTemplates();
		const loadedThemes = themesResult.themes;
		const customThemes = loadedThemes.filter((t) => t.sourcePath);
		const agentPaths = resourceLoader.getAgentPaths();
		const mcpServerCount = getExtensionMcpServers().reduce((n, e) => n + Object.keys(e.mcpServers).length, 0);
		// Plugins register their synthetic extension factory as `plugin:<id>`;
		// split them out so plugins and code extensions each get their own count.
		// (displayName is absent from the options-provided extension shape, so read
		// it structurally.)
		const displayNameOf = (e: object): string | undefined => {
			const value = (e as { displayName?: unknown }).displayName;
			return typeof value === "string" ? value : undefined;
		};
		const pluginCount = extensions.filter((e) => displayNameOf(e)?.startsWith("plugin:")).length;
		const codeExtensionCount = extensions.length - pluginCount;

		const rawMode = deps.getActiveMode();

		// ── Counted capability grid ──────────────────────────────────────────────
		// One cell per loaded capability class (glyph + count + label), so "what
		// can this session do" reads at a glance; names/paths sit one keypress
		// away in the details below. Cells pad to a common width, four per row.
		const plural = (n: number, s: string) => (n === 1 ? s : `${s}s`);
		const cells: Array<{ key: CategoryKey; count: number; label: string }> = [];
		if (skills.length > 0) cells.push({ key: "skills", count: skills.length, label: plural(skills.length, "skill") });
		if (templates.length > 0) {
			cells.push({ key: "commands", count: templates.length, label: plural(templates.length, "command") });
		}
		if (agentPaths.length > 0) {
			cells.push({ key: "agents", count: agentPaths.length, label: plural(agentPaths.length, "agent") });
		}
		if (mcpServerCount > 0) cells.push({ key: "mcp", count: mcpServerCount, label: "MCP" });
		if (pluginCount > 0) cells.push({ key: "plugins", count: pluginCount, label: plural(pluginCount, "plugin") });
		if (codeExtensionCount > 0) {
			cells.push({
				key: "extensions",
				count: codeExtensionCount,
				label: plural(codeExtensionCount, "extension"),
			});
		}
		if (customThemes.length > 0) {
			cells.push({ key: "themes", count: customThemes.length, label: plural(customThemes.length, "theme") });
		}

		if (cells.length > 0) {
			const cellPlain = (c: (typeof cells)[number]) => `${CATEGORY_GLYPH[c.key]} ${c.count} ${c.label}`;
			const cellWidth = Math.max(...cells.map((c) => visibleWidth(cellPlain(c)))) + 3;
			const styledCell = (c: (typeof cells)[number]) => {
				const pad = " ".repeat(Math.max(0, cellWidth - visibleWidth(cellPlain(c))));
				return (
					`${theme.fg("accent", CATEGORY_GLYPH[c.key])} ` +
					`${theme.bold(String(c.count))} ${theme.fg("muted", c.label)}${pad}`
				);
			};
			const PER_ROW = 4;
			const rows: string[] = [];
			for (let i = 0; i < cells.length; i += PER_ROW) {
				rows.push(
					`  ${cells
						.slice(i, i + PER_ROW)
						.map(styledCell)
						.join("")}`.trimEnd(),
				);
			}
			chatContainer.addChild(new Text(rows.join("\n"), 0, 0));
		} else {
			chatContainer.addChild(new Text(theme.fg("dim", "  no project resources loaded"), 0, 0));
		}

		// Context files stay visible in the summary — they shape every reply.
		if (contextFiles.length > 0) {
			const names = contextFiles.map((f) => formatContextPath(f.path, deps.getCwd())).join(", ");
			chatContainer.addChild(
				new Text(`  ${theme.fg("accent", CATEGORY_GLYPH.context)} ${theme.fg("muted", "context")} ${names}`, 0, 0),
			);
		}

		// ── Details: names + scope-grouped paths, one keypress away ─────────────
		// A single expandable holds every per-category listing; collapsed it is
		// just the hint line, so a resource-heavy project no longer buries the
		// prompt under a wall of names at launch.
		const detailSections: string[] = [];
		const metaItems = [`mode/${rawMode}`];
		if (deps.getSubagentEnabled()) metaItems.push("subagent_system_prompt");
		detailSections.push(`${sectionHeader("Resources")}\n${formatCompactList(metaItems)}`);

		if (contextFiles.length > 0) {
			const contextList = contextFiles.map((f) => theme.fg("dim", `  ${formatDisplayPath(f.path)}`)).join("\n");
			detailSections.push(`${sectionHeader("Context")}\n${contextList}`);
		}
		if (skills.length > 0) {
			const groups = buildScopeGroups(
				skills.map((skill) => ({ path: skill.filePath, sourceInfo: skill.sourceInfo })),
			);
			const skillList = formatScopeGroups(groups, {
				formatPath: (item) => formatDisplayPath(item.path),
				formatPackagePath: (item) => getShortPath(item.path, item.sourceInfo),
			});
			detailSections.push(
				`${sectionHeader("Skills")}\n${formatCompactList(skills.map((skill) => skill.name))}\n${skillList}`,
			);
		}
		if (templates.length > 0) {
			const groups = buildScopeGroups(
				templates.map((template) => ({ path: template.filePath, sourceInfo: template.sourceInfo })),
			);
			const templateByPath = new Map(templates.map((t) => [t.filePath, t]));
			const formatTemplate = (item: { path: string }) => {
				const template = templateByPath.get(item.path);
				return template ? `/${template.name}` : formatDisplayPath(item.path);
			};
			const templateList = formatScopeGroups(groups, {
				formatPath: formatTemplate,
				formatPackagePath: formatTemplate,
			});
			detailSections.push(
				`${sectionHeader("Prompts")}\n${formatCompactList(templates.map((t) => `/${t.name}`))}\n${templateList}`,
			);
		}
		if (extensions.length > 0) {
			const groups = buildScopeGroups(extensions);
			const extList = formatScopeGroups(groups, {
				formatPath: (item) => item.displayName ?? formatExtensionDisplayPath(item.path),
				formatPackagePath: (item) =>
					item.displayName ?? formatExtensionDisplayPath(getShortPath(item.path, item.sourceInfo)),
			});
			detailSections.push(
				`${sectionHeader("Extensions")}\n${formatCompactList(getCompactExtensionLabels(extensions))}\n${extList}`,
			);
		}
		if (customThemes.length > 0) {
			const groups = buildScopeGroups(
				customThemes.map((loadedTheme) => ({
					path: loadedTheme.sourcePath!,
					sourceInfo: loadedTheme.sourceInfo,
				})),
			);
			const themeList = formatScopeGroups(groups, {
				formatPath: (item) => formatDisplayPath(item.path),
				formatPackagePath: (item) => getShortPath(item.path, item.sourceInfo),
			});
			detailSections.push(`${sectionHeader("Themes")}\n${themeList}`);
		}

		if (detailSections.length > 0) {
			const expandHint = theme.fg("dim", `  ${appKeyLabel("app.tools.expand")} details`);
			chatContainer.addChild(
				new ExpandableText(
					() => expandHint,
					() => detailSections.join("\n\n"),
					deps.isExpanded(),
					0,
					0,
				),
			);
		}

		if (contextWarnings.length > 0) {
			for (const warning of contextWarnings) {
				chatContainer.addChild(new Text(theme.fg("warning", warning), 0, 0));
			}
		}
	}

	if (showDiagnostics) {
		const skillDiagnostics = skillsResult.diagnostics;
		if (skillDiagnostics.length > 0) {
			const warningLines = formatDiagnostics(skillDiagnostics, sourceInfos);
			chatContainer.addChild(new Text(`${theme.fg("warning", "[Skill conflicts]")}\n${warningLines}`, 0, 0));
			chatContainer.addChild(new Spacer(1));
		}

		const promptDiagnostics = promptsResult.diagnostics;
		if (promptDiagnostics.length > 0) {
			const warningLines = formatDiagnostics(promptDiagnostics, sourceInfos);
			chatContainer.addChild(new Text(`${theme.fg("warning", "[Prompt conflicts]")}\n${warningLines}`, 0, 0));
			chatContainer.addChild(new Spacer(1));
		}

		const extensionDiagnostics: ResourceDiagnostic[] = [];
		const extensionErrors = resourceLoader.getExtensions().errors;
		if (extensionErrors.length > 0) {
			for (const error of extensionErrors) {
				extensionDiagnostics.push({ type: "error", message: error.error, path: error.path });
			}
		}

		const extensionRunner = deps.getExtensionRunner();
		const commandDiagnostics = extensionRunner.getCommandDiagnostics();
		extensionDiagnostics.push(...commandDiagnostics);
		extensionDiagnostics.push(...deps.getBuiltInCommandConflictDiagnostics(extensionRunner));

		const shortcutDiagnostics = extensionRunner.getShortcutDiagnostics();
		extensionDiagnostics.push(...shortcutDiagnostics);

		if (extensionDiagnostics.length > 0) {
			const warningLines = formatDiagnostics(extensionDiagnostics, sourceInfos);
			chatContainer.addChild(new Text(`${theme.fg("warning", "[Extension issues]")}\n${warningLines}`, 0, 0));
			chatContainer.addChild(new Spacer(1));
		}

		const themeDiagnostics = themesResult.diagnostics;
		if (themeDiagnostics.length > 0) {
			const warningLines = formatDiagnostics(themeDiagnostics, sourceInfos);
			chatContainer.addChild(new Text(`${theme.fg("warning", "[Theme conflicts]")}\n${warningLines}`, 0, 0));
			chatContainer.addChild(new Spacer(1));
		}
	}
}
