/**
 * Undo and redo over snapshots of some state.
 *
 * Stores deep clones on the way in; hands snapshots back directly on the way
 * out, since a popped one is already detached and nobody else holds it.
 *
 * ## Why redo is here rather than in the caller
 *
 * The invariant that makes redo correct is easy to state and easy to forget:
 * **a fresh edit invalidates every redo.** Once you undo twice and then type,
 * the future you undid never happened and offering to restore it would drop the
 * character you just typed. Keeping `push`, `undo` and `redo` in one object is
 * what lets `push` clear the redo side itself, so no caller can forget to — and
 * the caller here is an editor with twenty-odd `pushUndoSnapshot()` sites.
 */
export class UndoStack<S> {
	private stack: S[] = [];
	private redoStack: S[] = [];

	/**
	 * Record a state that can be returned to, and abandon any undone future.
	 *
	 * The clone is the point: callers hold a live state object and keep mutating
	 * it, so storing the reference would store something that changes underneath
	 * the stack.
	 */
	push(state: S): void {
		this.stack.push(structuredClone(state));
		// A new edit is a new branch. Whatever was undone is unreachable now.
		if (this.redoStack.length > 0) this.redoStack.length = 0;
	}

	/**
	 * Step back, given the state to return to if the step is later redone.
	 *
	 * `current` is asked for rather than remembered because the stack never sees
	 * the live state between pushes — the caller mutates it directly. Without it
	 * the first redo would have nothing to restore.
	 */
	undo(current: S): S | undefined {
		const snapshot = this.stack.pop();
		if (snapshot === undefined) return undefined;
		this.redoStack.push(structuredClone(current));
		return snapshot;
	}

	/** Step forward again, if an undo has left somewhere to go. */
	redo(current: S): S | undefined {
		const snapshot = this.redoStack.pop();
		if (snapshot === undefined) return undefined;
		this.stack.push(structuredClone(current));
		return snapshot;
	}

	/**
	 * Pop and return the most recent snapshot, or undefined if empty.
	 *
	 * The pre-redo entry point: it steps back without recording anywhere to
	 * step forward to. Prefer `undo`, which keeps redo working.
	 */
	pop(): S | undefined {
		return this.stack.pop();
	}

	/** Remove all snapshots, in both directions. */
	clear(): void {
		this.stack.length = 0;
		this.redoStack.length = 0;
	}

	get length(): number {
		return this.stack.length;
	}

	get canUndo(): boolean {
		return this.stack.length > 0;
	}

	get canRedo(): boolean {
		return this.redoStack.length > 0;
	}
}
