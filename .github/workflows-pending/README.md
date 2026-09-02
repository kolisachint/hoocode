# Workflows waiting to be installed

A workflow in this directory is finished and reviewed but not yet live:
GitHub refuses a push that creates or edits anything under
`.github/workflows/` unless the pushing token carries the `workflow` scope,
and the token that opened this branch does not.

To install one, move it and commit:

```
git mv .github/workflows-pending/<name>.yml .github/workflows/<name>.yml
```

The move needs a push from a credential with `workflow` scope — a normal
local checkout has one. Nothing else about the file needs to change; it is
the final version, not a draft.

Delete this directory once it is empty.
