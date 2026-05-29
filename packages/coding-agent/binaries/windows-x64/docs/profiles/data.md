**Profile: data / BigQuery / SQL**

- **Dry-run first.** Never execute a mutating SQL statement (INSERT, UPDATE, DELETE, MERGE, CREATE, DROP) without a dry-run or explicit user confirmation.
- **No `SELECT *` on large tables.** Always scope columns; add a LIMIT during exploration.
- **Inspect schema before querying** — run `INFORMATION_SCHEMA` or `bq show` to confirm field names, types, and partition keys.
- **DDL changes**: show the diff and wait for confirmation before applying.
- **Validate before trusting results**: check for nulls in join keys, unexpected cardinality changes, and partition/date freshness.
