# Architecture Guide

## Dependency Rules

1. All crates may depend on `slateduck-core`
2. Only leaf crates depend on `slateduck-catalog`
3. Only `slateduck-pgwire` depends on `slateduck-sql`
4. No circular dependencies
5. `slateduck-core` has zero async dependencies

## Adding a New DuckLake Operation

1. Add key encoding in `slateduck-core`
2. Add catalog operation in `slateduck-catalog`
3. Add SQL pattern in `slateduck-sql`
4. Add wire handling in `slateduck-pgwire`
5. Add test at each layer
