// Run: node crates/spindle-mcp/tests/console_episode_requests.cjs
// Exercise the shipped handlers without a browser or third-party dependencies.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const html = fs.readFileSync(path.join(__dirname, '../src/console.html'), 'utf8');
const handlers = html.slice(html.indexOf('async function loadEpisodeDetail('), html.indexOf('async function renderUsage('));
const detail = {innerHTML: '', scrollIntoView() { this.scrolled = true; }};
const project = {value: 'project'};
let epoch = 0;
const context = vm.createContext({
  $: id => id === 'series-project' ? project : detail,
  loading: (_id, message) => {
    detail.innerHTML = message;
    const current = ++epoch;
    return () => epoch === current;
  },
  errorHTML: (_where, error) => `Error: ${error.message}`,
  episodeArgs: (pid, episode) => ({project_id: pid, ...episode}),
});
vm.runInContext(handlers, context);

(async () => {
  const episode = {book_number: 1, chapter_number: 2};
  const seen = [];
  context.callTool = async tool => { seen.push(tool); throw new Error('Connection lost'); };
  for (const run of [
    () => context.previewEpisode('project', episode),
    () => context.showRelease('release', 'project'),
    () => context.reviewEpisode('project', episode),
  ]) {
    detail.scrolled = false;
    await run();
    assert.match(detail.innerHTML, /Connection lost/);
    assert.match(detail.innerHTML, /retry/);
    assert.equal(detail.scrolled, true);
  }
  assert.deepEqual(seen, ['prepare_episode_release', 'get_episode_release', 'read_episode']);

  let rejectOld;
  context.callTool = () => new Promise((_resolve, reject) => { rejectOld = reject; });
  const old = context.loadEpisodeDetail('project', 'Old request', 'read_episode', {});
  context.callTool = async () => ({ok: true});
  assert.deepEqual(await context.loadEpisodeDetail('project', 'New request', 'read_episode', {}), {ok: true});
  detail.innerHTML = 'Newer result';
  rejectOld(new Error('Old failure'));
  assert.equal(await old, null);
  assert.equal(detail.innerHTML, 'Newer result');

  let resolveOld;
  context.callTool = () => new Promise(resolve => { resolveOld = resolve; });
  const switched = context.loadEpisodeDetail('project', 'Loading', 'read_episode', {});
  project.value = 'another-project';
  detail.innerHTML = 'Another project';
  resolveOld({ok: true});
  assert.equal(await switched, null);
  assert.equal(detail.innerHTML, 'Another project');
  console.log('Episode failures, retry guidance, stale responses and project isolation pass.');
})().catch(error => { console.error(error); process.exitCode = 1; });
