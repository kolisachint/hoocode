import chalk from "chalk";
import { spawnSync } from "child_process";
import { createHash, randomBytes } from "crypto";
import extractZip from "extract-zip";
import {
	chmodSync,
	createWriteStream,
	existsSync,
	mkdirSync,
	readdirSync,
	readFileSync,
	renameSync,
	rmSync,
	statSync,
} from "fs";
import { arch, platform } from "os";
import { join } from "path";
import { Readable } from "stream";
import { pipeline } from "stream/promises";
import { APP_NAME, getBinDir } from "../config.js";

const TOOLS_DIR = getBinDir();
const NETWORK_TIMEOUT_MS = 10_000;
const DOWNLOAD_TIMEOUT_MS = 120_000;

function isOfflineModeEnabled(): boolean {
	const value = process.env.HOOCODE_OFFLINE;
	if (!value) return false;
	return value === "1" || value.toLowerCase() === "true" || value.toLowerCase() === "yes";
}

/** Tools whose binaries hoocode can resolve from PATH or download on demand. */
export type ManagedTool = "fd" | "rg" | "webtools" | "filetools" | "browsertools" | "voicetools";

interface ToolConfig {
	name: string;
	repo: string; // GitHub repo (e.g., "sharkdp/fd")
	binaryName: string; // Name of the binary inside the archive
	systemBinaryNames?: string[]; // Alternative system command names to try before downloading
	tagPrefix: string; // Prefix for tags (e.g., "v" for v1.0.0, "" for 1.0.0)
	getAssetName: (version: string, plat: string, architecture: string) => string | null;
}

const TOOLS: Record<string, ToolConfig> = {
	fd: {
		name: "fd",
		repo: "sharkdp/fd",
		binaryName: "fd",
		systemBinaryNames: ["fd", "fdfind"],
		tagPrefix: "v",
		getAssetName: (version, plat, architecture) => {
			if (plat === "darwin") {
				const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
				return `fd-v${version}-${archStr}-apple-darwin.tar.gz`;
			} else if (plat === "linux") {
				const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
				return `fd-v${version}-${archStr}-unknown-linux-gnu.tar.gz`;
			} else if (plat === "win32") {
				const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
				return `fd-v${version}-${archStr}-pc-windows-msvc.zip`;
			}
			return null;
		},
	},
	rg: {
		name: "ripgrep",
		repo: "BurntSushi/ripgrep",
		binaryName: "rg",
		tagPrefix: "",
		getAssetName: (version, plat, architecture) => {
			if (plat === "darwin") {
				const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
				return `ripgrep-${version}-${archStr}-apple-darwin.tar.gz`;
			} else if (plat === "linux") {
				if (architecture === "arm64") {
					return `ripgrep-${version}-aarch64-unknown-linux-gnu.tar.gz`;
				}
				return `ripgrep-${version}-x86_64-unknown-linux-musl.tar.gz`;
			} else if (plat === "win32") {
				const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
				return `ripgrep-${version}-${archStr}-pc-windows-msvc.zip`;
			}
			return null;
		},
	},
	webtools: {
		name: "webtools",
		repo: "kolisachint/webtools",
		binaryName: "webtools",
		tagPrefix: "v",
		// Release assets follow Rust target triples: webtools-<arch>-<target>.<ext>.
		// Some platforms may not be published yet; a missing asset 404s and ensureTool
		// degrades gracefully (returns undefined, tools fall back to an error message).
		getAssetName: (_version, plat, architecture) => {
			const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
			if (plat === "darwin") {
				return `webtools-${archStr}-apple-darwin.tar.gz`;
			} else if (plat === "linux") {
				return `webtools-${archStr}-unknown-linux-gnu.tar.gz`;
			} else if (plat === "win32") {
				return `webtools-${archStr}-pc-windows-msvc.zip`;
			}
			return null;
		},
	},
	browsertools: {
		name: "browsertools",
		repo: "kolisachint/browsertools",
		binaryName: "browsertools",
		tagPrefix: "v",
		// Release archives follow Rust target triples: browsertools-<arch>-<target>.<ext>
		// (release.yml builds gnu + musl for linux; we prefer the gnu variant to match
		// webtools/filetools). A missing platform asset 404s and ensureTool degrades
		// gracefully (returns undefined; the browser tools then surface an error).
		getAssetName: (_version, plat, architecture) => {
			const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
			if (plat === "darwin") {
				return `browsertools-${archStr}-apple-darwin.tar.gz`;
			} else if (plat === "linux") {
				return `browsertools-${archStr}-unknown-linux-gnu.tar.gz`;
			} else if (plat === "win32") {
				return `browsertools-${archStr}-pc-windows-msvc.zip`;
			}
			return null;
		},
	},
	voicetools: {
		name: "voicetools",
		repo: "kolisachint/voicetools",
		binaryName: "voicetools",
		tagPrefix: "v",
		// Release archives follow Rust target triples: voicetools-<arch>-<target>.<ext>.
		// A missing platform asset 404s and ensureTool degrades gracefully (returns
		// undefined; the voice-transcribe caller then surfaces an error message).
		getAssetName: (_version, plat, architecture) => {
			const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
			if (plat === "darwin") {
				return `voicetools-${archStr}-apple-darwin.tar.gz`;
			} else if (plat === "linux") {
				return `voicetools-${archStr}-unknown-linux-gnu.tar.gz`;
			} else if (plat === "win32") {
				return `voicetools-${archStr}-pc-windows-msvc.zip`;
			}
			return null;
		},
	},
	filetools: {
		name: "filetools",
		repo: "kolisachint/filetools",
		binaryName: "filetools",
		tagPrefix: "v",
		// Release archives are named `filetools-<target-triple>.<ext>` (see the repo's
		// release.yml `archive: filetools-$target`). A missing platform asset 404s and
		// ensureTool degrades gracefully (returns undefined; the doc tools then surface
		// an error result).
		getAssetName: (_version, plat, architecture) => {
			const archStr = architecture === "arm64" ? "aarch64" : "x86_64";
			if (plat === "darwin") {
				return `filetools-${archStr}-apple-darwin.tar.gz`;
			} else if (plat === "linux") {
				return `filetools-${archStr}-unknown-linux-gnu.tar.gz`;
			} else if (plat === "win32") {
				return `filetools-${archStr}-pc-windows-msvc.zip`;
			}
			return null;
		},
	},
};

