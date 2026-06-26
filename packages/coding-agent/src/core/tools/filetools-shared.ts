/**
 * Shared plumbing for the `DocRead` / `DocEdit` / `DocWrite` tools.
 *
 * All three shell out to the `filetools` binary (extract / reconstruct
 * subcommands, resolved/downloaded via {@link ensureTool}) to losslessly
 * project structured/binary documents (XML, drawio, OOXML, PDF) into editable,
 * id-addressed JSON and reconstruct them after id-based patches.
 *
 * Unlike webtools, the filetools CLI is file-oriented, not stdout-oriented:
 * `extract` writes the envelope JSON to `--out` and the sidecar id-map next to
 * it, emitting only a human status line on stderr. This module therefore:
 * - owns a per-process working directory where envelopes + sidecars live,
 * - runs extract/reconstruct and reads the resulting files back,
 * - keeps a small cache mapping a source file to its extracted envelope +
 *   sidecar, so a DocRead can be followed by a DocEdit/DocWrite (the stateful
 *   extract -> patch -> reconstruct flow), and
 * - exposes the locked JSON wire types mirroring the Rust `model.rs`/`patch.rs`.
 */

import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { type Static, Type } from "typebox";
import { APP_NAME } from "../../config.js";
import { ensureTool } from "../../utils/tools-manager.js";
import { execCommand } from "../exec.js";

/** Default timeout (seconds) for a single filetools invocation. */
export const FILETOOLS_DEFAULT_TIMEOUT_SECS = 30;

/**
 * Soft token ceiling for a single DocRead render. The filetools binary has no
 * pagination, so a dense file (e.g. a large spreadsheet) can project into a
 * huge id-addressed dump that floods the model context and burns tokens. We
 * cannot make the extract itself smaller without the binary's help, so DocRead
 * truncates the rendered view to roughly this budget and tells the model how to
 * narrow it (readonly projection, a smaller/targeted file, or direct edits).
 */
export const DOCREAD_MAX_RENDER_TOKENS = 10000;

/** Rough token estimate (chars/4), matching the agent's compaction heuristic. */
export function estimateTextTokens(text: string): number {
	return Math.ceil(text.length / 4);
}

/**
 * Truncate rendered envelope lines to roughly `maxTokens`, keeping whole lines.
 * Returns the kept text plus how many lines were dropped (0 when nothing was
 * truncated).
 */
export function truncateRenderToTokenBudget(
	lines: string[],
	maxTokens: number = DOCREAD_MAX_RENDER_TOKENS,
): { text: string; droppedLines: number } {
	const full = lines.join("\n");
	if (estimateTextTokens(full) <= maxTokens) {
		return { text: full, droppedLines: 0 };
	}
	const budgetChars = maxTokens * 4;
	const kept: string[] = [];
	let used = 0;
	for (const line of lines) {
		const next = used + line.length + 1; // + newline
		if (next > budgetChars && kept.length > 0) break;
		kept.push(line);
		used = next;
	}
	return { text: kept.join("\n"), droppedLines: lines.length - kept.length };
}

// ============================================================================
// Wire types (locked against `filetools` model.rs / patch.rs)
// ============================================================================

/** How faithfully a handler can reconstruct a file after edits. */
export type Fidelity = "lossless" | "in_place_text" | "read_only";

export interface DocSource {
	path: string;
	/** Logical format, e.g. "xml", "drawio". */
	type: string;
	/** `sha256:<hex>` of the original bytes. */
	hash: string;
}

export interface DocAttr {
	name: string;
	value: string;
}

export interface DocNode {
	id: string;
	tag: string;
	attrs?: DocAttr[];
	text?: string;
	children?: DocNode[];
}

/** The extract output handed to the model. Mirrors the Rust `Envelope`. */
export interface Envelope {
	version: string;
	source: DocSource;
	fidelity: Fidelity;
	writable: boolean;
	idmap_ref?: string;
	structure: DocNode[];
}

/** A new element for an `add` op (text-only content, v1). */
export interface NewElement {
	tag: string;
	attrs?: DocAttr[];
	text?: string;
}

/**
 * One patch operation. RFC-6902 vocabulary, id-based pointers
 * (`/structure/<id>/text`, `/structure/<id>/attrs/<name>`), per the filetools
 * patch format.
 */
export type PatchOp =
	| { op: "test"; path: string; hash: string }
	| { op: "replace"; path: string; value: string }
	| { op: "add"; after?: string; before?: string; value: NewElement }
	| { op: "remove"; path: string };

