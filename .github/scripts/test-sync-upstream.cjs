const assert = require('node:assert/strict');
const test = require('node:test');
const syncUpstream = require('./sync-upstream.cjs');

const candidate = 'b'.repeat(40);
const older = 'a'.repeat(40);

function fixture({ ahead = 1, proposals = [] } = {}) {
  const writes = [];
  const github = {
    paginate: async () => proposals,
    rest: {
      repos: {
        getBranch: async ({ owner }) => ({ data: { commit: { sha: owner === 'denoland' ? candidate : older } } }),
        compareCommitsWithBasehead: async () => ({ data: { ahead_by: ahead, behind_by: 0 } }),
      },
      pulls: {
        list() {},
        create: async args => {
          writes.push(['pr', args]);
          return { data: { html_url: 'https://github.com/zshannon/celld/pull/1' } };
        },
      },
      git: {
        getRef: async () => { throw Object.assign(new Error('Not Found'), { status: 404 }); },
        createRef: async args => writes.push(['create', args]),
        updateRef: async args => writes.push(['update', args]),
      },
    },
  };
  return { github, writes, context: { repo: { owner: 'zshannon', repo: 'celld' } }, core: { info() {} } };
}

function pr({ sha = candidate, state = 'open', repository = 'denoland/celld' } = {}) {
  return { state, head: { sha, ref: 'main', repo: { full_name: repository } } };
}

test('no new upstream commits cause no writes', async () => {
  const f = fixture({ ahead: 0 });
  await syncUpstream(f);
  assert.deepEqual(f.writes, []);
});

test('new upstream changes open a draft PR without importing a branch', async () => {
  const f = fixture();
  await syncUpstream(f);
  assert.deepEqual(f.writes.map(([kind]) => kind), ['pr']);
  const request = f.writes[0][1];
  assert.equal(request.head, 'denoland:main');
  assert.equal(request.base, 'main');
  assert.equal(request.draft, true);
  assert.match(request.body, /merge commit/i);
});

test('an existing upstream PR is left completely unchanged', async () => {
  const f = fixture({ proposals: [pr()] });
  await syncUpstream(f);
  assert.deepEqual(f.writes, []);
});

test('a declined candidate is not proposed again unchanged', async () => {
  const f = fixture({ proposals: [pr({ state: 'closed' })] });
  await syncUpstream(f);
  assert.deepEqual(f.writes, []);
});

test('an older closed proposal does not suppress a new candidate', async () => {
  const f = fixture({ proposals: [pr({ state: 'closed', sha: older })] });
  await syncUpstream(f);
  assert.deepEqual(f.writes.map(([kind]) => kind), ['pr']);
});

test('an unrelated PR does not suppress an upstream proposal', async () => {
  const f = fixture({ proposals: [pr({ repository: 'someone/celld' })] });
  await syncUpstream(f);
  assert.deepEqual(f.writes.map(([kind]) => kind), ['pr']);
});
