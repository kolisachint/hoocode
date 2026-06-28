# Browser Flow Discovery Example

This example demonstrates how to use `browser_run` with `decide` steps for
exploratory/discovery scenarios.

## How It Works

1. **Navigate** to a URL
2. **Wait** for the page to settle
3. **Decide** (suspend) - The engine pauses and asks the LLM:
   - "What is this page about?"
   - "What should I explore next?"
4. **LLM responds** with a concrete action (click, extract, etc.)
5. **Engine executes** the action and continues

## Flow Structure

```json
{
  "steps": [
    { "action": { "action": "navigate", "url": "{{url}}" } },
    { "action": { "action": "wait_settle" } },
    { "action": { "action": "decide", "goal": "..." } }
  ]
}
```

## The `decide` Step

When the engine hits a `decide` step, it suspends with:

```json
{
  "outcome": "needs_parent",
  "request": {
    "request": "decide_next_action",
    "goal": "Identify the main content...",
    "observation": { "state_signature": "..." },
    "screenshot_ref": "..."
  },
  "token": "run_xxx:1"
}
```

The LLM then responds with:

```json
{
  "response": "next_action",
  "action": { "action": "click", "selector": "a.main-link" }
}
```

## Usage (when browsertools is installed)

```bash
# Start a flow with the discover pattern
hoocode --enable-browsertools

# In the session:
browser_run(
  flow_path: "examples/discover-flow.json",
  vars: { "url": "https://example.com" }
)

# When it suspends, use browser_continue to continue
browser_continue(
  token: "<token from suspension>",
  response: { "response": "next_action", "action": { "action": "click", "selector": "..." } }
)
```

## Available Actions

- `navigate` - Go to a URL
- `click` - Click an element
- `fill` - Fill an input field
- `select` - Select a dropdown option
- `wait_settle` - Wait for page to be idle
- `checkpoint` - Assert conditions
- `decide` - Delegate decision to LLM (suspends)
