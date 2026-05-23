# Type-Aware Stats

`prune_files()` uses type-aware comparison for min/max statistics.

## Rules

| Type | Comparison |
|------|-----------|
| Integers | Signed/unsigned per width |
| Decimals | Rational (not float) |
| Timestamps | Typed temporal |
| Floats | IEEE 754 |
| Strings | Lexicographic UTF-8 |
| Unknown | Fail closed (0A000) |

## `contains_nan`

When true, the file cannot be pruned by any range predicate on that column.
