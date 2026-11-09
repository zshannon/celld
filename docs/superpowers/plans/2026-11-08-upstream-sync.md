# Upstream sync implementation plan

Goal: reproduce the user's weekly upstream-update PR policy for `zshannon/celld`,
tracking `denoland/celld:main` with the built-in GitHub token.

Design: open a draft PR directly from upstream main and leave existing proposals
untouched. Merges stay manual, matching the September 8 and October 28 Codex
requests. Use the simpler existing S2/Tailcat workflow pattern, as clarified in
this conversation. No additional secret is required.

- [x] Add API behavior tests and verify failure before implementation.
- [x] Implement `.github/scripts/sync-upstream.cjs` with no upstream code execution,
  branch imports, publishing, or explicit CI dispatch.
- [x] Add a weekly/manual workflow with read-only PR validation and sync restricted
  to this fork's main branch, and document activation.
- [x] Run updated Node tests, workflow lint, and a read-only check against real refs.
- [x] Commit the workflow separately from the secrets implementation.

Additional notification customization and release automation are outside this
change. Conflict-resolution branches are deferred until needed (likelihood 3,
severity 5), consistent with the requested simplicity threshold.

Verification: all six updater tests and actionlint 1.7.12 pass. Running the actual
updater against live GitHub reads returned “No new upstream commits” and made no
remote writes. Activation requires the workflow on main.
