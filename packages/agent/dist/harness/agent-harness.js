import { Agent } from "../agent.js";
import { collectEntriesForBranchSummary, generateBranchSummary } from "./compaction/branch-summarization.js";
import { compact, DEFAULT_COMPACTION_SETTINGS, prepareCompaction } from "./compaction/compaction.js";
import { formatPromptTemplateInvocation } from "./prompt-templates.js";
import { formatSkillInvocation } from "./skills.js";
function createUserMessage(text, images) {
    const content = [{ type: "text", text }];
    if (images)
        content.push(...images);
    return { role: "user", content, timestamp: Date.now() };
}
export class AgentHarness {
    agent;
    env;
    session;
    model;
    thinkingLevel;
    activeToolNames;
    nextTurnQueue = [];
    phase = "idle";
    steerQueue = [];
    followUpQueue = [];
    pendingSessionWrites = [];
    resources;
    systemPrompt;
    getApiKeyAndHeaders;
    tools = new Map();
    listeners = new Set();
    hooks = new Map();
    constructor(options) {
        this.agent = new Agent({
            initialState: {
                model: options.model,
                thinkingLevel: options.thinkingLevel,
                tools: options.tools ?? [],
            },
            steeringMode: options.steeringMode,
            followUpMode: options.followUpMode,
        });
        this.env = options.env;
        this.session = options.session;
        this.resources = options.resources ?? {};
        this.systemPrompt = options.systemPrompt;
        this.getApiKeyAndHeaders = options.getApiKeyAndHeaders;
        for (const tool of options.tools ?? []) {
            this.tools.set(tool.name, tool);
        }
        this.model = options.model;
        this.thinkingLevel = options.thinkingLevel ?? this.agent.state.thinkingLevel;
        this.activeToolNames = options.activeToolNames ?? (options.tools ?? []).map((tool) => tool.name);
        this.agent.state.model = this.model;
        this.agent.state.thinkingLevel = this.thinkingLevel;
        this.agent.getApiKey = async (provider) => {
            const model = this.model;
            if (!this.getApiKeyAndHeaders || model.provider !== provider)
                return undefined;
            return (await this.getApiKeyAndHeaders(model))?.apiKey;
        };
        this.agent.transformContext = async (messages) => {
            const result = await this.emitHook({ type: "context", messages: [...messages] });
            return result?.messages ?? messages;
        };
        this.agent.beforeToolCall = async ({ toolCall, args }) => {
            const result = await this.emitHook({
                type: "tool_call",
                toolCallId: toolCall.id,
                toolName: toolCall.name,
                input: args,
            });
            return result ? { block: result.block, reason: result.reason } : undefined;
        };
        this.agent.afterToolCall = async ({ toolCall, args, result, isError }) => {
            const patch = await this.emitHook({
                type: "tool_result",
                toolCallId: toolCall.id,
                toolName: toolCall.name,
                input: args,
                content: result.content,
                details: result.details,
                isError,
            });
            return patch
                ? { content: patch.content, details: patch.details, isError: patch.isError, terminate: patch.terminate }
                : undefined;
        };
        this.agent.onPayload = async (payload) => {
            const result = await this.emitHook({ type: "before_provider_request", payload });
            return result?.payload ?? payload;
        };
        this.agent.onResponse = async (response) => {
            const headers = { ...response.headers };
            await this.emitOwn({ type: "after_provider_response", status: response.status, headers }, this.agent.signal);
        };
        this.agent.prepareNextTurn = async () => {
            await this.flushPendingSessionWrites();
            const turnState = await this.createTurnState();
            this.applyTurnState(turnState);
            return {
                context: {
                    systemPrompt: turnState.systemPrompt,
                    messages: turnState.messages.slice(),
                    tools: turnState.activeTools.slice(),
                },
                model: turnState.model,
                thinkingLevel: turnState.thinkingLevel,
            };
        };
        this.agent.subscribe(async (event, signal) => {
            await this.handleAgentEvent(event, signal);
        });
    }
    async emitOwn(event, signal) {
        for (const listener of this.listeners) {
            await listener(event, signal);
        }
    }
    async emitAny(event, signal) {
        for (const listener of this.listeners) {
            await listener(event, signal);
        }
    }
    async emitHook(event) {
        const handlers = this.hooks.get(event.type);
        if (!handlers || handlers.size === 0)
            return undefined;
        let lastResult;
        for (const handler of handlers) {
            const result = await handler(event);
            if (result !== undefined) {
                lastResult = result;
            }
        }
        return lastResult;
    }
    async emitQueueUpdate() {
        await this.emitOwn({
            type: "queue_update",
            steer: [...this.steerQueue],
            followUp: [...this.followUpQueue],
            nextTurn: [...this.nextTurnQueue],
        });
    }
    async createTurnState() {
        const context = await this.session.buildContext();
        const resources = this.getResources();
        const tools = [...this.tools.values()];
        const activeTools = this.activeToolNames
            .map((name) => this.tools.get(name))
            .filter((tool) => tool !== undefined);
        let systemPrompt = "You are a helpful assistant.";
        if (typeof this.systemPrompt === "string") {
            systemPrompt = this.systemPrompt;
        }
        else if (this.systemPrompt) {
            systemPrompt = await this.systemPrompt({
                env: this.env,
                session: this.session,
                model: this.model,
                thinkingLevel: this.thinkingLevel,
                activeTools,
                resources,
            });
        }
        return {
            messages: context.messages,
            resources,
            systemPrompt,
            model: this.model,
            thinkingLevel: this.thinkingLevel,
            tools,
            activeTools,
        };
    }
    applyTurnState(turnState) {
        this.agent.state.messages = turnState.messages;
        this.agent.state.systemPrompt = turnState.systemPrompt;
        this.agent.state.model = turnState.model;
        this.agent.state.thinkingLevel = turnState.thinkingLevel;
        this.agent.state.tools = turnState.activeTools;
    }
    validateToolNames(toolNames) {
        const missing = toolNames.filter((name) => !this.tools.has(name));
        if (missing.length > 0)
            throw new Error(`Unknown tool(s): ${missing.join(", ")}`);
    }
    async flushPendingSessionWrites() {
        const writes = this.pendingSessionWrites;
        this.pendingSessionWrites = [];
        for (const write of writes) {
            if (write.type === "message") {
                await this.session.appendMessage(write.message);
            }
            else if (write.type === "model_change") {
                await this.session.appendModelChange(write.provider, write.modelId);
            }
            else if (write.type === "thinking_level_change") {
                await this.session.appendThinkingLevelChange(write.thinkingLevel);
            }
            else if (write.type === "custom") {
                await this.session.appendCustomEntry(write.customType, write.data);
            }
            else if (write.type === "custom_message") {
                await this.session.appendCustomMessageEntry(write.customType, write.content, write.display, write.details);
            }
            else if (write.type === "label") {
                await this.session.appendLabel(write.targetId, write.label);
            }
            else if (write.type === "session_info") {
                await this.session.appendSessionName(write.name ?? "");
            }
        }
    }
    async handleAgentEvent(event, signal) {
        await this.emitAny(event, signal);
        if (event.type === "message_start" && event.message.role === "user") {
            const steerIndex = this.steerQueue.indexOf(event.message);
            if (steerIndex !== -1) {
                this.steerQueue.splice(steerIndex, 1);
                await this.emitQueueUpdate();
            }
            else {
                const followUpIndex = this.followUpQueue.indexOf(event.message);
                if (followUpIndex !== -1) {
                    this.followUpQueue.splice(followUpIndex, 1);
                    await this.emitQueueUpdate();
                }
            }
        }
        if (event.type === "message_end") {
            await this.session.appendMessage(event.message);
        }
        if (event.type === "turn_end") {
            const hadPendingMutations = this.pendingSessionWrites.length > 0;
            await this.flushPendingSessionWrites();
            await this.emitOwn({
                type: "save_point",
                hadPendingMutations,
            });
        }
        if (event.type === "agent_end") {
            await this.flushPendingSessionWrites();
            this.phase = "idle";
            await this.emitOwn({ type: "settled", nextTurnCount: this.nextTurnQueue.length }, signal);
        }
    }
    async executeTurn(turnState, text, options) {
        this.applyTurnState(turnState);
        const beforeLength = this.agent.state.messages.length;
        let messages = [createUserMessage(text, options?.images)];
        if (this.nextTurnQueue.length > 0) {
            messages = [...this.nextTurnQueue, messages[0]];
            this.nextTurnQueue = [];
            await this.emitQueueUpdate();
        }
        const beforeResult = await this.emitHook({
            type: "before_agent_start",
            prompt: text,
            images: options?.images,
            systemPrompt: turnState.systemPrompt,
            resources: turnState.resources,
        });
        if (beforeResult?.messages)
            messages = [...beforeResult.messages, ...messages];
        if (beforeResult?.systemPrompt)
            this.agent.state.systemPrompt = beforeResult.systemPrompt;
        try {
            await this.agent.prompt(messages);
        }
        finally {
            await this.flushPendingSessionWrites();
        }
        let response;
        const newMessages = this.agent.state.messages.slice(beforeLength);
        for (let i = newMessages.length - 1; i >= 0; i--) {
            const message = newMessages[i];
            if (message.role === "assistant") {
                response = message;
                break;
            }
        }
        if (!response)
            throw new Error("AgentHarness prompt completed without an assistant message");
        return response;
    }
    async prompt(text, options) {
        if (this.phase !== "idle")
            throw new Error("AgentHarness is busy");
        this.phase = "turn";
        try {
            const turnState = await this.createTurnState();
            return await this.executeTurn(turnState, text, options);
        }
        catch (error) {
            this.phase = "idle";
            throw error;
        }
    }
    async skill(name, additionalInstructions) {
        if (this.phase !== "idle")
            throw new Error("AgentHarness is busy");
        this.phase = "turn";
        try {
            const turnState = await this.createTurnState();
            const skill = (turnState.resources.skills ?? []).find((candidate) => candidate.name === name);
            if (!skill)
                throw new Error(`Unknown skill: ${name}`);
            return await this.executeTurn(turnState, formatSkillInvocation(skill, additionalInstructions));
        }
        catch (error) {
            this.phase = "idle";
            throw error;
        }
    }
    async promptFromTemplate(name, args = []) {
        if (this.phase !== "idle")
            throw new Error("AgentHarness is busy");
        this.phase = "turn";
        try {
            const turnState = await this.createTurnState();
            const template = (turnState.resources.promptTemplates ?? []).find((candidate) => candidate.name === name);
            if (!template)
                throw new Error(`Unknown prompt template: ${name}`);
            return await this.executeTurn(turnState, formatPromptTemplateInvocation(template, args));
        }
        catch (error) {
            this.phase = "idle";
            throw error;
        }
    }
    steer(text, options) {
        if (this.phase === "idle")
            throw new Error("Cannot steer while idle");
        const message = createUserMessage(text, options?.images);
        this.steerQueue.push(message);
        this.agent.steer(message);
        void this.emitQueueUpdate();
    }
    followUp(text, options) {
        if (this.phase === "idle")
            throw new Error("Cannot follow up while idle");
        const message = createUserMessage(text, options?.images);
        this.followUpQueue.push(message);
        this.agent.followUp(message);
        void this.emitQueueUpdate();
    }
    nextTurn(text, options) {
        this.nextTurnQueue.push(createUserMessage(text, options?.images));
        void this.emitQueueUpdate();
    }
    async appendMessage(message) {
        if (this.phase === "idle") {
            await this.session.appendMessage(message);
        }
        else {
            this.pendingSessionWrites.push({ type: "message", message });
        }
    }
    async compact(customInstructions) {
        if (this.phase !== "idle")
            throw new Error("compact() requires idle harness");
        this.phase = "compaction";
        const model = this.model;
        if (!model)
            throw new Error("No model set for compaction");
        const auth = await this.getApiKeyAndHeaders?.(model);
        if (!auth)
            throw new Error("No auth available for compaction");
        const branchEntries = await this.session.getBranch();
        const preparation = prepareCompaction(branchEntries, DEFAULT_COMPACTION_SETTINGS);
        if (!preparation)
            throw new Error("Nothing to compact");
        const hookResult = await this.emitHook({
            type: "session_before_compact",
            preparation,
            branchEntries,
            customInstructions,
            signal: new AbortController().signal,
        });
        if (hookResult?.cancel) {
            this.phase = "idle";
            throw new Error("Compaction cancelled");
        }
        const provided = hookResult?.compaction;
        const result = provided ??
            (await compact(preparation, model, auth.apiKey, auth.headers, customInstructions, undefined, this.thinkingLevel));
        const entryId = await this.session.appendCompaction(result.summary, result.firstKeptEntryId, result.tokensBefore, result.details, provided !== undefined);
        const entry = await this.session.getEntry(entryId);
        if (entry?.type === "compaction") {
            await this.emitOwn({ type: "session_compact", compactionEntry: entry, fromHook: provided !== undefined });
        }
        this.phase = "idle";
        return result;
    }
    async navigateTree(targetId, options) {
        if (this.phase !== "idle")
            throw new Error("navigateTree() requires idle harness");
        this.phase = "branch_summary";
        const oldLeafId = await this.session.getLeafId();
        if (oldLeafId === targetId) {
            this.phase = "idle";
            return { cancelled: false };
        }
        const targetEntry = await this.session.getEntry(targetId);
        if (!targetEntry)
            throw new Error(`Entry ${targetId} not found`);
        const { entries, commonAncestorId } = await collectEntriesForBranchSummary(this.session, oldLeafId, targetId);
        const preparation = {
            targetId,
            oldLeafId,
            commonAncestorId,
            entriesToSummarize: entries,
            userWantsSummary: options?.summarize ?? false,
            customInstructions: options?.customInstructions,
            replaceInstructions: options?.replaceInstructions,
            label: options?.label,
        };
        const signal = new AbortController().signal;
        const hookResult = await this.emitHook({
            type: "session_before_tree",
            preparation,
            signal,
        });
        if (hookResult?.cancel) {
            this.phase = "idle";
            return { cancelled: true };
        }
        let summaryEntry;
        let summaryText = hookResult?.summary?.summary;
        let summaryDetails = hookResult?.summary?.details;
        if (!summaryText && options?.summarize && entries.length > 0) {
            const model = this.model;
            if (!model)
                throw new Error("No model set for branch summary");
            const auth = await this.getApiKeyAndHeaders?.(model);
            if (!auth)
                throw new Error("No auth available for branch summary");
            const branchSummary = await generateBranchSummary(entries, {
                model,
                apiKey: auth.apiKey,
                headers: auth.headers,
                signal: new AbortController().signal,
                customInstructions: hookResult?.customInstructions ?? options?.customInstructions,
                replaceInstructions: hookResult?.replaceInstructions ?? options?.replaceInstructions,
            });
            if (branchSummary.aborted) {
                this.phase = "idle";
                return { cancelled: true };
            }
            if (branchSummary.error)
                throw new Error(branchSummary.error);
            summaryText = branchSummary.summary;
            summaryDetails = {
                readFiles: branchSummary.readFiles ?? [],
                modifiedFiles: branchSummary.modifiedFiles ?? [],
            };
        }
        let editorText;
        let newLeafId;
        if (targetEntry.type === "message" && targetEntry.message.role === "user") {
            newLeafId = targetEntry.parentId;
            const content = targetEntry.message.content;
            editorText =
                typeof content === "string"
                    ? content
                    : content
                        .filter((c) => c.type === "text")
                        .map((c) => c.text)
                        .join("");
        }
        else if (targetEntry.type === "custom_message") {
            newLeafId = targetEntry.parentId;
            editorText =
                typeof targetEntry.content === "string"
                    ? targetEntry.content
                    : targetEntry.content
                        .filter((c) => c.type === "text")
                        .map((c) => c.text)
                        .join("");
        }
        else {
            newLeafId = targetId;
        }
        const summaryId = await this.session.moveTo(newLeafId, summaryText
            ? {
                summary: summaryText,
                details: summaryDetails,
                fromHook: hookResult?.summary !== undefined,
            }
            : undefined);
        if (summaryId) {
            summaryEntry = await this.session.getEntry(summaryId);
        }
        await this.emitOwn({
            type: "session_tree",
            newLeafId: await this.session.getLeafId(),
            oldLeafId,
            summaryEntry,
            fromHook: hookResult?.summary !== undefined,
        });
        this.phase = "idle";
        return { cancelled: false, editorText, summaryEntry };
    }
    async setModel(model) {
        const previousModel = this.model;
        this.model = model;
        if (this.phase === "idle") {
            this.agent.state.model = model;
            await this.session.appendModelChange(model.provider, model.id);
        }
        else {
            this.pendingSessionWrites.push({ type: "model_change", provider: model.provider, modelId: model.id });
        }
        await this.emitOwn({ type: "model_select", model, previousModel, source: "set" });
    }
    async setThinkingLevel(level) {
        const previousLevel = this.thinkingLevel;
        this.thinkingLevel = level;
        if (this.phase === "idle") {
            this.agent.state.thinkingLevel = level;
            await this.session.appendThinkingLevelChange(level);
        }
        else {
            this.pendingSessionWrites.push({ type: "thinking_level_change", thinkingLevel: level });
        }
        await this.emitOwn({ type: "thinking_level_select", level, previousLevel });
    }
    async setActiveTools(toolNames) {
        this.validateToolNames(toolNames);
        this.activeToolNames = [...toolNames];
        if (this.phase === "idle") {
            this.agent.state.tools = this.activeToolNames.map((name) => this.tools.get(name));
        }
    }
    get steeringMode() {
        return this.agent.steeringMode;
    }
    set steeringMode(mode) {
        this.agent.steeringMode = mode;
    }
    get followUpMode() {
        return this.agent.followUpMode;
    }
    set followUpMode(mode) {
        this.agent.followUpMode = mode;
    }
    getResources() {
        return {
            skills: this.resources.skills?.slice(),
            promptTemplates: this.resources.promptTemplates?.slice(),
        };
    }
    async setResources(resources) {
        const previousResources = this.getResources();
        this.resources = {
            skills: resources.skills?.slice(),
            promptTemplates: resources.promptTemplates?.slice(),
        };
        await this.emitOwn({ type: "resources_update", resources: this.getResources(), previousResources });
    }
    async setTools(tools, activeToolNames) {
        this.tools = new Map(tools.map((tool) => [tool.name, tool]));
        if (activeToolNames) {
            this.validateToolNames(activeToolNames);
            this.activeToolNames = [...activeToolNames];
        }
        else {
            this.validateToolNames(this.activeToolNames);
        }
        if (this.phase === "idle") {
            this.agent.state.tools = this.activeToolNames.map((name) => this.tools.get(name));
        }
    }
    async abort() {
        const clearedSteer = [...this.steerQueue];
        const clearedFollowUp = [...this.followUpQueue];
        this.steerQueue = [];
        this.followUpQueue = [];
        this.agent.clearAllQueues();
        await this.emitQueueUpdate();
        this.agent.abort();
        await this.agent.waitForIdle();
        await this.emitOwn({ type: "abort", clearedSteer, clearedFollowUp });
        return { clearedSteer, clearedFollowUp };
    }
    async waitForIdle() {
        await this.agent.waitForIdle();
    }
    subscribe(listener) {
        this.listeners.add(listener);
        return () => this.listeners.delete(listener);
    }
    on(type, handler) {
        let handlers = this.hooks.get(type);
        if (!handlers) {
            handlers = new Set();
            this.hooks.set(type, handlers);
        }
        handlers.add(handler);
        return () => handlers.delete(handler);
    }
}
//# sourceMappingURL=agent-harness.js.map