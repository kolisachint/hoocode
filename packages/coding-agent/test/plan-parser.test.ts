import { describe, expect, it } from "vitest";
import {
	findNextPendingStep,
	getCompletionStats,
	getStepById,
	type ParsedPlan,
	parsePlan,
} from "../src/utils/plan/parser.js";

const samplePlanMd = `# Plan: Sample Test Plan

## Phase 1: Setup

### Stage 1.1: Dependencies

- [x] Step 1.1.1: Install required packages
  - Status: success
  - Completed: 2024-01-15T10:30:00Z
  - Checklist:
    - [x] Package A installed
    - [x] Package B installed
  - Evidence: package.json updated

- [ ] Step 1.1.2: Configure environment
  - Status: pending
  - Checklist:
    - [ ] Env file created
    - [ ] Database connected

### Stage 1.2: Initialization

- [ ] Step 1.2.1: Initialize database
  - Status: pending
  - Evidence: migrations run

- [x] Step 1.2.2: Seed initial data
  - Status: success
  - Completed: 2024-01-15T11:00:00Z

## Phase 2: Implementation

### Stage 2.1: Core Logic

- [ ] Step 2.1.1: Create API endpoints
  - Status: in-progress
  - Checklist:
    - [x] GET /api/users
    - [ ] POST /api/users
    - [ ] PUT /api/users/:id

### Stage 2.2: Integration

- [ ] Step 2.2.1: Connect frontend
  - Status: pending

- [ ] Step 2.2.2: Add authentication
  - Status: failed
  - Evidence: OAuth configuration missing

## Phase 3: Testing

### Stage 3.1: Unit Tests

- [ ] Step 3.1.1: Write unit tests for utils
  - Status: skipped

- [ ] Step 3.1.2: Write unit tests for services
  - Status: pending
  - Checklist:
    - [ ] User service tests
    - [ ] Auth service tests

### Stage 3.2: Integration Tests

- [x] Step 3.2.1: Setup test environment
  - Status: success
  - Completed: 2024-01-16T09:00:00Z
  - Evidence: test database ready

- [ ] Step 3.2.2: Run E2E tests
  - Status: pending
`;

describe("parsePlan", () => {
	it("parses a sample plan.md with 3 phases correctly", () => {
		const plan = parsePlan(samplePlanMd);

		// Verify structure
		expect(plan.phases).toHaveLength(3);

		// Phase 1
		expect(plan.phases[0].id).toBe("1");
		expect(plan.phases[0].title).toBe("Setup");
		expect(plan.phases[0].stages).toHaveLength(2);

		// Stage 1.1
		expect(plan.phases[0].stages[0].id).toBe("1.1");
		expect(plan.phases[0].stages[0].title).toBe("Dependencies");
		expect(plan.phases[0].stages[0].steps).toHaveLength(2);

		// Step 1.1.1 (completed)
		expect(plan.phases[0].stages[0].steps[0].id).toBe("1.1.1");
		expect(plan.phases[0].stages[0].steps[0].title).toBe("Install required packages");
		expect(plan.phases[0].stages[0].steps[0].status).toBe("success");
		expect(plan.phases[0].stages[0].steps[0].completedAt).toBe("2024-01-15T10:30:00Z");
		expect(plan.phases[0].stages[0].steps[0].evidence).toBe("package.json updated");
		expect(plan.phases[0].stages[0].steps[0].checklist).toHaveLength(2);
		expect(plan.phases[0].stages[0].steps[0].checklist[0].text).toBe("Package A installed");
		expect(plan.phases[0].stages[0].steps[0].checklist[0].completed).toBe(true);
		expect(plan.phases[0].stages[0].steps[0].checklist[1].text).toBe("Package B installed");
		expect(plan.phases[0].stages[0].steps[0].checklist[1].completed).toBe(true);

		// Step 1.1.2 (pending)
		expect(plan.phases[0].stages[0].steps[1].id).toBe("1.1.2");
		expect(plan.phases[0].stages[0].steps[1].title).toBe("Configure environment");
		expect(plan.phases[0].stages[0].steps[1].status).toBe("pending");
		expect(plan.phases[0].stages[0].steps[1].checklist).toHaveLength(2);
		expect(plan.phases[0].stages[0].steps[1].checklist[0].completed).toBe(false);

		// Stage 1.2
		expect(plan.phases[0].stages[1].id).toBe("1.2");
		expect(plan.phases[0].stages[1].title).toBe("Initialization");
		expect(plan.phases[0].stages[1].steps).toHaveLength(2);

		// Phase 2
		expect(plan.phases[1].id).toBe("2");
		expect(plan.phases[1].title).toBe("Implementation");
		expect(plan.phases[1].stages).toHaveLength(2);

		// Stage 2.1
		expect(plan.phases[1].stages[0].steps[0].id).toBe("2.1.1");
		expect(plan.phases[1].stages[0].steps[0].status).toBe("in-progress");
		expect(plan.phases[1].stages[0].steps[0].checklist).toHaveLength(3);
		expect(plan.phases[1].stages[0].steps[0].checklist[0].completed).toBe(true);
		expect(plan.phases[1].stages[0].steps[0].checklist[1].completed).toBe(false);

		// Step 2.2.2 (failed)
		expect(plan.phases[1].stages[1].steps[1].id).toBe("2.2.2");
		expect(plan.phases[1].stages[1].steps[1].status).toBe("failed");
		expect(plan.phases[1].stages[1].steps[1].evidence).toBe("OAuth configuration missing");

		// Phase 3
		expect(plan.phases[2].id).toBe("3");
		expect(plan.phases[2].title).toBe("Testing");
		expect(plan.phases[2].stages).toHaveLength(2);

		// Step 3.1.1 (skipped)
		expect(plan.phases[2].stages[0].steps[0].id).toBe("3.1.1");
		expect(plan.phases[2].stages[0].steps[0].status).toBe("skipped");
	});

	it("handles empty plan", () => {
		const plan = parsePlan("");
		expect(plan.phases).toHaveLength(0);
	});

	it("handles plan with only phases and no stages", () => {
		const content = `## Phase 1: Empty
Some content here

## Phase 2: Also Empty
More content`;
		const plan = parsePlan(content);
		expect(plan.phases).toHaveLength(2);
		expect(plan.phases[0].stages).toHaveLength(0);
		expect(plan.phases[1].stages).toHaveLength(0);
	});

	it("parses status case-insensitively", () => {
		const content = `## Phase 1: Test
### Stage 1.1: Test
- [ ] Step 1.1.1: Test step
  - Status: IN-PROGRESS
- [ ] Step 1.1.2: Test step 2
  - Status: SUCCESS
- [ ] Step 1.1.3: Test step 3
  - Status: FAILED
- [ ] Step 1.1.4: Test step 4
  - Status: SKIPPED
- [ ] Step 1.1.5: Test step 5
  - Status: UNKNOWN`;

		const plan = parsePlan(content);
		expect(plan.phases[0].stages[0].steps[0].status).toBe("in-progress");
		expect(plan.phases[0].stages[0].steps[1].status).toBe("success");
		expect(plan.phases[0].stages[0].steps[2].status).toBe("failed");
		expect(plan.phases[0].stages[0].steps[3].status).toBe("skipped");
		expect(plan.phases[0].stages[0].steps[4].status).toBe("pending");
	});
});

