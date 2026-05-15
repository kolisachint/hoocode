import { copyFile, mkdir, readdir, stat } from "fs/promises";
import { dirname, join } from "path";
import { getHooCodeDir, getTemplatesDir } from "./config.js";

const HOOCODE_DIR = getHooCodeDir();
const TEMPLATES_DIR = getTemplatesDir();

async function exists(p: string): Promise<boolean> {
	try {
		await stat(p);
		return true;
	} catch {
		return false;
	}
}

async function _copyDir(srcDir: string, destDir: string): Promise<void> {
	await mkdir(destDir, { recursive: true });
	let entries: string[];
	try {
		entries = await readdir(srcDir);
	} catch {
		return;
	}
	for (const entry of entries) {
		const srcPath = join(srcDir, entry);
		const destPath = join(destDir, entry);
		const entryStat = await stat(srcPath);
		if (entryStat.isDirectory()) {
			await _copyDir(srcPath, destPath);
		} else {
			await copyFile(srcPath, destPath);
		}
	}
}

async function readDirOrWarn(dir: string, label: string): Promise<string[]> {
	try {
		return await readdir(dir);
	} catch (err) {
		const reason = err instanceof Error ? err.message : String(err);
		console.warn(`hoocode init: could not read bundled ${label} at ${dir} (${reason}). Skipping.`);
		return [];
	}
}

export async function initConfig(): Promise<void> {
	const configPath = join(HOOCODE_DIR, "agent", "hoo-config.json");

	if (await exists(configPath)) {
		return;
	}

	await mkdir(join(HOOCODE_DIR, "modes"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "profiles"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "mcp-servers"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "extensions"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "agent"), { recursive: true });

	await copyFile(join(TEMPLATES_DIR, "default-config.json"), configPath);

	const modesDir = join(TEMPLATES_DIR, "modes");
	const modeNames = await readDirOrWarn(modesDir, "modes");

	for (const modeName of modeNames) {
		const src = join(modesDir, modeName, "system.md");
		const dest = join(HOOCODE_DIR, "modes", modeName, "system.md");
		if (await exists(src)) {
			await mkdir(dirname(dest), { recursive: true });
			await copyFile(src, dest);
		}
	}

	const profilesDir = join(TEMPLATES_DIR, "profiles");
	const profileNames = await readDirOrWarn(profilesDir, "profiles");

	for (const profileName of profileNames) {
		const src = join(profilesDir, profileName, "context.md");
		const dest = join(HOOCODE_DIR, "profiles", profileName, "context.md");
		if (await exists(src)) {
			await mkdir(dirname(dest), { recursive: true });
			await copyFile(src, dest);
		}
	}
}
