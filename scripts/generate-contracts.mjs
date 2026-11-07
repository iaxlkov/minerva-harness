import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../', import.meta.url));
const output = new URL('../packages/client/src/generated.ts', import.meta.url);
const result = spawnSync('cargo', ['run', '--quiet', '-p', 'minerva-contracts'], {
  cwd: root, encoding: 'utf8', env: { ...process.env, TS_RS_LARGE_INT: 'number' },
  maxBuffer: 16 * 1024 * 1024,
});
if (result.status !== 0) { process.stderr.write(result.stderr || String(result.error)); process.exit(result.status || 1); }
const bytes = Buffer.from(result.stdout.replaceAll('\r\n', '\n').replace(/[\t ]+$/gm, ''));
if (process.argv.includes('--check')) {
  let existing;
  try { existing = readFileSync(output); } catch { existing = Buffer.alloc(0); }
  if (!existing.equals(bytes)) { console.error('Generated contracts differ. Run npm run contracts:generate.'); process.exit(1); }
} else { writeFileSync(output, bytes); }
