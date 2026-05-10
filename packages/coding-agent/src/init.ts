import { copyFile, mkdir, readdir, stat } from "fs/promises";
import { homedir } from "os";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const HOOCODE_DIR = join(homedir(), ".hoocode");
const TEMPLATES_DIR = join(__dirname, "..", "templates");

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

export async function initConfig(): Promise<void> {
	const configPath = join(HOOCODE_DIR, "config.json");

	if (await exists(configPath)) {
		return;
	}

	await mkdir(join(HOOCODE_DIR, "modes"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "profiles"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "mcp-servers"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "extensions"), { recursive: true });

	await copyFile(join(TEMPLATES_DIR, "default-config.json"), configPath);

	const modesDir = join(TEMPLATES_DIR, "modes");
	let modeNames: string[] = [];
	try {
		modeNames = await readdir(modesDir);
	} catch {}

	for (const modeName of modeNames) {
		const src = join(modesDir, modeName, "system.md");
		const dest = join(HOOCODE_DIR, "modes", modeName, "system.md");
		if (await exists(src)) {
			await mkdir(dirname(dest), { recursive: true });
			await copyFile(src, dest);
		}
	}

	const profilesDir = join(TEMPLATES_DIR, "profiles");
	let profileNames: string[] = [];
	try {
		profileNames = await readdir(profilesDir);
	} catch {}

	for (const profileName of profileNames) {
		const src = join(profilesDir, profileName, "context.md");
		const dest = join(HOOCODE_DIR, "profiles", profileName, "context.md");
		if (await exists(src)) {
			await mkdir(dirname(dest), { recursive: true });
			await copyFile(src, dest);
		}
	}
}
