/**
 * Plan parser module for hierarchical phase/stage/step structure.
 * Parses .hoocode/plan.md files with completion tracking.
 */
export type Status = "pending" | "in-progress" | "success" | "failed" | "skipped";
export interface ChecklistItem {
    text: string;
    completed: boolean;
}
export interface Step {
    id: string;
    title: string;
    status: Status;
    checklist: ChecklistItem[];
    evidence?: string;
    completedAt?: string;
}
export interface Stage {
    id: string;
    title: string;
    steps: Step[];
}
export interface Phase {
    id: string;
    title: string;
    stages: Stage[];
}
export interface ParsedPlan {
    phases: Phase[];
}
export declare function parsePlan(content: string): ParsedPlan;
export declare function findNextPendingStep(plan: ParsedPlan): Step | null;
export declare function getStepById(plan: ParsedPlan, stepId: string): Step | null;
export declare function getCompletionStats(plan: ParsedPlan): {
    total: number;
    completed: number;
    pending: number;
    inProgress: number;
    failed: number;
    skipped: number;
};
//# sourceMappingURL=parser.d.ts.map