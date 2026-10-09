import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const knipCli = resolve(scriptDir, '..', 'node_modules', 'knip', 'bin', 'knip.js');

// Oxc raw transfer reserves a 6 GiB virtual ArrayBuffer. That optimization can
// fail in constrained CI/build environments even for this small project; the
// regular parser path is deterministic and avoids the oversized reservation.
const result = spawnSync(process.execPath, [knipCli, ...process.argv.slice(2)], {
  stdio: 'inherit',
  env: { ...process.env, KNIP_DISABLE_RAW_TRANSFER: '1' },
});

if (result.error) {
  console.error(`[check:unused] Failed to start Knip: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
