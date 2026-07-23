/**
 * Process-wide store for transient startup progress: first-run downloads of the
 * external tool binaries (fd, rg, the semantic-index engine) and the semantic
 * index build itself.
 *
 * Modeled on `taskStore` — a singleton the footer reads directly and re-renders
 * on change — but deliberately separate from it: this is short-lived startup
 * status, not agent work, so it must never render as a task-panel plan row.
 * Entries are keyed so several concurrent downloads each own a stable footer
 * line; a caller removes its key when the work settles and the line disappears.
 */

export type StartupProgress =
	| { readonly key: string; kind: "download"; label: string; receivedBytes: number; totalBytes: number | null }
	| { readonly key: string; kind: "work"; label: string; done: number; total: number; unit: string }
	| { readonly key: string; kind: "error"; label: string; message: string };

type Listener = () => void;

class StartupProgressStore {
	// Map preserves insertion order, so lines stay in the order work started
	// (fd, rg, then the index) instead of jumping as byte counts update.
	private readonly entries = new Map<string, StartupProgress>();
	private readonly listeners = new Set<Listener>();

	/** Insert or replace the entry for its key. */
	set(entry: StartupProgress): void {
		this.entries.set(entry.key, entry);
		this.emit();
	}

	/** Drop a key's line once its work settled (download done, index ready/skipped). */
	remove(key: string): void {
		if (this.entries.delete(key)) this.emit();
	}

	/** Wipe everything — called when the first user turn starts (transient startup status). */
	clear(): void {
		if (this.entries.size === 0) return;
		this.entries.clear();
		this.emit();
	}

	list(): readonly StartupProgress[] {
		return [...this.entries.values()];
	}

	subscribe(listener: Listener): () => void {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	}

	private emit(): void {
		for (const listener of this.listeners) listener();
	}
}

/** Shared, process-wide startup-progress store. */
export const startupProgress = new StartupProgressStore();
