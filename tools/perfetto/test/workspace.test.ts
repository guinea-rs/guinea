import {test} from 'node:test';
import assert from 'node:assert/strict';
import {eventTracks} from '../src/workspace.ts';

test('event lanes survive UI renaming and merged tracks; CPU markers stay out', () => {
  const tracks = [
    {uri: '/native/thread-7/events', tags: {trackIds: [8, 9]}},
    {uri: '/native/thread-7/another-name', tags: {trackIds: [10]}},
    {uri: '/native/thread-7/samples', tags: {trackIds: [11]}},
    {uri: '/native/thread-7/flamechart'},
  ];
  assert.deepEqual(
    eventTracks(
      tracks,
      new Map([
        [9, 'Main thread lane 1'],
        [10, 'Worker lane 0'],
      ]),
    ),
    [
      {uri: '/native/thread-7/events', name: 'Events · Main thread lane 1'},
      {uri: '/native/thread-7/another-name', name: 'Events · Worker lane 0'},
    ],
  );
});

test('packaged UI preparation enables Guinea by default and is idempotent', async () => {
  const {mkdtempSync, mkdirSync, writeFileSync, readFileSync} =
    await import('node:fs');
  const {tmpdir} = await import('node:os');
  const {join, resolve, dirname} = await import('node:path');
  const {fileURLToPath} = await import('node:url');
  const {spawnSync} = await import('node:child_process');
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const checkout = mkdtempSync(join(tmpdir(), 'guinea-ui-defaults-'));
  const defaults = join(checkout, 'ui/src/core/embedder/default_plugins.ts');
  mkdirSync(dirname(defaults), {recursive: true});
  writeFileSync(
    defaults,
    "export const defaultPlugins = ['dev.perfetto.Test'];\n",
  );
  for (let i = 0; i < 2; i++) {
    const prepared = spawnSync(
      process.execPath,
      [
        '--import',
        'tsx',
        join(root, 'scripts/prepare.ts'),
        checkout,
        join(root, 'package.json'),
      ],
      {cwd: root, encoding: 'utf8'},
    );
    assert.equal(prepared.status, 0, prepared.stderr);
  }
  assert.equal(
    readFileSync(defaults, 'utf8'),
    "export const defaultPlugins = [\n  'rs.guinea.Profile','dev.perfetto.Test'];\n",
  );
});