export interface Patch {
	patch: PatchOp[];
}

// ----------------------------------------------------------------------------
// TypeBox schema for the model-facing patch input (shared by DocEdit/DocWrite)
// ----------------------------------------------------------------------------

const attrSchema = Type.Object({
	name: Type.String(),
	value: Type.String(),
});

const newElementSchema = Type.Object({
	tag: Type.String({ description: 'Element tag name, e.g. "w:p" or "mxCell".' }),
	attrs: Type.Optional(Type.Array(attrSchema, { description: "Attributes in document order." })),
	text: Type.Optional(Type.String({ description: "Inline text content (text-only elements, v1)." })),
});

const patchOpSchema = Type.Union([
	Type.Object(
		{
			op: Type.Literal("test"),
			path: Type.String({ description: "Pointer `/structure/<id>` (or /text, /attrs/<name>) to guard." }),
			hash: Type.String({ description: "Expected content hash of the target node." }),
		},
		{ description: "Optimistic guard: assert the target node's content hash before mutating." },
	),
	Type.Object(
		{
			op: Type.Literal("replace"),
			path: Type.String({
				description: "`/structure/<id>/text` for element text, or `/structure/<id>/attrs/<name>` for an attribute.",
			}),
			value: Type.String({ description: "New text or attribute value." }),
		},
		{ description: "Replace an element's text or an attribute value." },
	),
	Type.Object(
		{
			op: Type.Literal("add"),
			after: Type.Optional(Type.String({ description: "Anchor node id to insert AFTER." })),
			before: Type.Optional(Type.String({ description: "Anchor node id to insert BEFORE." })),
			value: newElementSchema,
		},
		{ description: "Insert a new element next to an anchor. Provide exactly one of `after`/`before`." },
	),
	Type.Object(
		{
			op: Type.Literal("remove"),
			path: Type.String({ description: "Pointer `/structure/<id>` of the element to delete." }),
		},
		{ description: "Delete an element and all its bytes." },
	),
]);

/**
 * The model-facing patch parameter: an array of id-based RFC-6902 ops, matching
 * the filetools patch wire format. Shared by DocEdit and DocWrite.
 */
export const patchOpsSchema = Type.Array(patchOpSchema, {
	description:
		"Ordered id-based patch ops (test/replace/add/remove) targeting node ids from a prior DocRead. Applied atomically.",
});

export type PatchOpsInput = Static<typeof patchOpsSchema>;

/** Wrap the model-facing ops array into the binary's `{ patch: [...] }` envelope. */
export function toPatch(ops: PatchOpsInput): Patch {
	return { patch: ops as PatchOp[] };
}

// ============================================================================
// Binary runner + working directory
// ============================================================================

const BINARY_MISSING_MESSAGE =
	"filetools binary unavailable and could not be downloaded — the document tools require the `filetools` CLI on PATH or a published release for this platform";

/** Lazily-created per-process working directory for envelopes + sidecars. */
let workDir: string | undefined;
function getWorkDir(): string {
	if (workDir) return workDir;
	const base = join(tmpdir(), `${APP_NAME}-filetools`);
	mkdirSync(base, { recursive: true });
	workDir = mkdtempSync(join(base, "doc-"));
	return workDir;
}

/** Short, filesystem-safe key for a source path (used to name its subdir). */
function pathKey(absolutePath: string): string {
	return createHash("sha256").update(absolutePath).digest("hex").slice(0, 16);
}

async function resolveBinary(): Promise<string> {
	const binaryPath = await ensureTool("filetools", true);
	if (!binaryPath) throw new Error(BINARY_MISSING_MESSAGE);
	return binaryPath;
}

async function runFiletools(
	binaryPath: string,
	subcommand: "extract" | "reconstruct",
	args: string[],
	cwd: string,
	signal: AbortSignal | undefined,
	timeoutSecs: number,
): Promise<string> {
	if (signal?.aborted) throw new Error("Operation aborted");
	const spawnTimeoutMs = (timeoutSecs + 5) * 1000;
	const result = await execCommand(binaryPath, [subcommand, ...args], cwd, { signal, timeout: spawnTimeoutMs });
	if (signal?.aborted) throw new Error("Operation aborted");
	if (result.killed) throw new Error(`filetools ${subcommand} timed out after ${timeoutSecs}s`);
	if (result.code !== 0) {
		const stderr = result.stderr.trim();
		throw new Error(stderr || `filetools ${subcommand} exited with code ${result.code}`);
	}
	// Status goes to stderr; callers read the produced files, not stdout.
	return result.stderr.trim();
}

