import test from 'node:test';
import assert from 'node:assert/strict';
import { splitCodeBlocks, codeSlotKey } from '../apps/web/src/pages/chat/codeBlocks.ts';

test('splits fenced blocks and keeps an unclosed fence as text', () => {
  assert.deepEqual(splitCodeBlocks('Run:\n```bash\nnpm test\nnpm run build\n```\nDone'), [
    { kind: 'text', text: 'Run:\n' },
    { kind: 'code', language: 'bash', code: 'npm test\nnpm run build' },
    { kind: 'text', text: '\nDone' },
  ]);
  assert.deepEqual(splitCodeBlocks('```sh\nls'), [{ kind: 'text', text: '```sh\nls' }]);
  assert.equal(codeSlotKey('PowerShell'), 'shell');
  assert.equal(codeSlotKey(''), 'text');
  assert.equal(codeSlotKey('rust'), 'rust');
});
