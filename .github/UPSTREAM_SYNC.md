# Upstream updates

`Check upstream` checks `denoland/celld:main` every Monday at 16:17 UTC and can
also be run manually. It opens a draft PR directly from upstream main and reuses
that PR while open. Review and merge updates manually with **Create a merge
commit**, preserving upstream ancestry. The existing release workflow remains
separate.

No new upstream commits or an existing proposal cause no repository writes,
PR edits, or explicit CI dispatches. Closing a proposal declines that candidate;
it is not recreated unchanged. The updater never imports branches, runs upstream
code, merges main, publishes, or deploys. It uses the built-in `GITHUB_TOKEN`;
**no additional token or repository secret is required**.

## Activation

Merge the workflow into this fork's `main` to activate scheduled/manual runs.
GitHub Actions must be enabled for the fork. Enable **Settings → Actions →
General → Allow GitHub Actions to create and approve pull requests**. PR
validation only has read permissions; sync runs only on this fork's `main`.

The PR tracks upstream's branch. If integration needs conflict-resolution
commits, create a branch in this fork, merge upstream into it, and open a
replacement PR rather than trying to push fixes to upstream main.

This repository's existing Release workflow runs for `candidate`, published
releases, and manual dispatch. The updater's validation tests the updater itself;
validate proposed celld changes with the normal Cargo checks before merging.

## Prior requests

This follows the September 8 Codex thread **Sync fork and add Dependabot**
(`01a08248-0989-7870-ab70-f7535973679b`) and the October 28 thread **Audit upstream
merge PR workflows** (`01a1839c-d209-7433-8e1f-247a5106235a`): weekly checks,
manual merges, and quiet unchanged proposals. It uses the simple built-in-token
approach already running in the S2 and Tailcat Swift forks.

References: [scheduled workflow behavior](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule),
[repository workflow permissions](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository#setting-the-permissions-of-the-github_token).

Run updater regressions locally with `node --test .github/scripts/test-sync-upstream.cjs`.
