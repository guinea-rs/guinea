import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseStatePayload, stateSelectionQuery} from '../src/state-model.ts';
import {renderStateQueries} from '../scripts/sql.ts';

test('state captures keep unchanged, null, missing and failed values distinct', () => {
  const value = {
    schema_version: 1,
    update_id: 7,
    before: {status: 'json', value: null},
    after: {status: 'json', value: null},
    update: {status: 'failed', value: 'failed serializer'},
    diff: {status: 'json', value: []},
  };
  assert.deepEqual(parseStatePayload(JSON.stringify(value)), value);
  assert.equal(parseStatePayload(null), undefined);
  for (const status of ['unavailable', 'limit_exceeded', 'failed']) {
    const input = {...value, diff: {status}};
    assert.deepEqual(parseStatePayload(JSON.stringify(input))?.diff, {status});
  }
  assert.throws(() =>
    parseStatePayload(JSON.stringify({...value, schema_version: 2})),
  );
  assert.throws(() =>
    parseStatePayload(JSON.stringify({...value, before: {status: 'json'}})),
  );
});

test('state selection SQL is parameterized and rejects invalid event identities', () => {
  const sql = stateSelectionQuery(renderStateQueries().selection, 42);
  assert.match(sql, /slice_id = 42/);
  assert.doesNotMatch(sql, /__EVENT_ID__|\{\{/);
  for (const id of [-1, NaN, Infinity, 1.5])
    assert.throws(() => stateSelectionQuery(sql, id));
});
