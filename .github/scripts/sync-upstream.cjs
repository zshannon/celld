// Runs from the fork's main branch; never checks out or executes upstream code.
module.exports = async function syncUpstream({ context, core, github }) {
  const { owner, repo } = context.repo;
  const base = 'main';
  const head = 'denoland:main';
  const { data: upstream } = await github.rest.repos.getBranch({ owner: 'denoland', repo, branch: base });
  const candidate = upstream.commit.sha;
  const { data: comparison } = await github.rest.repos.compareCommitsWithBasehead({
    owner, repo, basehead: `${base}...${candidate}`, per_page: 1,
  });
  if (comparison.ahead_by === 0) {
    core.info('No new upstream commits.');
    return;
  }

  const proposals = (await github.paginate(github.rest.pulls.list, {
    owner, repo, base, head, state: 'all', per_page: 100,
  })).filter(pr => pr.head.repo?.full_name === 'denoland/celld' && pr.head.ref === base);
  const existing = proposals.find(pr => pr.state === 'open');
  if (existing) {
    core.info(`Upstream changes are already tracked by ${existing.html_url}`);
    return;
  }
  if (proposals.some(pr => pr.state === 'closed' && pr.head.sha === candidate)) {
    core.info('This upstream candidate was already proposed and closed.');
    return;
  }

  const { data: pr } = await github.rest.pulls.create({
    owner, repo, base, head, draft: true, maintainer_can_modify: false,
    title: 'chore(upstream): merge denoland/celld main',
    body: [
      'Bring upstream changes from `denoland/celld:main` into this fork.',
      '',
      'Weekly checks reuse this PR without edits or explicit CI dispatches. It follows upstream main automatically while open.',
      'Review the changes, resolve any conflicts, and validate the integration before marking it ready.',
      '',
      'Merge manually with **Create a merge commit** to retain upstream ancestry. Do not squash or rebase upstream imports.',
      'This workflow does not merge, publish, or deploy. It uses the built-in GITHUB_TOKEN and needs no additional secret.',
    ].join('\n'),
  });
  core.info(`Opened draft upstream proposal: ${pr.html_url}`);
};