// Check if a command exists in PATH by trying to run it
function commandExists(cmd: string): boolean {
	try {
		const result = spawnSync(cmd, ["--version"], { stdio: "pipe" });
		// Check for ENOENT error (command not found)
		return result.error === undefined || result.error === null;
	} catch {
		return false;
	}
}

// Resolved tool paths are stable for the life of the process. Cache the first
// successful resolution so we never re-run the synchronous spawnSync probe in
// commandExists() on every grep/find/glob invocation, which blocks the event loop.
const resolvedToolPathCache = new Map<ManagedTool, string>();

// Get the path to a tool (system-wide or in our tools dir)
export function getToolPath(tool: ManagedTool): string | null {
	const config = TOOLS[tool];
	if (!config) return null;

	// Explicit binary override (per-tool env var, e.g. HOOCODE_BROWSERTOOLS_BINARY).
	// Lets a developer point at a locally built binary that predates a release,
	// bypassing the tools-dir/PATH resolution and download. Authoritative when set
	// and the path exists.
	const overrideEnv = `HOOCODE_${tool.toUpperCase()}_BINARY`;
	const override = process.env[overrideEnv]?.trim();
	if (override && existsSync(override)) {
		return override;
	}

	// Reuse a previously resolved path. A bare command name resolves via PATH;
	// an absolute path must still exist (revalidate cheaply with existsSync).
	const cached = resolvedToolPathCache.get(tool);
	if (cached !== undefined) {
		const isAbsolutePath = cached.includes("/") || cached.includes("\\");
		if (!isAbsolutePath || existsSync(cached)) {
			return cached;
		}
		resolvedToolPathCache.delete(tool);
	}

	// Check our tools directory first
	const localPath = join(TOOLS_DIR, config.binaryName + (platform() === "win32" ? ".exe" : ""));
	if (existsSync(localPath)) {
		resolvedToolPathCache.set(tool, localPath);
		return localPath;
	}

	// Check system PATH - if found, just return the command name (it's in PATH)
	const systemBinaryNames = config.systemBinaryNames ?? [config.binaryName];
	for (const systemBinaryName of systemBinaryNames) {
		if (commandExists(systemBinaryName)) {
			resolvedToolPathCache.set(tool, systemBinaryName);
			return systemBinaryName;
		}
	}

	return null;
}

// Fetch latest release version from GitHub
async function getLatestVersion(repo: string): Promise<string> {
	const response = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
		headers: { "User-Agent": `${APP_NAME}-coding-agent` },
		signal: AbortSignal.timeout(NETWORK_TIMEOUT_MS),
	});

	if (!response.ok) {
		throw new Error(`GitHub API error: ${response.status}`);
	}

	const data = (await response.json()) as { tag_name: string };
	return data.tag_name.replace(/^v/, "");
}

