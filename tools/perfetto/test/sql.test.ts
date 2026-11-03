import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, writeFileSync, readFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve, join, dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {
  renderQueries,
  renderCapabilities,
  renderEventRanking,
} from '../scripts/sql.ts';

test('Nunjucks resolves SQL includes and generates every metric', () => {
  const queries = renderQueries();
  assert.deepEqual(Object.keys(queries), [
    'hot_actions',
    'memory',
    'peaks',
    'retention',
    'scopes',
    'growth',
    'churn',
    'events',
  ]);
  for (const sql of Object.values(queries).flatMap((query) => [
    query.sql,
    query.schemaSql,
  ]))
    assert.doesNotMatch(sql, /\{%|\{\{/);
  assert.match(queries.memory.sql, /Rust heap \/ live bytes/);
  assert.match(queries.growth.sql, /COALESCE\(label/);
});

test(
  'all materialized datasets execute in the real Trace Processor',
  {skip: !process.env.TRACE_PROCESSOR || !process.env.GUINEA_TEST_TRACE},
  () => {
    const processor = process.env.TRACE_PROCESSOR;
    const trace = process.env.GUINEA_TEST_TRACE;
    if (!processor || !trace) return;
    const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
    const directory = mkdtempSync(join(tmpdir(), 'guinea-profile-sql-'));
    const sql =
      readFileSync(
        join(root, 'crates/guinea-trace/sql/guinea/memory.sql'),
        'utf8',
      ) +
      '\n' +
      Object.values(renderQueries())
        .map((query) => query.sql)
        .join('\n') +
      '\n' +
      renderCapabilities() +
      '\n' +
      readFileSync(
        join(root, 'crates/guinea-trace/sql/event_tracks.sql'),
        'utf8',
      ) +
      '\n' +
      renderEventRanking() +
      `
      SELECT
        (SELECT SUM(occurrences) FROM guinea_profile_events) =
          (SELECT COUNT(*) FROM slice WHERE EXTRACT_ARG(arg_set_id, 'debug.guinea_id') IS NOT NULL)
        AND
        NOT EXISTS (SELECT 1 FROM guinea_profile_events
          WHERE event IN ('deliver', 'publish', 'handle', 'push'))
        AND ABS(
          (SELECT COALESCE(SUM(allocated_mib), 0) FROM guinea_profile_events) -
          (SELECT COALESCE(SUM(EXTRACT_ARG(arg_set_id, 'debug.memory_allocated_bytes')), 0) / 1048576.0
            FROM slice WHERE EXTRACT_ARG(arg_set_id, 'debug.guinea_id') IS NOT NULL)
        ) < 1e-9 AS event_labels_and_totals_valid;
      `;
    const file = join(directory, 'metrics.sql');
    writeFileSync(file, sql);
    const result = spawnSync(
      processor,
      ['query', '--format', 'csv', '-f', file, trace],
      {
        encoding: 'utf8',
      },
    );
    assert.equal(result.status, 0, result.stderr + result.stdout);
    assert.match(
      result.stdout,
      /"event_labels_and_totals_valid"\r?\n1(?:\r?\n|$)/,
    );
  },
);
