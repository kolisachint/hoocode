/**
 * Plan parser module for hierarchical phase/stage/step structure.
 * Parses .hoocode/plan.md files with completion tracking.
 */
const PHASE_REGEX = /^##\s+Phase\s+(\d+):\s+(.+)$/i;
const STAGE_REGEX = /^###\s+Stage\s+(\d+\.\d+):\s+(.+)$/i;
const STEP_REGEX = /^-\s+\[([ xX])\]\s+Step\s+(\d+\.\d+\.\d+):\s+(.+)$/i;
const STATUS_REGEX = /^\s+-\s+Status:\s*(\S+)/i;
const CHECKLIST_REGEX = /^\s+-\s+\[([ xX])\]\s+(.+)$/i;
const EVIDENCE_REGEX = /^\s+-\s+Evidence:\s*(.+)$/i;
const COMPLETED_REGEX = /^\s+-\s+Completed:\s*(.+)$/i;
function parseStatus(status) {
    switch (status.toLowerCase()) {
        case "in-progress":
            return "in-progress";
        case "success":
            return "success";
        case "failed":
            return "failed";
        case "skipped":
            return "skipped";
        default:
            return "pending";
    }
}
export function parsePlan(content) {
    const lines = content.split("\n");
    const plan = { phases: [] };
    let currentPhase = null;
    let currentStage = null;
    let currentStep = null;
    let inChecklist = false;
    for (const line of lines) {
        // Check for phase header
        const phaseMatch = line.match(PHASE_REGEX);
        if (phaseMatch) {
            currentPhase = {
                id: phaseMatch[1],
                title: phaseMatch[2].trim(),
                stages: [],
            };
            plan.phases.push(currentPhase);
            currentStage = null;
            currentStep = null;
            inChecklist = false;
            continue;
        }
        // Check for stage header
        const stageMatch = line.match(STAGE_REGEX);
        if (stageMatch && currentPhase) {
            currentStage = {
                id: stageMatch[1],
                title: stageMatch[2].trim(),
                steps: [],
            };
            currentPhase.stages.push(currentStage);
            currentStep = null;
            inChecklist = false;
            continue;
        }
        // Check for step header
        const stepMatch = line.match(STEP_REGEX);
        if (stepMatch && currentStage) {
            currentStep = {
                id: stepMatch[2],
                title: stepMatch[3].trim(),
                status: stepMatch[1].trim() !== "" ? "success" : "pending",
                checklist: [],
            };
            currentStage.steps.push(currentStep);
            inChecklist = false;
            continue;
        }
        // If no current step, skip further processing
        if (!currentStep)
            continue;
        // Check for status
        const statusMatch = line.match(STATUS_REGEX);
        if (statusMatch) {
            currentStep.status = parseStatus(statusMatch[1]);
            inChecklist = false;
            continue;
        }
        // Check for completed timestamp
        const completedMatch = line.match(COMPLETED_REGEX);
        if (completedMatch) {
            currentStep.completedAt = completedMatch[1].trim();
            continue;
        }
        // Check for evidence
        const evidenceMatch = line.match(EVIDENCE_REGEX);
        if (evidenceMatch) {
            currentStep.evidence = evidenceMatch[1].trim();
            continue;
        }
        // Check for checklist start
        if (line.match(/^\s+-\s+Checklist:\s*$/i)) {
            inChecklist = true;
            continue;
        }
        // Check for checklist items
        if (inChecklist) {
            const checklistMatch = line.match(CHECKLIST_REGEX);
            if (checklistMatch) {
                currentStep.checklist.push({
                    text: checklistMatch[2].trim(),
                    completed: checklistMatch[1].trim() !== "",
                });
            }
        }
    }
    return plan;
}
export function findNextPendingStep(plan) {
    // First, look for any in-progress steps (to resume after crash)
    for (const phase of plan.phases) {
        for (const stage of phase.stages) {
            for (const step of stage.steps) {
                if (step.status === "in-progress") {
                    return step;
                }
            }
        }
    }
    // Then, look for pending steps
    for (const phase of plan.phases) {
        for (const stage of phase.stages) {
            for (const step of stage.steps) {
                if (step.status === "pending") {
                    return step;
                }
            }
        }
    }
    return null;
}
export function getStepById(plan, stepId) {
    for (const phase of plan.phases) {
        for (const stage of phase.stages) {
            for (const step of stage.steps) {
                if (step.id === stepId) {
                    return step;
                }
            }
        }
    }
    return null;
}
export function getCompletionStats(plan) {
    const stats = {
        total: 0,
        completed: 0,
        pending: 0,
        inProgress: 0,
        failed: 0,
        skipped: 0,
    };
    for (const phase of plan.phases) {
        for (const stage of phase.stages) {
            for (const step of stage.steps) {
                stats.total++;
                switch (step.status) {
                    case "success":
                        stats.completed++;
                        break;
                    case "pending":
                        stats.pending++;
                        break;
                    case "in-progress":
                        stats.inProgress++;
                        break;
                    case "failed":
                        stats.failed++;
                        break;
                    case "skipped":
                        stats.skipped++;
                        break;
                }
            }
        }
    }
    return stats;
}
//# sourceMappingURL=parser.js.map