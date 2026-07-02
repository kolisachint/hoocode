/**
 * Startup environment probes and the changelog-on-update lookup, shown as
 * notices when the interactive mode starts. Extracted from interactive-mode.ts.
 */

import { spawn } from "node:child_process";
import { APP_NAME, getAgentDir, VERSION } from "../../config.js";
import { DefaultPackageManager } from "../../core/package-manager.js";
import type { SettingsManager } from "../../core/settings-manager.js";
import { getChangelogPath, getNewEntries, parseChangelog } from "../../utils/changelog.js";

export async function checkForPackageUpdates(cwd: string, settingsManager: SettingsManager): Promise<string[]> {
	if (process.env.HOOCODE_OFFLINE) {
		return [];
	}

	try {
		const packageManager = new DefaultPackageManager({
			cwd,
			agentDir: getAgentDir(),
			settingsManager,
		});
		const updates = await packageManager.checkForAvailableUpdates();
		return updates.map((update) => update.displayName);
	} catch {
		return [];
	}
}

export async function checkTmuxKeyboardSetup(): Promise<string | undefined> {
	if (!process.env.TMUX) return undefined;

	const runTmuxShow = (option: string): Promise<string | undefined> => {
		return new Promise((resolve) => {
			const proc = spawn("tmux", ["show", "-gv", option], {
				stdio: ["ignore", "pipe", "ignore"],
			});
			let stdout = "";
			const timer = setTimeout(() => {
				proc.kill();
				resolve(undefined);
			}, 2000);

			proc.stdout?.on("data", (data) => {
				stdout += data.toString();
			});
			proc.on("error", () => {
				clearTimeout(timer);
				resolve(undefined);
			});
			proc.on("close", (code) => {
				clearTimeout(timer);
				resolve(code === 0 ? stdout.trim() : undefined);
			});
		});
	};

	const [extendedKeys, extendedKeysFormat] = await Promise.all([
		runTmuxShow("extended-keys"),
		runTmuxShow("extended-keys-format"),
	]);

	// If we couldn't query tmux (timeout, sandbox, etc.), don't warn
	if (extendedKeys === undefined) return undefined;

	if (extendedKeys !== "on" && extendedKeys !== "always") {
		return "tmux extended-keys is off. Modified Enter keys may not work. Add `set -g extended-keys on` to ~/.tmux.conf and restart tmux.";
	}

	if (extendedKeysFormat === "xterm") {
		return `tmux extended-keys-format is xterm. ${APP_NAME} works best with csi-u. Add \`set -g extended-keys-format csi-u\` to ~/.tmux.conf and restart tmux.`;
	}

	return undefined;
}

function reportInstallTelemetry(_version: string): void {
	// Disabled in HooCode fork: this is forked from pi (upstream pi.dev install-telemetry endpoint is not run by this fork).
}

/**
 * Get changelog entries to display on startup.
 * Only shows new entries since last seen version, skips for resumed sessions.
 */
export function getChangelogForDisplay(options: {
	/** True for resumed/continued sessions (which already have messages). */
	hasMessages: boolean;
	settingsManager: SettingsManager;
}): string | undefined {
	// Skip changelog for resumed/continued sessions (already have messages)
	if (options.hasMessages) {
		return undefined;
	}

	const settingsManager = options.settingsManager;
	const lastVersion = settingsManager.getLastChangelogVersion();
	const changelogPath = getChangelogPath();
	const entries = parseChangelog(changelogPath);

	if (!lastVersion) {
		// Fresh install - record the latest entry's version (not VERSION, which may
		// overshoot the latest entry and silently swallow it once it appears).
		// Fall back to VERSION only if no entries exist yet.
		const seedVersion = entries.length > 0 ? `${entries[0].major}.${entries[0].minor}.${entries[0].patch}` : VERSION;
		settingsManager.setLastChangelogVersion(seedVersion);
		reportInstallTelemetry(seedVersion);
		return undefined;
	}

	const newEntries = getNewEntries(entries, lastVersion);
	if (newEntries.length > 0) {
		const latest = newEntries[0];
		const latestVersion = `${latest.major}.${latest.minor}.${latest.patch}`;
		settingsManager.setLastChangelogVersion(latestVersion);
		reportInstallTelemetry(latestVersion);
		return newEntries.map((e) => e.content).join("\n\n");
	}

	return undefined;
}
