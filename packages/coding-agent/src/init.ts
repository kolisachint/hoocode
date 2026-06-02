import { mkdir, stat, writeFile } from "fs/promises";
import { dirname, join } from "path";
import { getHooCodeDir } from "./config.js";
import { EMBEDDED_DEFAULT_CONFIG, EMBEDDED_MODES } from "./init-templates.generated.js";

const HOOCODE_DIR = getHooCodeDir();

async function exists(p: string): Promise<boolean> {
	try {
		await stat(p);
		return true;
	} catch {
		return false;
	}
}

async function writeSeedFile(dest: string, contents: string): Promise<void> {
	await mkdir(dirname(dest), { recursive: true });
	await writeFile(dest, contents);
}

export async function initConfig(): Promise<void> {
	const configPath = join(HOOCODE_DIR, "agent", "hoo-config.json");

	if (await exists(configPath)) {
		return;
	}

	await mkdir(join(HOOCODE_DIR, "modes"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "mcp-servers"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "agent", "extensions"), { recursive: true });
	// Skill and agent directories — created up-front so users know where to put files.
	// Use /new-skill <name> or /new-agent <name> to scaffold correctly-formatted files.
	await mkdir(join(HOOCODE_DIR, "agent", "skills"), { recursive: true });
	await mkdir(join(HOOCODE_DIR, "agent", "agents"), { recursive: true });

	await writeSeedFile(configPath, EMBEDDED_DEFAULT_CONFIG);

	for (const [modeName, content] of Object.entries(EMBEDDED_MODES)) {
		await writeSeedFile(join(HOOCODE_DIR, "modes", modeName, "system.md"), content);
	}
}
