import { spawnSync } from 'node:child_process';
import { rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

// Negative self-test for `check:unused` (Knip): proves the gate actually
// detects dead code instead of passing blindly. Plants a file with an
// unreferenced export, expects Knip to fail naming it, removes the file,
// then expects Knip to pass again. Cleanup runs even on failure so a red
// run never poisons the working tree.
const scriptDir = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDir, '..');
const knipCli = resolve(root, 'node_modules', 'knip', 'bin', 'knip.js');
const probe = resolve(root, 'src', '__knip_selftest__.ts');

const runKnip = () =>
  spawnSync(process.execPath, [knipCli], {
    encoding: 'utf8',
    cwd: root,
    env: { ...process.env, KNIP_DISABLE_RAW_TRANSFER: '1' },
  });

let failures = 0;
try {
  writeFileSync(probe, 'export const __knipSelftestDeadExport = 42;\n');
  const caught = runKnip();
  const output = `${caught.stdout ?? ''}\n${caught.stderr ?? ''}`;
  if (caught.status === 0 || !output.includes('__knip_selftest__')) {
    console.error('[check:unused:selftest] Knip did NOT flag the planted dead export.');
    console.error(output);
    failures += 1;
  } else {
    console.log('[check:unused:selftest] planted dead export detected.');
  }
} finally {
  rmSync(probe, { force: true });
}

const clean = runKnip();
if (clean.status !== 0) {
  console.error('[check:unused:selftest] Knip fails on the clean tree:');
  console.error(`${clean.stdout ?? ''}\n${clean.stderr ?? ''}`);
  failures += 1;
} else {
  console.log('[check:unused:selftest] clean tree passes.');
}

process.exit(failures === 0 ? 0 : 1);
