#!/usr/bin/env bash
# Generate placeholder crates for the cortexcode workspace.
# This is a one-off scaffolding script used during the initial migration.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATES_DIR="$ROOT/crates"

mkdir -p "$CRATES_DIR"

# name | description | namespace | kind
# kind: leaf
declare -a CRATES=(

    "cortexcode-ai-env|Environment and API key handling for cortex AI|ai|leaf"
    "cortexcode-ai-models|LLM model registry and discovery for cortex AI|ai|leaf"
    "cortexcode-ai-oauth|OAuth flows for cortex AI providers|ai|leaf"
    "cortexcode-ai-provider-anthropic|Anthropic provider for cortex AI|ai|leaf"
    "cortexcode-ai-provider-azure|Azure OpenAI provider for cortex AI|ai|leaf"
    "cortexcode-ai-provider-faux|Faux / test provider for cortex AI|ai|leaf"
    "cortexcode-ai-provider-google|Google Gemini provider for cortex AI|ai|leaf"
    "cortexcode-ai-provider-openai|OpenAI provider for cortex AI|ai|leaf"
    "cortexcode-ai-stream|Streaming response utilities for cortex AI|ai|leaf"
    "cortexcode-ai-types|Shared types for cortex AI|ai|leaf"
    "cortexcode-ai-util|Shared utilities for cortex AI|ai|leaf"

    "cortexcode-agent-core|Core agent runtime for cortex agents|agent|leaf"
    "cortexcode-agent-compaction|Session compaction for cortex agents|agent|leaf"
    "cortexcode-agent-harness|Agent harness for cortex agents|agent|leaf"
    "cortexcode-agent-loop|Agent loop for cortex agents|agent|leaf"
    "cortexcode-agent-session|Session management for cortex agents|agent|leaf"
    "cortexcode-agent-types|Shared types for cortex agents|agent|leaf"

    "cortexcode-code-config|Configuration for the cortex coding agent|code|leaf"
    "cortexcode-code-main|Main entry point for the cortex coding agent|code|leaf"
    "cortexcode-code-print|Output formatting for the cortex coding agent|code|leaf"
    "cortexcode-code-prompts|Prompt templates for the cortex coding agent|code|leaf"
    "cortexcode-code-resources|Resource management for the cortex coding agent|code|leaf"
    "cortexcode-code-rpc|RPC mode for the cortex coding agent|code|leaf"
    "cortexcode-code-session|Session handling for the cortex coding agent|code|leaf"
    "cortexcode-code-subagents|Subagent orchestration for the cortex coding agent|code|leaf"
    "cortexcode-code-tools|Coding tools for the cortex coding agent|code|leaf"

    "cortexcode-tui-components|UI components for the cortex TUI|tui|leaf"
    "cortexcode-tui-editing|Text editing primitives for the cortex TUI|tui|leaf"
    "cortexcode-tui-fuzzy|Fuzzy matching for the cortex TUI|tui|leaf"
    "cortexcode-tui-images|Terminal image rendering for the cortex TUI|tui|leaf"
    "cortexcode-tui-keys|Keyboard handling for the cortex TUI|tui|leaf"
    "cortexcode-tui-render|Differential rendering for the cortex TUI|tui|leaf"
    "cortexcode-tui-terminal|Terminal abstraction for the cortex TUI|tui|leaf"
    "cortexcode-tui-util|Shared utilities for the cortex TUI|tui|leaf"
)

for entry in "${CRATES[@]}"; do
    IFS='|' read -r name desc namespace kind <<< "$entry"
    dir="$CRATES_DIR/$name"
    mkdir -p "$dir/src"

    # Cargo.toml
    cat > "$dir/Cargo.toml" <<EOF
[package]
name = "$name"
version.workspace = true
authors.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true
description = "$desc"
readme = "README.md"

[package.metadata.cortex]
publish = true

[dependencies]
EOF

    # src/lib.rs
    cat > "$dir/src/lib.rs" <<EOF
//! $desc
//!
//! This crate is currently a placeholder reserved for the cortexcode Rust migration.
//! Functionality will be ported from the TypeScript HooCode project incrementally.
EOF

    # README.md
    cat > "$dir/README.md" <<EOF
# $name

$desc

Part of the [cortexcode](https://github.com/kolisachint/cortexcode) Rust workspace.

This crate is currently a placeholder reserved for the Rust migration from HooCode.
EOF

done

echo "Generated ${#CRATES[@]} crates in $CRATES_DIR"