// Best-effort SHA-256 verification: fetch "<downloadUrl>.sha256" and, when it is
// served (HTTP 200), verify the downloaded file against it. A 404 (or any other
// non-200 / network error) means no published checksum, so verification is
// skipped rather than treated as a failure. A genuine mismatch throws.
async function verifyChecksum(downloadUrl: string, filePath: string): Promise<void> {
	let checksumResponse: Awaited<ReturnType<typeof fetch>>;
	try {
		checksumResponse = await fetch(`${downloadUrl}.sha256`, {
			signal: AbortSignal.timeout(NETWORK_TIMEOUT_MS),
		});
	} catch {
		// Network error fetching the checksum is non-fatal for best-effort verification.
		return;
	}

	if (checksumResponse.status !== 200) {
		return;
	}

	// sha256 files are commonly "<hex>  <filename>"; take the leading token.
	const expectedHash = (await checksumResponse.text()).trim().split(/\s+/)[0]?.toLowerCase();
	if (!expectedHash || !/^[0-9a-f]{64}$/.test(expectedHash)) {
		// Unusable checksum body: skip rather than fail (still best-effort).
		return;
	}

	const actualHash = createHash("sha256").update(readFileSync(filePath)).digest("hex");
	if (actualHash !== expectedHash) {
		throw new Error(`Checksum mismatch for ${downloadUrl}: expected ${expectedHash}, got ${actualHash}`);
	}
}

/**
 * Live download progress: bytes received so far and the total from
 * Content-Length (null when the server doesn't send one). Callers use it to
 * drive a progress bar instead of an indefinite spinner.
 */
export type DownloadProgress = (receivedBytes: number, totalBytes: number | null) => void;

// Download a file from URL into `dest`, validating integrity. Throws (and removes
// the partial file) on a truncated transfer (bytes written != Content-Length when
// the header is present) or a SHA-256 mismatch, so a corrupt artifact is never
// left behind. Exported for tests.
export async function downloadFile(url: string, dest: string, onProgress?: DownloadProgress): Promise<void> {
	try {
		const response = await fetch(url, {
			signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS),
		});

		if (!response.ok) {
			throw new Error(`Failed to download: ${response.status}`);
		}

		if (!response.body) {
			throw new Error("No response body");
		}

		const contentLengthHeader = response.headers.get("content-length");
		const expectedBytes =
			contentLengthHeader !== null && contentLengthHeader.trim() !== "" ? Number(contentLengthHeader) : null;

		const fileStream = createWriteStream(dest);
		const source = Readable.fromWeb(response.body as Parameters<typeof Readable.fromWeb>[0]);
		if (onProgress) {
			let received = 0;
			source.on("data", (chunk: Buffer) => {
				received += chunk.length;
				onProgress(received, expectedBytes);
			});
		}
		await pipeline(source, fileStream);

		if (expectedBytes !== null && Number.isFinite(expectedBytes)) {
			const bytesWritten = statSync(dest).size;
			if (bytesWritten !== expectedBytes) {
				throw new Error(
					`Truncated download from ${url}: expected ${expectedBytes} bytes, received ${bytesWritten}`,
				);
			}
		}

		await verifyChecksum(url, dest);
	} catch (e) {
		// Never leave a partial/corrupt file behind on any failure.
		rmSync(dest, { force: true });
		throw e;
	}
}

function findBinaryRecursively(rootDir: string, binaryFileName: string): string | null {
	const stack: string[] = [rootDir];

	while (stack.length > 0) {
		const currentDir = stack.pop();
		if (!currentDir) continue;

		const entries = readdirSync(currentDir, { withFileTypes: true });
		for (const entry of entries) {
			const fullPath = join(currentDir, entry.name);
			if (entry.isFile() && entry.name === binaryFileName) {
				return fullPath;
			}
			if (entry.isDirectory()) {
				stack.push(fullPath);
			}
		}
	}

	return null;
}

