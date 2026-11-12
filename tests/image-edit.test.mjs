import test from 'node:test';
import assert from 'node:assert/strict';
import { markAt, scaledSize, commentsText } from '../apps/web/src/harness/imageEdit.ts';

test('erases the topmost mark under the pointer', () => {
  const marks = [
    { kind: 'stroke', color: 'red', width: 4, points: [{ x: 0, y: 0 }, { x: 100, y: 0 }] },
    { kind: 'pin', at: { x: 50, y: 40 }, text: 'here' },
  ];
  assert.equal(markAt(marks, { x: 50, y: 3 }, 4), 0);
  assert.equal(markAt(marks, { x: 52, y: 41 }, 6), 1);
  assert.equal(markAt(marks, { x: 50, y: 20 }, 4), -1);
});

test('scales down only and numbers non-empty comments', () => {
  assert.deepEqual(scaledSize(1000, 500, 0.5), { width: 500, height: 250 });
  assert.deepEqual(scaledSize(3, 3, 2), { width: 3, height: 3 });
  assert.equal(commentsText('a.png', [{ kind: 'pin', at: { x: 0, y: 0 }, text: ' fix ' }, { kind: 'pin', at: { x: 1, y: 1 }, text: '' }]), 'Комментарии к a.png:\n1. fix');
  assert.equal(commentsText('a.png', [{ kind: 'pin', at: { x: 0, y: 0 }, text: '' }, { kind: 'pin', at: { x: 1, y: 1 }, text: 'x' }]), 'Комментарии к a.png:\n2. x');
  assert.equal(commentsText('a.png', []), '');
});
