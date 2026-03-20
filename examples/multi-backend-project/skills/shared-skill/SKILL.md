# Code Quality Audit

A skill for auditing code quality across the project.

## When to use

Use this skill when the user asks for a code quality review, audit, or health check of the codebase.

## Steps

1. Run `cargo clippy` and report any warnings.
2. Check for `TODO` and `FIXME` comments and list them with file locations.
3. Look for functions longer than 50 lines and suggest splitting them.
4. Identify any `unsafe` blocks and verify they have safety comments.
5. Check that all public items have documentation comments.
6. Summarize findings with a severity rating (healthy, needs attention, critical).

## Notes

- This skill works across all backends since it operates on the source code directly.
- Focus on actionable findings rather than style nitpicks.
