#!/usr/bin/env node
// Runs the native hoocode binary from the platform package npm/bun installed
// alongside this one (an optionalDependency picked by os/cpu). No postinstall.

"use strict";

const { spawnSync } = require("node:child_process");

const PLATFORMS = {
	"darwin-arm64": "@kolisachint/hoocode-darwin-arm64",
	"darwin-x64": "@kolisachint/hoocode-darwin-x64",
	"linux-arm64": "@kolisachint/hoocode-linux-arm64",
	"linux-x64": "@kolisachint/hoocode-linux-x64",
};

const key = `${process.platform}-${process.arch}`;
const pkg = PLATFORMS[key];
if (!pkg) {
	console.error(`hoocode: no prebuilt binary for ${key}.`);
	console.error("Supported: macOS and Linux on x64 and arm64.");
	process.exit(1);
}

let bin;
try {
	bin = require.resolve(`${pkg}/bin/hoocode`);
} catch {
	// This launcher ships under two names (`hoocode` and `@kolisachint/hoocode`),
	// so the reinstall hint uses whichever one installed this copy.
	let selfName = "@kolisachint/hoocode";
	try {
		selfName = require("../package.json").name;
	} catch {}
	console.error(`hoocode: the platform package ${pkg} is not installed.`);
	console.error("It is an optional dependency; reinstall without --no-optional / --omit=optional:");
	console.error(`  npm install -g ${selfName}`);
	process.exit(1);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
	console.error(`hoocode: could not start ${bin}: ${result.error.message}`);
	process.exit(1);
}
if (result.signal) {
	process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
