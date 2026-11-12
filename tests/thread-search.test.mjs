import test from 'node:test';
import assert from 'node:assert/strict';
import { findHits } from '../apps/web/src/pages/chat/search.ts';

test('finds every case-insensitive match in reading order with context', () => {
  const messages = [{ id: 'a', text: 'Build the Plugin' }, { id: 'b', text: 'no match here' }, { id: 'c', text: 'plugin, PLUGIN' }];
  const hits = findHits(messages, ' plugin ');
  assert.deepEqual(hits.map(hit => hit.id), ['a:10', 'c:0', 'c:8']);
  assert.equal(hits[0].match, 'Plugin');
  assert.equal(hits[0].before, 'Build the ');
  assert.equal(hits[2].match, 'PLUGIN');
  assert.equal(hits[2].position, 96);
  assert.deepEqual(findHits(messages, '   '), []);
  assert.equal(findHits([{ id: 'x', text: 'aaaa' }], 'a', 3).length, 3);
});