describe("findNextPendingStep", () => {
	it("returns first in-progress step before pending steps", () => {
		const plan = parsePlan(samplePlanMd);
		const nextStep = findNextPendingStep(plan);

		expect(nextStep).not.toBeNull();
		expect(nextStep?.id).toBe("2.1.1");
		expect(nextStep?.status).toBe("in-progress");
	});

	it("returns null when all steps are completed", () => {
		const content = `## Phase 1: Test
### Stage 1.1: Test
- [x] Step 1.1.1: Test step
  - Status: success`;

		const plan = parsePlan(content);
		const nextStep = findNextPendingStep(plan);

		expect(nextStep).toBeNull();
	});

	it("returns first pending step when no in-progress steps", () => {
		const content = `## Phase 1: Test
### Stage 1.1: Test
- [x] Step 1.1.1: Completed step
  - Status: success
- [ ] Step 1.1.2: Pending step
  - Status: pending`;

		const plan = parsePlan(content);
		const nextStep = findNextPendingStep(plan);

		expect(nextStep?.id).toBe("1.1.2");
	});
});

describe("getStepById", () => {
	it("finds step by ID", () => {
		const plan = parsePlan(samplePlanMd);

		const step = getStepById(plan, "2.2.2");
		expect(step).not.toBeNull();
		expect(step?.id).toBe("2.2.2");
		expect(step?.title).toBe("Add authentication");
	});

	it("returns null for non-existent step", () => {
		const plan = parsePlan(samplePlanMd);

		const step = getStepById(plan, "99.99.99");
		expect(step).toBeNull();
	});
});

describe("getCompletionStats", () => {
	it("returns correct statistics for sample plan", () => {
		const plan = parsePlan(samplePlanMd);
		const stats = getCompletionStats(plan);

		// Total steps in sample plan:
		// Phase 1: 2 + 2 = 4
		// Phase 2: 1 + 2 = 3
		// Phase 3: 2 + 2 = 4
		// Total: 11
		expect(stats.total).toBe(11);
		expect(stats.completed).toBe(3); // 1.1.1, 1.2.2, 3.2.1
		expect(stats.inProgress).toBe(1); // 2.1.1
		expect(stats.failed).toBe(1); // 2.2.2
		expect(stats.skipped).toBe(1); // 3.1.1
		expect(stats.pending).toBe(5); // Remaining
	});

	it("returns zero stats for empty plan", () => {
		const plan: ParsedPlan = { phases: [] };
		const stats = getCompletionStats(plan);

		expect(stats.total).toBe(0);
		expect(stats.completed).toBe(0);
		expect(stats.pending).toBe(0);
		expect(stats.inProgress).toBe(0);
		expect(stats.failed).toBe(0);
		expect(stats.skipped).toBe(0);
	});
});