// ============================================================================
// Extraction cache (source file -> extracted envelope + sidecar)
// ============================================================================

export interface ExtractRecord {
	/** Absolute path of the source document. */
	source: string;
	/** Path to the envelope JSON in the working directory. */
	envelopePath: string;
	/** Parsed envelope (also returned to the model on DocRead). */
	envelope: Envelope;
	/** The source's stat signature at extract time, to detect drift cheaply. */
	signature: string;
}

const records = new Map<string, ExtractRecord>();

function statSignature(absolutePath: string): string {
	try {
		const st = statSync(absolutePath);
		return `${st.mtimeMs}:${st.size}`;
	} catch {
		return "absent";
	}
}

/**
 * Extract `absolutePath` to an envelope (+ sidecar) in the working directory,
 * cache the result keyed by the source path, and return the parsed envelope.
 *
 * `readonly` strips ids for a smaller, analysis-only projection that cannot be
 * reconstructed (DocRead's default-off mode).
 */
export async function extractDocument(
	absolutePath: string,
	cwd: string,
	signal: AbortSignal | undefined,
	options?: { readonly?: boolean; timeoutSecs?: number },
): Promise<Envelope> {
	const binaryPath = await resolveBinary();
	const dir = join(getWorkDir(), pathKey(absolutePath));
	mkdirSync(dir, { recursive: true });
	const envelopePath = join(dir, "envelope.json");

	const args = ["--input", absolutePath, "--out", envelopePath];
	if (options?.readonly) args.push("--readonly");
	await runFiletools(binaryPath, "extract", args, cwd, signal, options?.timeoutSecs ?? FILETOOLS_DEFAULT_TIMEOUT_SECS);

	const envelope = readEnvelope(envelopePath);
	if (!options?.readonly) {
		records.set(absolutePath, {
			source: absolutePath,
			envelopePath,
			envelope,
			signature: statSignature(absolutePath),
		});
	}
	return envelope;
}

function readEnvelope(envelopePath: string): Envelope {
	let raw: string;
	try {
		raw = readFileSync(envelopePath, "utf8");
	} catch {
		throw new Error("filetools extract produced no envelope");
	}
	try {
		return JSON.parse(raw) as Envelope;
	} catch {
		throw new Error("filetools extract produced a malformed envelope");
	}
}

/** Look up a cached extraction for `absolutePath`, if one is still valid. */
export function getExtractRecord(absolutePath: string): ExtractRecord | undefined {
	const record = records.get(absolutePath);
	if (!record) return undefined;
	// Drop a stale record if the source changed since extract; reconstruct would
	// fail the binary's hash-drift guard anyway, but a clearer error is better.
	if (record.signature !== statSignature(absolutePath)) {
		records.delete(absolutePath);
		return undefined;
	}
	return record;
}

/** Drop any cached extraction for `absolutePath`. */
export function invalidateExtractRecord(absolutePath: string): void {
	records.delete(absolutePath);
}

/**
 * Apply `patch` to a previously-extracted document, writing the reconstructed
 * bytes to `outPath`. Requires a prior {@link extractDocument} (the stateful
 * flow): the cached envelope + sidecar carry the id-map reconstruct needs.
 */
export async function reconstructDocument(
	absolutePath: string,
	patch: Patch,
	outPath: string,
	cwd: string,
	signal: AbortSignal | undefined,
	options?: { timeoutSecs?: number },
): Promise<void> {
	const record = getExtractRecord(absolutePath);
	if (!record) {
		throw new Error(
			`no extracted envelope for ${basename(absolutePath)} — run DocRead on it first, then DocEdit/DocWrite`,
		);
	}
	if (!record.envelope.writable) {
		throw new Error(
			`${basename(absolutePath)} is read-only (fidelity ${record.envelope.fidelity}); it cannot be edited`,
		);
	}

	const binaryPath = await resolveBinary();
	const patchPath = join(getWorkDir(), pathKey(absolutePath), "patch.json");
	writeFileSync(patchPath, JSON.stringify(patch), "utf8");
	try {
		await runFiletools(
			binaryPath,
			"reconstruct",
			["--envelope", record.envelopePath, "--patch", patchPath, "--out", outPath, "--original", absolutePath],
			cwd,
			signal,
			options?.timeoutSecs ?? FILETOOLS_DEFAULT_TIMEOUT_SECS,
		);
	} finally {
		rmSync(patchPath, { force: true });
	}
}
