import { describe, expect, it, vi } from "vitest";
import { SessionImportFileNotFoundError } from "../src/core/agent-session-runtime.js";
import { type CommandContext, CommandExecutor } from "../src/modes/interactive/command-executor.js";

type PathCommand = "/export" | "/import";

type ImportCommandContext = {
	stopLoadingAnimation: () => void;
	statusContainer: { clear: () => void };
	runtimeHost: { importFromJsonl: (inputPath: string, cwdOverride?: string) => Promise<{ cancelled: boolean }> };
	showError: (message: string) => void;
	showStatus: (message: string) => void;
	showExtensionConfirm: (title: string, message: string) => Promise<boolean>;
	renderCurrentSessionState: () => void;
	handleFatalRuntimeError: (prefix: string, error: unknown) => Promise<never>;
	promptForMissingSessionCwd: (error: unknown) => Promise<string | undefined>;
};

// getPathArgument is a pure helper on CommandExecutor (does not use `this`).
const getPathArgument = (text: string, command: PathCommand): string | undefined =>
	(
		CommandExecutor.prototype as unknown as {
			getPathArgument(text: string, command: PathCommand): string | undefined;
		}
	).getPathArgument(text, command);

function makeExecutor(context: ImportCommandContext): CommandExecutor {
	return new CommandExecutor(context as unknown as CommandContext);
}

function createImportContext(overrides: Partial<ImportCommandContext> = {}): ImportCommandContext {
	return {
		stopLoadingAnimation: vi.fn(),
		statusContainer: { clear: vi.fn() },
		runtimeHost: { importFromJsonl: vi.fn(async () => ({ cancelled: false })) },
		showError: vi.fn(),
		showStatus: vi.fn(),
		showExtensionConfirm: vi.fn(async () => true),
		renderCurrentSessionState: vi.fn(),
		handleFatalRuntimeError: vi.fn(async () => {
			throw new Error("unexpected fatal error");
		}),
		promptForMissingSessionCwd: vi.fn(async () => undefined),
		...overrides,
	};
}

describe("CommandExecutor /import parsing", () => {
	it("strips quotes from /import path arguments", () => {
		expect(getPathArgument('/import "path/to/session.jsonl"', "/import")).toBe("path/to/session.jsonl");
		expect(getPathArgument('/import "path with spaces/session.jsonl"', "/import")).toBe(
			"path with spaces/session.jsonl",
		);
	});

	it("preserves apostrophes in unquoted /import path arguments", () => {
		expect(getPathArgument("/import john's/session.jsonl", "/import")).toBe("john's/session.jsonl");
	});

	it("enforces command token boundaries", () => {
		expect(getPathArgument("/important /tmp/session.jsonl", "/import")).toBe(undefined);
		expect(getPathArgument("/exporter out.html", "/export")).toBe(undefined);
		expect(getPathArgument("/import /tmp/session.jsonl", "/import")).toBe("/tmp/session.jsonl");
	});

	it("passes unquoted path to runtimeHost.importFromJsonl", async () => {
		const importFromJsonl = vi.fn(async () => ({ cancelled: false }));
		const showExtensionConfirm = vi.fn(async () => true);
		const showStatus = vi.fn();
		const showError = vi.fn();

		const context = createImportContext({
			runtimeHost: { importFromJsonl },
			showError,
			showStatus,
			showExtensionConfirm,
		});

		await makeExecutor(context).handleImport('/import "path/to/session.jsonl"');

		expect(showExtensionConfirm).toHaveBeenCalledWith(
			"Import session",
			"Replace current session with path/to/session.jsonl?",
		);
		expect(importFromJsonl).toHaveBeenCalledWith("path/to/session.jsonl");
		expect(showError).not.toHaveBeenCalled();
		expect(showStatus).toHaveBeenCalledWith("Session imported from: path/to/session.jsonl");
	});

	it("passes unquoted apostrophe path to runtimeHost.importFromJsonl unchanged", async () => {
		const importFromJsonl = vi.fn(async () => ({ cancelled: false }));
		const showStatus = vi.fn();
		const showError = vi.fn();

		const context = createImportContext({
			runtimeHost: { importFromJsonl },
			showError,
			showStatus,
		});

		await makeExecutor(context).handleImport("/import john's/session.jsonl");

		expect(importFromJsonl).toHaveBeenCalledWith("john's/session.jsonl");
		expect(showError).not.toHaveBeenCalled();
		expect(showStatus).toHaveBeenCalledWith("Session imported from: john's/session.jsonl");
	});

	it("shows a non-fatal error when /import path does not exist", async () => {
		const importFromJsonl = vi.fn(async () => {
			throw new SessionImportFileNotFoundError("/tmp/missing-session.jsonl");
		});
		const showStatus = vi.fn();
		const showError = vi.fn();
		const handleFatalRuntimeError = vi.fn(async () => {
			throw new Error("unexpected fatal error");
		});

		const context = createImportContext({
			runtimeHost: { importFromJsonl },
			showError,
			showStatus,
			handleFatalRuntimeError,
		});

		await makeExecutor(context).handleImport("/import /tmp/missing-session.jsonl");

		expect(showError).toHaveBeenCalledWith("Failed to import session: File not found: /tmp/missing-session.jsonl");
		expect(showStatus).not.toHaveBeenCalled();
		expect(handleFatalRuntimeError).not.toHaveBeenCalled();
	});
});
