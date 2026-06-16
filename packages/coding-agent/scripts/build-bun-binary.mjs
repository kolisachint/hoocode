#!/usr/bin/env node
/**
 * Builds a standalone, self-contained `hoocode` executable using
 * `bun build --compile`. The binary embeds the Bun runtime plus the bundled
 * application code, so end users do not need Node.js or Bun installed.
 *
 * The runtime resolves a handful of assets next to the executable (see
 * `getPackageDir()` / `isBunBinary` in src/config.ts), so this script also
 * stages those files into the output directory:
 *
 *   <out>/
 *     hoocode[.exe]           compiled executable
 *     package.json            read for name/version/config
 *     README.md, CHANGELOG.md surfaced by docs commands
 *     photon_rs_bg.wasm       photon image processing (fs fallback in photon.ts)
 *     theme/*.json            built-in themes (getThemesDir)
 *     export-html/...         HTML export templates (getExportTemplateDir)
 *     docs/                   provider/model docs (getDocsPath)
 *     examples/               bundled extension examples (getExamplesPath)
 *     templates/              first-run seed templates (getTemplatesDir)
 *
 * Usage:
 *   node scripts/build-bun-binary.mjs [--target <bun-target>] [--outdir <dir>]
 *
 *   --target   Bun compile target (e.g. bun-darwin-arm64, bun-linux-x64,
 *              bun-windows-x64). Defaults to the host (no --target passed).
 *   --outdir   Output directory. Defaults to dist/bun-binary (or
 *              dist/bun-binary/<target> when --target is given).
 *
 * Prerequisites: this script builds the workspace dependencies and this
 * package first, so run it from packages/coding-agent.
 */
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const packageDir = resolve(__dirname, "..");
const repoRoot = resolve(packageDir, "..", "..");
const srcDir = join(packageDir, "src");
const distDir = join(packageDir, "dist");

function parseArgs(argv) {
	const args = { target: undefined, outdir: undefined };
	for (let i = 0; i < argv.length; i++) {
		const arg = argv[i];
		if (arg === "--target") {
			args.target = argv[++i];
		} else if (arg.startsWith("--target=")) {
			args.target = arg.slice("--target=".length);
		} else if (arg === "--outdir") {
			args.outdir = argv[++i];
		} else if (arg.startsWith("--outdir=")) {
			args.outdir = arg.slice("--outdir=".length);
		} else {
			throw new Error(`Unknown argument: ${arg}`);
		}
	}
	return args;
}

function run(command, commandArgs, options = {}) {
	console.log(`> ${command} ${commandArgs.join(" ")}`);
	execFileSync(command, commandArgs, { stdio: "inherit", ...options });
}

function bunRun(scriptDir, script) {
	run("bun", ["run", script], { cwd: scriptDir });
}

const { target, outdir } = parseArgs(process.argv.slice(2));
const isWindowsTarget = (target ?? "").includes("windows");
const exeName = isWindowsTarget ? "hoocode.exe" : "hoocode";
const outDir = resolve(outdir ?? join(distDir, "bun-binary", target ?? ""));

// 1. Build workspace dependencies and this package (produces dist/).
console.log("Building workspace dependencies...");
bunRun(join(repoRoot, "packages", "tui"), "build");
bunRun(join(repoRoot, "packages", "ai"), "build");
bunRun(join(repoRoot, "packages", "agent"), "build");
console.log("Building coding-agent (with embedded templates)...");
bunRun(packageDir, "build:full");

const bunEntry = join(distDir, "bun", "cli.js");
if (!existsSync(bunEntry)) {
	throw new Error(`Expected compiled entry at ${bunEntry} after build. Did the build succeed?`);
}

// 2. Compile the standalone executable.
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
const exePath = join(outDir, exeName);
console.log(`Compiling standalone binary -> ${exePath}`);
const compileArgs = ["build", "--compile", bunEntry, "--outfile", exePath];
if (target) {
	compileArgs.push(`--target=${target}`);
}
run("bun", compileArgs, { cwd: packageDir });

// 3. Stage the runtime assets next to the executable.
console.log("Staging runtime assets...");
const copyFile = (from, to) => {
	mkdirSync(dirname(to), { recursive: true });
	cpSync(from, to);
};
const copyDir = (from, to) => {
	if (!existsSync(from)) return;
	cpSync(from, to, { recursive: true });
};
const copyGlobJson = (fromDir, toDir) => {
	if (!existsSync(fromDir)) return;
	mkdirSync(toDir, { recursive: true });
	for (const name of readdirSync(fromDir).filter((n) => n.endsWith(".json"))) {
		copyFile(join(fromDir, name), join(toDir, name));
	}
};

// package.json + top-level docs read by the runtime.
copyFile(join(packageDir, "package.json"), join(outDir, "package.json"));
for (const f of ["README.md", "CHANGELOG.md"]) {
	const src = join(packageDir, f);
	if (existsSync(src)) copyFile(src, join(outDir, f));
}

// photon wasm (fs.readFileSync fallback in src/utils/photon.ts).
const photonWasm = join(repoRoot, "node_modules", "@silvia-odwyer", "photon-node", "photon_rs_bg.wasm");
if (existsSync(photonWasm)) {
	copyFile(photonWasm, join(outDir, "photon_rs_bg.wasm"));
} else {
	console.warn(`Warning: photon wasm not found at ${photonWasm}; image tools may be degraded.`);
}

// Built-in themes (getThemesDir -> <exe>/theme).
copyGlobJson(join(srcDir, "modes", "interactive", "theme"), join(outDir, "theme"));

// HTML export templates (getExportTemplateDir -> <exe>/export-html).
const exportSrc = join(srcDir, "core", "export-html");
const exportOut = join(outDir, "export-html");
for (const f of ["template.html", "template.css", "template.js"]) {
	copyFile(join(exportSrc, f), join(exportOut, f));
}
copyDir(join(exportSrc, "vendor"), join(exportOut, "vendor"));

// docs/ and examples/ (getDocsPath / getExamplesPath).
copyDir(join(packageDir, "docs"), join(outDir, "docs"));
copyDir(join(packageDir, "examples"), join(outDir, "examples"));

// First-run seed templates (getTemplatesDir -> <exe>/templates).
copyDir(join(packageDir, "templates"), join(outDir, "templates"));

// 4. Smoke test (skip when cross-compiling for another platform).
const version = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf-8")).version;
if (!target) {
	console.log("Smoke-testing the binary (--version)...");
	const out = execFileSync(exePath, ["--version"], { encoding: "utf-8" }).trim();
	if (out !== version) {
		throw new Error(`Smoke test failed: expected version ${version}, got "${out}"`);
	}
	console.log(`OK: ${exeName} reports version ${out}`);
} else {
	console.log(`Cross-compiled for ${target}; skipping smoke test on host.`);
}

console.log(`\nStandalone binary ready: ${exePath}`);
console.log(`Distribute the entire ${outDir} directory (assets must sit next to the executable).`);
