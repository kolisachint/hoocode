import { createHash } from "crypto";
import { existsSync, mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { Readable } from "stream";
import { afterEach, describe, expect, it, vi } from "vitest";
import { downloadFile } from "../src/utils/tools-manager.js";

// Minimal stand-in for the parts of a `fetch` Response that downloadFile reads.
interface FakeResponse {
	ok: boolean;
	status: number;
	body: ReadableStream<Uint8Array> | null;
	headers: { get(name: string): string | null };
	text(): Promise<string>;
}

function webStreamOf(buf: Buffer): ReadableStream<Uint8Array> {
	return Readable.toWeb(Readable.from([buf])) as unknown as ReadableStream<Uint8Array>;
}

function bodyResponse(buf: Buffer, contentLength: number | null): FakeResponse {
	return {
		ok: true,
		status: 200,
		body: webStreamOf(buf),
		headers: {
			get: (name) =>
				name.toLowerCase() === "content-length" && contentLength !== null ? String(contentLength) : null,
		},
		text: async () => "",
	};
}

function textResponse(status: number, text: string): FakeResponse {
	return {
		ok: status >= 200 && status < 300,
		status,
		body: null,
		headers: { get: () => null },
		text: async () => text,
	};
}

describe("downloadFile integrity guards", () => {
	let tmp = "";

	afterEach(() => {
		vi.unstubAllGlobals();
		if (tmp) rmSync(tmp, { recursive: true, force: true });
		tmp = "";
	});

	it("throws and leaves no partial file when bytes < Content-Length", async () => {
		tmp = mkdtempSync(join(tmpdir(), "tools-dl-"));
		const dest = join(tmp, "asset.tar.gz");
		const payload = Buffer.from("partial-binary-bytes");

		// Advertise more bytes than the body actually delivers (truncated transfer).
		const fetchMock = vi.fn(async (input: string | URL) => {
			const url = input.toString();
			if (url.endsWith(".sha256")) return textResponse(404, "");
			return bodyResponse(payload, payload.length + 100);
		});
		vi.stubGlobal("fetch", fetchMock);

		await expect(downloadFile("https://example.com/asset.tar.gz", dest)).rejects.toThrow(/truncated/i);
		expect(existsSync(dest)).toBe(false);
	});

	it("succeeds and writes the file when Content-Length matches and no checksum is published", async () => {
		tmp = mkdtempSync(join(tmpdir(), "tools-dl-"));
		const dest = join(tmp, "asset.tar.gz");
		const payload = Buffer.from("complete-binary-bytes");

		const fetchMock = vi.fn(async (input: string | URL) => {
			const url = input.toString();
			if (url.endsWith(".sha256")) return textResponse(404, "");
			return bodyResponse(payload, payload.length);
		});
		vi.stubGlobal("fetch", fetchMock);

		await downloadFile("https://example.com/asset.tar.gz", dest);
		expect(existsSync(dest)).toBe(true);
	});

	it("throws and leaves no file on a SHA-256 mismatch", async () => {
		tmp = mkdtempSync(join(tmpdir(), "tools-dl-"));
		const dest = join(tmp, "asset.tar.gz");
		const payload = Buffer.from("complete-binary-bytes");
		const wrongHash = "0".repeat(64);

		const fetchMock = vi.fn(async (input: string | URL) => {
			const url = input.toString();
			if (url.endsWith(".sha256")) return textResponse(200, `${wrongHash}  asset.tar.gz`);
			return bodyResponse(payload, payload.length);
		});
		vi.stubGlobal("fetch", fetchMock);

		await expect(downloadFile("https://example.com/asset.tar.gz", dest)).rejects.toThrow(/checksum mismatch/i);
		expect(existsSync(dest)).toBe(false);
	});

	it("succeeds when a published SHA-256 matches the downloaded bytes", async () => {
		tmp = mkdtempSync(join(tmpdir(), "tools-dl-"));
		const dest = join(tmp, "asset.tar.gz");
		const payload = Buffer.from("complete-binary-bytes");
		const goodHash = createHash("sha256").update(payload).digest("hex");

		const fetchMock = vi.fn(async (input: string | URL) => {
			const url = input.toString();
			if (url.endsWith(".sha256")) return textResponse(200, `${goodHash}  asset.tar.gz`);
			return bodyResponse(payload, payload.length);
		});
		vi.stubGlobal("fetch", fetchMock);

		await downloadFile("https://example.com/asset.tar.gz", dest);
		expect(existsSync(dest)).toBe(true);
	});
});
