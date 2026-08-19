/**
 * Discovery and resolver unit tests.
 *
 * Design: `docs/canvas-extensions-design.md` §4.1, §4.3. The behaviour worth
 * pinning is what discovery deliberately ignores: `package.json` plays no part,
 * because 3 of the 23 extensions in `github/awesome-copilot` ship without one and
 * none ships `node_modules`. A directory is a canvas extension if and only if it
 * contains `extension.mjs`.
 */

import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
	CANVAS_ENTRY_FILE,
	type CanvasSearchRoot,
	canvasSearchRoots,
	discoverCanvasExtensions,
} from "../src/core/canvas/discovery.js";
import { canvasResolverImportArg } from "../src/core/canvas/resolver.js";

describe("canvasSearchRoots", () => {
	it("puts .agents/ first and the Copilot conventions after it", () => {
		const roots = canvasSearchRoots("/repo", "/home/u");
		expect(roots).toEqual([
			{ dir: path.join("/repo", ".agents", "extensions"), scope: "agents" },
			{ dir: path.join("/repo", ".github", "extensions"), scope: "project" },
			{ dir: path.join("/home/u", ".copilot", "extensions"), scope: "user" },
		]);
	});
});

describe("discoverCanvasExtensions", () => {
	let root: string;

	beforeEach(() => {
		root = mkdtempSync(path.join(tmpdir(), "hoocode-canvas-discovery-"));
	});

	afterEach(() => {
		rmSync(root, { recursive: true, force: true });
	});

	function extensionDir(
		scopeDir: string,
		id: string,
		options: { entry?: boolean; packageJson?: boolean } = {},
	): string {
		const dir = path.join(root, scopeDir, id);
		mkdirSync(dir, { recursive: true });
		if (options.entry !== false) writeFileSync(path.join(dir, CANVAS_ENTRY_FILE), "export {};\n");
		if (options.packageJson) writeFileSync(path.join(dir, "package.json"), '{"main":"extension.mjs"}\n');
		return dir;
	}

	function roots(): CanvasSearchRoot[] {
		return [
			{ dir: path.join(root, "agents"), scope: "agents" },
			{ dir: path.join(root, "project"), scope: "project" },
			{ dir: path.join(root, "user"), scope: "user" },
		];
	}

	it("finds a directory containing extension.mjs and names it after the directory", () => {
		extensionDir("project", "pr-artifact-explorer");
		expect(discoverCanvasExtensions(roots())).toEqual([
			{
				id: "pr-artifact-explorer",
				dir: path.join(root, "project", "pr-artifact-explorer"),
				entry: path.join(root, "project", "pr-artifact-explorer", CANVAS_ENTRY_FILE),
				scope: "project",
			},
		]);
	});

	it("finds an extension that ships no package.json", () => {
		extensionDir("project", "no-manifest", { packageJson: false });
		expect(discoverCanvasExtensions(roots()).map((found) => found.id)).toEqual(["no-manifest"]);
	});

	it("ignores a directory with a package.json but no extension.mjs", () => {
		extensionDir("project", "manifest-only", { entry: false, packageJson: true });
		expect(discoverCanvasExtensions(roots())).toEqual([]);
	});

	it("lets the first root win an id collision", () => {
		extensionDir("agents", "board");
		extensionDir("project", "board");
		extensionDir("user", "board");
		const found = discoverCanvasExtensions(roots());
		expect(found).toHaveLength(1);
		expect(found[0]?.scope).toBe("agents");
	});

	it("returns nothing for roots that do not exist", () => {
		expect(discoverCanvasExtensions(roots())).toEqual([]);
	});

	it("follows a symlinked extension directory", () => {
		const target = extensionDir("elsewhere", "linked");
		mkdirSync(path.join(root, "project"), { recursive: true });
		symlinkSync(target, path.join(root, "project", "linked"), "dir");
		expect(discoverCanvasExtensions(roots()).map((found) => found.id)).toEqual(["linked"]);
	});

	it("sorts by directory name so ordering does not depend on the filesystem", () => {
		extensionDir("project", "zeta");
		extensionDir("project", "alpha");
		expect(discoverCanvasExtensions(roots()).map((found) => found.id)).toEqual(["alpha", "zeta"]);
	});
});

describe("canvasResolverImportArg", () => {
	const shimUrl = "file:///repo/dist/sdk-shim/index.js";

	it("produces a data: module that needs no file on disk", () => {
		expect(canvasResolverImportArg(shimUrl).startsWith("data:text/javascript,")).toBe(true);
	});

	it("carries the shim URL through register's data channel, not the environment", () => {
		const decoded = decodeURIComponent(canvasResolverImportArg(shimUrl).slice("data:text/javascript,".length));
		expect(decoded).toContain("register(");
		expect(decoded).toContain(shimUrl);
		expect(decoded).not.toContain("process.env");
	});

	it("maps the package and its subpaths, and nothing else", () => {
		const outer = decodeURIComponent(canvasResolverImportArg(shimUrl).slice("data:text/javascript,".length));
		const hooksUrl = outer.match(/register\("([^"]+)"/)?.[1] ?? "";
		const hooks = decodeURIComponent(hooksUrl.slice("data:text/javascript,".length));
		expect(hooks).toContain('specifier === "@github/copilot-sdk"');
		expect(hooks).toContain('specifier.startsWith("@github/copilot-sdk/")');
		expect(hooks).toContain("return next(specifier, context)");
	});

	it("refuses a shim path that is not an absolute file URL", () => {
		expect(() => canvasResolverImportArg("./relative/shim.js")).toThrow(/absolute file: URL/);
	});
});
