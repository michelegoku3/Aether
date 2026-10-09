import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const packageLock = JSON.parse(readFileSync(resolve(root, 'package-lock.json'), 'utf8'));
const cargoLock = readFileSync(resolve(root, 'src-tauri', 'Cargo.lock'), 'utf8');

const apiVersion = packageLock.packages?.['node_modules/@tauri-apps/api']?.version;
const cliVersion = packageLock.packages?.['node_modules/@tauri-apps/cli']?.version;
const tauriBlock = cargoLock
  .split('[[package]]')
  .find((block) => /^name = "tauri"$/m.test(block));
const rustVersion = tauriBlock?.match(/^version = "([^"]+)"$/m)?.[1];

if (!apiVersion || !cliVersion || !rustVersion) {
  console.error('[tauri-versions] Could not read Tauri versions from package-lock.json/Cargo.lock.');
  process.exit(1);
}

const majorMinor = (version) => version.split('.').slice(0, 2).join('.');
const lines = new Set([majorMinor(apiVersion), majorMinor(cliVersion), majorMinor(rustVersion)]);
if (lines.size !== 1) {
  console.error(
    `[tauri-versions] Mismatch: Rust tauri ${rustVersion}, ` +
    `@tauri-apps/api ${apiVersion}, @tauri-apps/cli ${cliVersion}.\n` +
    'Keep all three on the same major/minor release line before building.',
  );
  process.exit(1);
}

console.log(
  `[tauri-versions] OK — Rust ${rustVersion}, API ${apiVersion}, CLI ${cliVersion}.`,
);
