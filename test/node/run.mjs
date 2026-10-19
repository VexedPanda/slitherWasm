// Bundles a TypeScript test (with rolldown, from Vite) and runs it in Node: node test/node/run.mjs <test.ts> [args]
import { spawnSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { build } from 'rolldown';

const [input, ...args] = process.argv.slice(2);
// Inside the repository, so the external scenerystack imports resolve from node_modules
const outputDirectory = resolve(import.meta.dirname, '.out');
mkdirSync(outputDirectory, { recursive: true });
const output = join(outputDirectory, 'test.mjs');
await build({
  input: resolve(input),
  platform: 'node',
  external: [/^scenerystack/],
  output: { file: output, format: 'esm' },
  logLevel: 'silent',
});
const result = spawnSync(
  process.execPath,
  ['--no-warnings', '--import', resolve(import.meta.dirname, 'stubs.mjs'), output, ...args],
  { stdio: 'inherit', cwd: resolve(import.meta.dirname, '../..') },
);
process.exit(result.status ?? 1);
