/**
 * Structured document tools (DocRead/DocEdit/DocWrite/DocScan/DocGrep/DocPeek),
 * backed by the `filetools` document engine. See docs/doc-tools-flow.md.
 */

export {
	createDocEditToolDefinition,
	type DocEditToolOptions,
} from "./docedit.js";
export {
	createDocGrepToolDefinition,
	type DocGrepToolOptions,
} from "./docgrep.js";
export {
	createDocPeekToolDefinition,
	type DocPeekToolOptions,
} from "./docpeek.js";
export {
	createDocReadToolDefinition,
	type DocReadToolOptions,
} from "./docread.js";
export {
	createDocScanToolDefinition,
	type DocScanToolOptions,
} from "./docscan.js";
export {
	createDocWriteToolDefinition,
	type DocWriteToolOptions,
} from "./docwrite.js";
