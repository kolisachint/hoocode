import { copyFile, mkdir, readdir, stat } from "fs/promises";
import { dirname, join } from "path";
import { fileURLToPath } from "url";
import { getHooCodeDir } from "./config.js";
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const HOOCODE_DIR = getHooCodeDir();
const TEMPLATES_DIR = join(__dirname, "..", "templates");
async function exists(p) {
    try {
        await stat(p);
        return true;
    }
    catch {
        return false;
    }
}
async function _copyDir(srcDir, destDir) {
    await mkdir(destDir, { recursive: true });
    let entries;
    try {
        entries = await readdir(srcDir);
    }
    catch {
        return;
    }
    for (const entry of entries) {
        const srcPath = join(srcDir, entry);
        const destPath = join(destDir, entry);
        const entryStat = await stat(srcPath);
        if (entryStat.isDirectory()) {
            await _copyDir(srcPath, destPath);
        }
        else {
            await copyFile(srcPath, destPath);
        }
    }
}
export async function initConfig() {
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
    let modeNames = [];
    try {
        modeNames = await readdir(modesDir);
    }
    catch { }
    for (const modeName of modeNames) {
        const src = join(modesDir, modeName, "system.md");
        const dest = join(HOOCODE_DIR, "modes", modeName, "system.md");
        if (await exists(src)) {
            await mkdir(dirname(dest), { recursive: true });
            await copyFile(src, dest);
        }
    }
    const profilesDir = join(TEMPLATES_DIR, "profiles");
    let profileNames = [];
    try {
        profileNames = await readdir(profilesDir);
    }
    catch { }
    for (const profileName of profileNames) {
        const src = join(profilesDir, profileName, "context.md");
        const dest = join(HOOCODE_DIR, "profiles", profileName, "context.md");
        if (await exists(src)) {
            await mkdir(dirname(dest), { recursive: true });
            await copyFile(src, dest);
        }
    }
}
//# sourceMappingURL=init.js.map