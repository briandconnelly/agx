# Deploy Checklist

A skill for walking through pre-deployment verification steps.

## When to use

Use this skill when the user asks to prepare a deployment, release, or push to production.

## Steps

1. Run the full test suite and confirm all tests pass.
2. Check for any uncommitted changes in the working tree.
3. Verify that the changelog has been updated for this release.
4. Confirm the version number has been bumped in the project config.
5. Check that environment variables are documented and match production expectations.
6. Review any pending database migrations.
7. Summarize findings and ask the user to confirm before proceeding.

## Notes

- Do not deploy if any check fails. Report the failure clearly.
- If the project uses CI/CD, remind the user to verify the pipeline status.
