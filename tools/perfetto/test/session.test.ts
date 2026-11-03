import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseMetrics} from '../src/model.js';
import {prepareDatasets} from '../src/session.js';

test('only supported metrics enter the session', () => {
  assert.deepEqual(parseMetrics(['hot_actions', 'hot_actions']), [
    'hot_actions',
  ]);
  assert.throws(() => parseMetrics({metrics: []}));
  assert.throws(() => parseMetrics(['unknown']));
});

test('CPU source is materialized with known columns before rendering', async () => {
  const queries: string[] = [];
  const sources = await prepareDatasets(
    {
      async query(sql) {
        queries.push(sql);
        return {
          columns: () => ['action', 'estimated_cpu_ms'],
          error: () => undefined,
        };
      },
    },
    [
      {
        id: 'hot_actions',
        schemaSql: 'SELECT * FROM guinea_profile_hot_actions LIMIT 0',
        sql: 'CREATE PERFETTO TABLE guinea_profile_hot_actions AS SELECT action, estimated_cpu_ms FROM source',
      },
    ],
    'memory definitions',
  );
  assert.deepEqual(queries, [
    'CREATE PERFETTO TABLE guinea_profile_hot_actions AS SELECT action, estimated_cpu_ms FROM source',
    'SELECT * FROM guinea_profile_hot_actions LIMIT 0',
  ]);
  assert.deepEqual(sources, [
    {
      id: 'hot_actions',
      tableName: 'guinea_profile_hot_actions',
      columns: ['action', 'estimated_cpu_ms'],
    },
  ]);
});

test('memory definitions load once and SQL errors abort initialization', async () => {
  const queries: string[] = [];
  await assert.rejects(
    prepareDatasets(
      {
        async query(sql) {
          queries.push(sql);
          return {
            columns: () => [],
            error: () =>
              sql.startsWith('CREATE') ? 'missing scope data' : undefined,
          };
        },
      },
      [
        {
          id: 'growth',
          schemaSql: 'SELECT * FROM guinea_profile_growth LIMIT 0',
          sql: 'CREATE PERFETTO TABLE guinea_profile_growth AS SELECT 1',
        },
      ],
      'memory definitions',
    ),
    /missing scope data/,
  );
  assert.deepEqual(queries, [
    'memory definitions',
    'CREATE PERFETTO TABLE guinea_profile_growth AS SELECT 1',
  ]);
});
