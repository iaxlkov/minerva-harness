import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveLink, isMarkdown } from '../apps/web/src/plugins/editor/markdownPath.ts';

test('resolves relative markdown links inside the workspace only', () => {
  assert.equal(resolveLink('docs/guide/intro.md', './setup.md#install'), 'docs/guide/setup.md');
  assert.equal(resolveLink('docs/guide/intro.md', '../plugin-ui.md'), 'docs/plugin-ui.md');
  assert.equal(resolveLink('docs/intro.md', '/README.md'), 'README.md');
  assert.equal(resolveLink('README.md', '../outside.md'), null);
  assert.equal(resolveLink('README.md', 'https://example.com/a.md'), null);
  assert.equal(resolveLink('README.md', '#section'), null);
  assert.equal(resolveLink('docs/a.md', 'my%20file.md'), 'docs/my file.md');
  assert.ok(isMarkdown('Docs/README.MD') && !isMarkdown('main.rs'));
});
