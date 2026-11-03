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
