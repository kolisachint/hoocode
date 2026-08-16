import { describe, expect, it } from "vitest";
import {
	type ClusterInput,
	fallbackLabel,
	parseClusterLabels,
	renderClusterRequest,
	trimVocabulary,
} from "../src/core/learn/cluster.js";

function inputs(...texts: string[]): ClusterInput[] {
	return texts.map((text, id) => ({ id, kind: "directive" as const, text }));
}

describe("renderClusterRequest", () => {
	it("numbers every item, since the answer comes back by number", () => {
		const rendered = renderClusterRequest(inputs("we're on bun now", "stop using npm"), []);
		expect(rendered).toContain("0. [directive] we're on bun now");
		expect(rendered).toContain("1. [directive] stop using npm");
	});

	it("offers the labels already on record, which is what keeps suppression working", () => {
		const rendered = renderClusterRequest(inputs("we're on bun now"), ["use-bun-not-npm"]);
		expect(rendered).toContain("KNOWN LABELS");
		expect(rendered).toContain("- use-bun-not-npm");
	});

	it("omits the vocabulary section entirely on a first run", () => {
		expect(renderClusterRequest(inputs("x"), [])).not.toContain("KNOWN LABELS");
	});

	it("keeps names invented earlier in the run, not just the ones on record", () => {
		// The list is state labels first, then names coined in earlier batches. A
		// plain prefix drops the second group precisely when a window is big enough
		// to be split, which is the only time it matters.
		const onRecord = Array.from({ length: 200 }, (_, i) => `known-${i}`);
		const rendered = renderClusterRequest(inputs("x"), [...onRecord, "coined-this-run"]);
		expect(rendered).toContain("- coined-this-run");
		expect(rendered).toContain("- known-0");
	});

	it("flattens a quote to one line, so item numbering survives a multi-line directive", () => {
		const rendered = renderClusterRequest(inputs("use tabs\nnot spaces"), []);
		expect(rendered).toContain("0. [directive] use tabs not spaces");
		expect(rendered.split("\n").filter((line) => line.startsWith("0."))).toHaveLength(1);
	});

	it("marks the kind, so a directive is never merged with a request", () => {
		const rendered = renderClusterRequest([{ id: 7, kind: "request", text: "scaffold then register" }], []);
		expect(rendered).toContain("7. [request] scaffold then register");
	});
});

describe("parseClusterLabels", () => {
	const known = new Set([1, 2]);

	it("reads a well-formed assignment", () => {
		const labels = parseClusterLabels(
			'{"labels":[{"id":1,"label":"use-bun-not-npm"},{"id":2,"label":"use-bun-not-npm"}]}',
			known,
		);
		expect(labels.get(1)).toBe("use-bun-not-npm");
		expect(labels.get(2)).toBe("use-bun-not-npm");
	});

	it("tolerates a fenced response", () => {
		const labels = parseClusterLabels('```json\n{"labels":[{"id":1,"label":"a-b"}]}\n```', known);
		expect(labels.get(1)).toBe("a-b");
	});

	it("normalizes to the slug shape the state file keys on", () => {
		// Otherwise "Use Bun Not Npm" and "use-bun-not-npm" are two clusters, and
		// the bookmark from last run matches neither.
		const labels = parseClusterLabels('{"labels":[{"id":1,"label":"Use Bun, Not Npm!"}]}', known);
		expect(labels.get(1)).toBe("use-bun-not-npm");
	});

	it("ignores an id nobody asked about", () => {
		const labels = parseClusterLabels('{"labels":[{"id":99,"label":"invented"}]}', known);
		expect(labels.size).toBe(0);
	});

	it("ignores an entry with no usable label", () => {
		const labels = parseClusterLabels('{"labels":[{"id":1,"label":"   "},{"id":2}]}', known);
		expect(labels.size).toBe(0);
	});

	it("returns nothing rather than throwing on unparseable output", () => {
		expect(parseClusterLabels("the model refused", known).size).toBe(0);
		expect(parseClusterLabels("{ not json", known).size).toBe(0);
	});
});

describe("trimVocabulary", () => {
	it("leaves a vocabulary that already fits alone", () => {
		expect(trimVocabulary(["a", "b"], 4)).toEqual(["a", "b"]);
	});

	it("keeps both ends when it does not fit, never a bare prefix", () => {
		expect(trimVocabulary(["a", "b", "c", "d", "e", "f"], 4)).toEqual(["a", "b", "e", "f"]);
	});
});

describe("fallbackLabel", () => {
	it("names a candidate after its own wording when clustering is unavailable", () => {
		// The floor to fail to: identical sentences still group, nothing else does,
		// which is what the pipeline did before a naming pass existed.
		expect(fallbackLabel("Always use bun!")).toBe("always-use-bun");
		expect(fallbackLabel("always use bun")).toBe(fallbackLabel("Always  use  bun"));
	});

	it("never returns an empty key, which would collapse unrelated items into one", () => {
		expect(fallbackLabel("!!!")).toBe("unlabelled");
	});
});