// Download and install a tool
async function downloadTool(tool: ManagedTool, onProgress?: DownloadProgress): Promise<string> {
	const config = TOOLS[tool];
	if (!config) throw new Error(`Unknown tool: ${tool}`);

	const plat = platform();
	const architecture = arch();

	// Get latest version
	const version = await getLatestVersion(config.repo);

	// Get asset name for this platform
	const assetName = config.getAssetName(version, plat, architecture);
	if (!assetName) {
		throw new Error(`Unsupported platform: ${plat}/${architecture}`);
	}

	// Create tools directory
	mkdirSync(TOOLS_DIR, { recursive: true });

	const downloadUrl = `https://github.com/${config.repo}/releases/download/${config.tagPrefix}${version}/${assetName}`;
	const archivePath = join(TOOLS_DIR, assetName);
	const binaryExt = plat === "win32" ? ".exe" : "";
	const binaryPath = join(TOOLS_DIR, config.binaryName + binaryExt);

	// Download to a unique temp path, validate, then atomically rename into place.
	// Writing the shared archive path directly would leave a corrupt partial behind
	// if the transfer fails or is truncated. fd/rg/webtools can also download
	// concurrently at startup, so the per-attempt temp name must be unique.
	const tempArchivePath = `${archivePath}.${process.pid}.${randomBytes(6).toString("hex")}.part`;

	// Extract into a unique temp directory. fd and rg downloads can run concurrently
	// during startup, so sharing a fixed directory causes races.
	const extractDir = join(
		TOOLS_DIR,
		`extract_tmp_${config.binaryName}_${process.pid}_${Date.now()}_${Math.random().toString(36).slice(2, 10)}`,
	);

	try {
		// One retry (2 attempts total) around download + integrity verification.
		// downloadFile removes its own partial on failure, so each attempt is clean.
		let lastError: unknown;
		let downloaded = false;
		for (let attempt = 1; attempt <= 2 && !downloaded; attempt++) {
			try {
				await downloadFile(downloadUrl, tempArchivePath, onProgress);
				downloaded = true;
			} catch (e) {
				lastError = e;
				rmSync(tempArchivePath, { force: true });
			}
		}
		if (!downloaded) {
			throw lastError instanceof Error ? lastError : new Error(String(lastError));
		}

		// Atomic publish of the verified archive, then extract.
		renameSync(tempArchivePath, archivePath);
		mkdirSync(extractDir, { recursive: true });

		if (assetName.endsWith(".tar.gz")) {
			const extractResult = spawnSync("tar", ["xzf", archivePath, "-C", extractDir], { stdio: "pipe" });
			if (extractResult.error || extractResult.status !== 0) {
				const errMsg = extractResult.error?.message ?? extractResult.stderr?.toString().trim() ?? "unknown error";
				throw new Error(`Failed to extract ${assetName}: ${errMsg}`);
			}
		} else if (assetName.endsWith(".zip")) {
			await extractZip(archivePath, { dir: extractDir });
		} else {
			throw new Error(`Unsupported archive format: ${assetName}`);
		}

		// Find the binary in extracted files. Some archives contain files directly
		// at root, others nest under a versioned subdirectory.
		const binaryFileName = config.binaryName + binaryExt;
		const extractedDir = join(extractDir, assetName.replace(/\.(tar\.gz|zip)$/, ""));
		const extractedBinaryCandidates = [join(extractedDir, binaryFileName), join(extractDir, binaryFileName)];
		let extractedBinary = extractedBinaryCandidates.find((candidate) => existsSync(candidate));

		if (!extractedBinary) {
			extractedBinary = findBinaryRecursively(extractDir, binaryFileName) ?? undefined;
		}

		if (extractedBinary) {
			renameSync(extractedBinary, binaryPath);
		} else {
			throw new Error(`Binary not found in archive: expected ${binaryFileName} under ${extractDir}`);
		}

		// Make executable (Unix only)
		if (plat !== "win32") {
			chmodSync(binaryPath, 0o755);
		}
	} finally {
		// Guaranteed cleanup of every transient artifact on ANY outcome: the temp
		// download (if a failure left it before the rename), the published archive,
		// and the temp extract dir.
		rmSync(tempArchivePath, { force: true });
		rmSync(archivePath, { force: true });
		rmSync(extractDir, { recursive: true, force: true });
	}

	return binaryPath;
}

// Termux package names for tools
const TERMUX_PACKAGES: Record<string, string> = {
	fd: "fd",
	rg: "ripgrep",
	webtools: "webtools",
	filetools: "filetools",
	browsertools: "browsertools",
	voicetools: "voicetools",
};

// Ensure a tool is available, downloading if necessary
// Returns the path to the tool, or null if unavailable
// onProgress receives live byte counts while the release archive downloads.
export async function ensureTool(
	tool: ManagedTool,
	silent: boolean = false,
	onProgress?: DownloadProgress,
): Promise<string | undefined> {
	const existingPath = getToolPath(tool);
	if (existingPath) {
		return existingPath;
	}

	const config = TOOLS[tool];
	if (!config) return undefined;

	if (isOfflineModeEnabled()) {
		if (!silent) {
			console.log(chalk.yellow(`${config.name} not found. Offline mode enabled, skipping download.`));
		}
		return undefined;
	}

	// On Android/Termux, Linux binaries don't work due to Bionic libc incompatibility.
	// Users must install via pkg.
	if (platform() === "android") {
		const pkgName = TERMUX_PACKAGES[tool] ?? tool;
		if (!silent) {
			console.log(chalk.yellow(`${config.name} not found. Install with: pkg install ${pkgName}`));
		}
		return undefined;
	}

	// Tool not found - download it
	if (!silent) {
		console.log(chalk.dim(`${config.name} not found. Downloading...`));
	}

	try {
		const path = await downloadTool(tool, onProgress);
		if (!silent) {
			console.log(chalk.dim(`${config.name} installed to ${path}`));
		}
		return path;
	} catch (e) {
		if (!silent) {
			console.log(chalk.yellow(`Failed to download ${config.name}: ${e instanceof Error ? e.message : e}`));
		}
		return undefined;
	}
}
