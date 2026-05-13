#!/usr/bin/env node
/**
 * Hoist a `bun install` (default isolated layout) to a flat node_modules
 * tree that tsgo / tsc / npm-style tooling can consume.
 *
 * Bun puts transitive deps under `node_modules/.bun/node_modules/<scope>/<name>`
 * and only direct deps at `node_modules/<scope>/<name>`. Walk both:
 *   1. Symlink every `node_modules/.bun/node_modules/{*|@scope}/{name}` to
 *      `node_modules/{*|@scope}/{name}` if no entry exists there.
 *   2. Symlink every `packages/*` workspace package to
 *      `node_modules/{name}` based on its `package.json#name`.
 *
 * Idempotent: existing entries (real dirs or symlinks) are left alone.
 */

import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, symlinkSync } from "node:fs";
import { dirname, join } from "node:path";

const NM = "node_modules";
const BUN_NM = join(NM, ".bun", "node_modules");

function ensureSymlink(linkPath, targetRel) {
	try {
		lstatSync(linkPath);
		return false;
	} catch {}
	mkdirSync(dirname(linkPath), { recursive: true });
	symlinkSync(targetRel, linkPath);
	return true;
}

let hoistedDeps = 0;
if (existsSync(BUN_NM)) {
	for (const entry of readdirSync(BUN_NM, { withFileTypes: true })) {
		if (entry.name.startsWith("@")) {
			let subEntries;
			try {
				subEntries = readdirSync(join(BUN_NM, entry.name), { withFileTypes: true });
			} catch {
				continue;
			}
			for (const sub of subEntries) {
				const linkPath = join(NM, entry.name, sub.name);
				const targetRel = join("..", ".bun", "node_modules", entry.name, sub.name);
				if (ensureSymlink(linkPath, targetRel)) hoistedDeps++;
			}
		} else {
			const linkPath = join(NM, entry.name);
			const targetRel = join(".bun", "node_modules", entry.name);
			if (ensureSymlink(linkPath, targetRel)) hoistedDeps++;
		}
	}
}

let hoistedWorkspaces = 0;
if (existsSync("packages")) {
	for (const entry of readdirSync("packages", { withFileTypes: true })) {
		if (!entry.isDirectory()) continue;
		const pkgJsonPath = join("packages", entry.name, "package.json");
		if (!existsSync(pkgJsonPath)) continue;
		let pkg;
		try {
			pkg = JSON.parse(readFileSync(pkgJsonPath, "utf8"));
		} catch {
			continue;
		}
		if (!pkg.name) continue;
		const isScoped = pkg.name.startsWith("@");
		const linkPath = isScoped ? join(NM, ...pkg.name.split("/")) : join(NM, pkg.name);
		// node_modules/<name> -> 1x ".." to repo root; node_modules/@scope/<name> -> 2x ".."
		const upCount = isScoped ? 2 : 1;
		const targetRel = join(...Array(upCount).fill(".."), "packages", entry.name);
		if (ensureSymlink(linkPath, targetRel)) hoistedWorkspaces++;
	}
}

console.log(`hoisted ${hoistedDeps} bun deps, ${hoistedWorkspaces} workspace packages`);
