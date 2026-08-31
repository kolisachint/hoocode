import { describe, expect, it } from "vitest";
import { sessionColorSlotFor, sessionSlugFor } from "../../src/core/session-identity.js";
import { SessionManager } from "../../src/core/session-manager.js";

describe("SessionManager session info", () => {
	it("wears an auto-assigned name and colour before anyone sets one", () => {
		const session = SessionManager.inMemory();
		const id = session.getSessionId();

		expect(session.getSessionName()).toBeUndefined();
		expect(session.getSessionSlug()).toBe(sessionSlugFor(id));
		expect(session.getDisplayName()).toBe(sessionSlugFor(id));
		expect(session.getSessionColorSlot()).toBe(sessionColorSlotFor(id));
	});

	it("prefers the chosen name over the slug", () => {
		const session = SessionManager.inMemory();

		session.appendSessionInfo({ name: "refactor-auth" });

		expect(session.getSessionName()).toBe("refactor-auth");
		expect(session.getDisplayName()).toBe("refactor-auth");
		// The slug is still there underneath, unchanged by the rename.
		expect(session.getSessionSlug()).toBe(sessionSlugFor(session.getSessionId()));
	});

	// The two fields share one entry type, so a naive "latest entry wins" read
	// would let `/color` write an entry with no name and silently unname the
	// session — and a later `/name` would drop the colour the same way.
	it("keeps the name and the colour from clearing each other", () => {
		const session = SessionManager.inMemory();

		session.appendSessionInfo({ name: "refactor-auth" });
		session.appendSessionInfo({ color: 4 });

		expect(session.getSessionName()).toBe("refactor-auth");
		expect(session.getSessionColorSlot()).toBe(4);

		session.appendSessionInfo({ name: "parser-spike" });

		expect(session.getSessionName()).toBe("parser-spike");
		expect(session.getSessionColorSlot()).toBe(4);
	});

	it("takes the latest value of each field", () => {
		const session = SessionManager.inMemory();

		session.appendSessionInfo({ name: "first", color: 2 });
		session.appendSessionInfo({ color: 5 });
		session.appendSessionInfo({ name: "second" });

		expect(session.getSessionName()).toBe("second");
		expect(session.getSessionColorSlot()).toBe(5);
	});

	it("still lets an empty name clear the name explicitly", () => {
		const session = SessionManager.inMemory();

		session.appendSessionInfo({ name: "refactor-auth" });
		session.appendSessionInfo({ name: "" });

		expect(session.getSessionName()).toBeUndefined();
		// Cleared back to auto, not to nothing: there is always something to show.
		expect(session.getDisplayName()).toBe(session.getSessionSlug());
	});

	it("ignores a colour outside the palette rather than rendering nothing", () => {
		const session = SessionManager.inMemory();

		session.appendSessionInfo({ color: 99 });

		expect(session.getSessionColorSlot()).toBe(sessionColorSlotFor(session.getSessionId()));
	});

	// The branch is recorded rather than derived because it cannot be recovered
	// later: by the time anyone reads the session list, the working tree has moved
	// on. Unlike a guessed title it is a fact, which is what makes it safe to
	// write down and still true a month afterwards.
	it("records the branch it started on, or nothing outside a repo", () => {
		const session = SessionManager.inMemory();
		const branch = session.getSessionBranch();

		// The suite runs inside this repo, so there is a branch to record; the
		// assertion that matters is that it is a real name, never "detached".
		expect(branch === undefined || (typeof branch === "string" && branch.length > 0)).toBe(true);
		expect(branch).not.toBe("detached");
	});

	it("writes only the fields it was given", () => {
		const session = SessionManager.inMemory();

		session.appendSessionInfo({ color: 3 });

		const entry = session.getEntries().find((e) => e.type === "session_info");
		expect(entry).toBeDefined();
		expect(entry && "name" in entry).toBe(false);
	});
});
