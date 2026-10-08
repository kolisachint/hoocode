#!/usr/bin/env bash
# Generate placeholder crates for the hoocode workspace.
# This is a one-off scaffolding script used during the initial migration.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATES_DIR="$ROOT/crates"

mkdir -p "$CRATES_DIR"

# name | description | namespace | kind
# kind: leaf
declare -a CRATES=(

    "hoocode-ai-env|Environment and API key handling for hoocode AI|ai|leaf"
    "hoocode-ai-models|LLM model registry and discovery for hoocode AI|ai|leaf"
    "hoocode-ai-oauth|OAuth flows for hoocode AI providers|ai|leaf"
    "hoocode-ai-provider-anthropic|Anthropic provider for hoocode AI|ai|leaf"
    "hoocode-ai-provider-faux|Faux / test provider for hoocode AI|ai|leaf"
    "hoocode-ai-provider-google|Google Gemini provider for hoocode AI|ai|leaf"
    "hoocode-ai-provider-openai|OpenAI provider for hoocode AI|ai|leaf"
    "hoocode-ai-stream|Streaming response utilities for hoocode AI|ai|leaf"
    "hoocode-ai-types|Shared types for hoocode AI|ai|leaf"
    "hoocode-ai-util|Shared utilities for hoocode AI|ai|leaf"

    "hoocode-agent-core|Core agent runtime for hoocode agents|agent|leaf"
    "hoocode-agent-compaction|Session compaction for hoocode agents|agent|leaf"
    "hoocode-agent-harness|Agent harness for hoocode agents|agent|leaf"
    "hoocode-agent-loop|Agent loop for hoocode agents|agent|leaf"
    "hoocode-agent-session|Session management for hoocode agents|agent|leaf"
    "hoocode-agent-types|Shared types for hoocode agents|agent|leaf"

    "hoocode-code-config|Configuration for the hoocode coding agent|code|leaf"
    "hoocode-code-main|Main entry point for the hoocode coding agent|code|leaf"
    "hoocode-code-print|Output formatting for the hoocode coding agent|code|leaf"
    "hoocode-code-prompts|Prompt templates for the hoocode coding agent|code|leaf"
    "hoocode-code-resources|Resource management for the hoocode coding agent|code|leaf"
    "hoocode-code-rpc|RPC mode for the hoocode coding agent|code|leaf"
    "hoocode-code-session|Session handling for the hoocode coding agent|code|leaf"
    "hoocode-code-subagents|Subagent orchestration for the hoocode coding agent|code|leaf"
    "hoocode-code-tools|Coding tools for the hoocode coding agent|code|leaf"

    "hoocode-tui-components|UI components for the hoocode TUI|tui|leaf"
    "hoocode-tui-editing|Text editing primitives for the hoocode TUI|tui|leaf"
    "hoocode-tui-fuzzy|Fuzzy matching for the hoocode TUI|tui|leaf"
    "hoocode-tui-images|Terminal image rendering for the hoocode TUI|tui|leaf"
    "hoocode-tui-keys|Keyboard handling for the hoocode TUI|tui|leaf"
    "hoocode-tui-render|Differential rendering for the hoocode TUI|tui|leaf"
    "hoocode-tui-terminal|Terminal abstraction for the hoocode TUI|tui|leaf"
    "hoocode-tui-util|Shared utilities for the hoocode TUI|tui|leaf"
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

[package.metadata.hoocode]
publish = true

[dependencies]
EOF

    # src/lib.rs
    cat > "$dir/src/lib.rs" <<EOF
//! $desc
//!
//! This crate is currently a placeholder reserved for the hoocode Rust migration.
//! Functionality will be ported from the TypeScript HooCode project incrementally.
EOF

    # README.md
    cat > "$dir/README.md" <<EOF
# $name

$desc

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

This crate is currently a placeholder reserved for the Rust migration from HooCode.
EOF

done

echo "Generated ${#CRATES[@]} crates in $CRATES_DIR"
