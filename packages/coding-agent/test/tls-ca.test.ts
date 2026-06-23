import { mkdtempSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { rootCertificates } from "tls";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resolveTrustedCAs } from "../src/utils/tls-ca.js";

// A syntactically PEM-shaped string that is guaranteed not to collide with any
// bundled root certificate, so we can assert it is added (and not deduped away).
const FAKE_CA_PEM = `-----BEGIN CERTIFICATE-----\nHOOCODE_TEST_FAKE_CA_DO_NOT_TRUST\n-----END CERTIFICATE-----\n`;

describe("resolveTrustedCAs", () => {
	let tmpDir = "";
	let caPath = "";
	const savedArgv = process.argv;
	const savedEnv = {
		HOOCODE_CA_CERT: process.env.HOOCODE_CA_CERT,
		NODE_EXTRA_CA_CERTS: process.env.NODE_EXTRA_CA_CERTS,
		HOOCODE_USE_SYSTEM_CA: process.env.HOOCODE_USE_SYSTEM_CA,
	};

	beforeEach(() => {
		tmpDir = mkdtempSync(join(tmpdir(), "tls-ca-"));
		caPath = join(tmpDir, "corporate-ca.pem");
		writeFileSync(caPath, FAKE_CA_PEM);
		// Start from a clean slate: no flags, no relevant env.
		process.argv = ["node", "cli.js"];
		delete process.env.HOOCODE_CA_CERT;
		delete process.env.NODE_EXTRA_CA_CERTS;
		delete process.env.HOOCODE_USE_SYSTEM_CA;
	});

	afterEach(() => {
		process.argv = savedArgv;
		for (const [key, value] of Object.entries(savedEnv)) {
			if (value === undefined) delete process.env[key];
			else process.env[key] = value;
		}
		if (tmpDir) rmSync(tmpDir, { recursive: true, force: true });
		tmpDir = "";
	});

	it("always includes the bundled root certificates", () => {
		const cas = resolveTrustedCAs();
		expect(cas.length).toBe(rootCertificates.length);
		for (const root of rootCertificates) {
			expect(cas).toContain(root);
		}
	});

	it("merges a custom PEM from HOOCODE_CA_CERT additively without dropping defaults", () => {
		process.env.HOOCODE_CA_CERT = caPath;
		const cas = resolveTrustedCAs();
		// Every bundled root is still present...
		for (const root of rootCertificates) {
			expect(cas).toContain(root);
		}
		// ...plus the custom CA.
		expect(cas).toContain(FAKE_CA_PEM);
		expect(cas.length).toBe(rootCertificates.length + 1);
	});

	it("deduplicates when the same custom PEM is supplied via two sources", () => {
		// Precedence picks HOOCODE_CA_CERT, so the file is read once and appears once.
		process.env.HOOCODE_CA_CERT = caPath;
		process.env.NODE_EXTRA_CA_CERTS = caPath;
		const cas = resolveTrustedCAs();
		expect(cas.filter((c) => c === FAKE_CA_PEM).length).toBe(1);
		expect(cas.length).toBe(rootCertificates.length + 1);
	});

	it("falls back to NODE_EXTRA_CA_CERTS when no higher-precedence source is set", () => {
		process.env.NODE_EXTRA_CA_CERTS = caPath;
		const cas = resolveTrustedCAs();
		expect(cas).toContain(FAKE_CA_PEM);
		expect(cas.length).toBe(rootCertificates.length + 1);
	});

	it("does not throw or drop defaults when the CA file is missing", () => {
		process.env.HOOCODE_CA_CERT = join(tmpDir, "does-not-exist.pem");
		const cas = resolveTrustedCAs();
		// Bundled defaults are retained (fail closed, not trust-all).
		expect(cas.length).toBe(rootCertificates.length);
		for (const root of rootCertificates) {
			expect(cas).toContain(root);
		}
	});

	it("honors precedence: --ca-cert > HOOCODE_CA_CERT > NODE_EXTRA_CA_CERTS", () => {
		const flagPath = join(tmpDir, "flag-ca.pem");
		const flagPem = `-----BEGIN CERTIFICATE-----\nHOOCODE_TEST_FLAG_CA\n-----END CERTIFICATE-----\n`;
		writeFileSync(flagPath, flagPem);

		const otherPath = join(tmpDir, "other-ca.pem");
		const otherPem = `-----BEGIN CERTIFICATE-----\nHOOCODE_TEST_OTHER_CA\n-----END CERTIFICATE-----\n`;
		writeFileSync(otherPath, otherPem);

		process.argv = ["node", "cli.js", "--ca-cert", flagPath];
		process.env.HOOCODE_CA_CERT = caPath;
		process.env.NODE_EXTRA_CA_CERTS = otherPath;

		const cas = resolveTrustedCAs();
		// Only the highest-precedence source (--ca-cert) is used.
		expect(cas).toContain(flagPem);
		expect(cas).not.toContain(FAKE_CA_PEM);
		expect(cas).not.toContain(otherPem);
		expect(cas.length).toBe(rootCertificates.length + 1);
	});

	it("excludes the system CA store unless explicitly opted in", () => {
		// No opt-in: result is exactly the bundled roots (system store not appended).
		const withoutOptIn = resolveTrustedCAs();
		expect(withoutOptIn.length).toBe(rootCertificates.length);

		// Opt-in: never throws, and the bundled roots are still all present. The OS
		// store may overlap the bundled roots (and thus add nothing after dedupe),
		// so we only assert it is a non-shrinking superset.
		process.env.HOOCODE_USE_SYSTEM_CA = "1";
		const withOptIn = resolveTrustedCAs();
		expect(withOptIn.length).toBeGreaterThanOrEqual(rootCertificates.length);
		for (const root of rootCertificates) {
			expect(withOptIn).toContain(root);
		}
	});
});
