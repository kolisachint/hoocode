import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { createDocEditTool } from "../src/core/tools/docedit.js";
import { createDocReadTool } from "../src/core/tools/docread.js";
import { createDocWriteTool } from "../src/core/tools/docwrite.js";
import { invalidateExtractRecord } from "../src/core/tools/filetools-shared.js";

// A fake `filetools` binary placed on PATH. It mimics the real CLI's file-based
// contract: `extract` writes the envelope JSON to --out (plus a sidecar id-map
// next to it) and a status line to stderr; `reconstruct` writes the output file
// to --out. No real Rust binary or document parsing is needed to exercise the
// wrapper's spawn + file-readback + caching paths.
const FAKE_BIN = `#!/bin/sh
sub="$1"; shift
out=""; input=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="$2"; shift 2;;
    --input) input="$2"; shift 2;;
    --envelope|--patch|--original) shift 2;;
    --readonly) shift;;
    *) shift;;
  esac
done
if [ -n "$FILETOOLS_ARGV_LOG" ]; then echo "$sub $@" >> "$FILETOOLS_ARGV_LOG"; fi
if [ "$sub" = "extract" ]; then
  cat > "$out" <<JSON
{
  "version": "1.0",
  "source": { "path": "$input", "type": "xml", "hash": "sha256:abc" },
  "fidelity": "lossless",
  "writable": true,
  "idmap_ref": "envelope.idmap.json",
  "structure": [
    { "id": "n1", "tag": "title", "text": "Hello" },
    { "id": "n2", "tag": "body", "attrs": [{ "name": "class", "value": "main" }], "text": "World" }
  ]
}
JSON
  dir=$(dirname "$out")
  echo '{ "for_hash": "sha256:abc", "map": {} }' > "$dir/envelope.idmap.json"
  echo "extracted $input -> $out [Lossless, 2 nodes]" 1>&2
elif [ "$sub" = "reconstruct" ]; then
  printf '<doc><title>Patched</title></doc>' > "$out"
  echo "reconstructed -> $out" 1>&2
else
  echo "unknown subcommand" 1>&2; exit 1
fi
`;

function getText(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content
		.filter((c) => c.type === "text")
		.map((c) => c.text)
		.join("\n");
}

describe("document tools", () => {
	let binDir: string;
	let cwd: string;
	let originalPath: string | undefined;

	beforeAll(() => {
		binDir = join(tmpdir(), `filetools-fake-bin-${Date.now()}`);
		mkdirSync(binDir, { recursive: true });
		const binPath = join(binDir, "filetools");
		writeFileSync(binPath, FAKE_BIN);
		chmodSync(binPath, 0o755);
		originalPath = process.env.PATH;
		process.env.PATH = `${binDir}:${process.env.PATH ?? ""}`;
	});

	afterAll(() => {
		process.env.PATH = originalPath;
		rmSync(binDir, { recursive: true, force: true });
	});

	beforeEach(() => {
		cwd = join(tmpdir(), `filetools-cwd-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	describe("DocRead", () => {
		it("extracts a document into id-addressed text", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			const tool = createDocReadTool(cwd);
			const result = await tool.execute("r1", { path: docPath });
			const text = getText(result);
			expect(text).toContain("#n1 <title>");
			expect(text).toContain('#n2 <body class="main">');
			expect(text).toContain("Hello");
			const details = result.details as { type?: string; writable?: boolean; nodeCount?: number };
			expect(details.type).toBe("xml");
			expect(details.writable).toBe(true);
			expect(details.nodeCount).toBe(2);
		});
	});

	describe("DocEdit", () => {
		it("requires a prior DocRead", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			invalidateExtractRecord(docPath);
			const tool = createDocEditTool(cwd);
			await expect(
				tool.execute("e1", { path: docPath, patch: [{ op: "replace", path: "/structure/n1/text", value: "Hi" }] }),
			).rejects.toThrow(/DocRead/);
		});

		it("applies a patch in place after a DocRead", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			await createDocReadTool(cwd).execute("r2", { path: docPath });
			const result = await createDocEditTool(cwd).execute("e2", {
				path: docPath,
				patch: [{ op: "replace", path: "/structure/n1/text", value: "Patched" }],
			});
			expect((result.details as { ops?: number }).ops).toBe(1);
			expect(readFileSync(docPath, "utf8")).toContain("Patched");
		});
	});

	describe("DocWrite", () => {
		it("reconstructs to a new path leaving the source untouched", async () => {
			const docPath = join(cwd, "report.xml");
			const outPath = join(cwd, "report_v2.xml");
			const original = "<doc><title>Hello</title></doc>";
			writeFileSync(docPath, original);
			await createDocReadTool(cwd).execute("r3", { path: docPath });
			const result = await createDocWriteTool(cwd).execute("w1", {
				path: docPath,
				out: outPath,
				patch: [{ op: "replace", path: "/structure/n1/text", value: "Patched" }],
			});
			const details = result.details as { ops?: number; out?: string };
			expect(details.ops).toBe(1);
			expect(existsSync(outPath)).toBe(true);
			expect(readFileSync(outPath, "utf8")).toContain("Patched");
			// Source left unchanged.
			expect(readFileSync(docPath, "utf8")).toBe(original);
		});
	});
});
