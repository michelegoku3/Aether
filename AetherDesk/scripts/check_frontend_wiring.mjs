#!/usr/bin/env node
/**
 * Frontend wiring guard.
 *
 * Why it exists
 * -------------
 * `SyncStatusModal.tsx` sat in the repo for months without any file
 * importing it: written, typed, never reachable from the UI. Neither
 * `tsc` nor Vite flag it, because an unimported module is not an error —
 * just dead code. This guard turns that case into a build failure.
 *
 * What it checks
 * -------------
 * 1. **Reachability**: starting from `src/main.tsx`, every file in `src/` must
 *    be reachable through the import chain. Unreachable ones are dead code
 *    (or forgotten wiring).
 * 2. **Required links** (`REQUIRED_LINKS`): module → importer pairs that must
 *    exist for a feature to be genuinely exposed to the user. Punctual
 *    regression coverage for cases that already happened.
 *
 * Not a type checker: it only resolves relative imports (`./`, `../`).
 * Aliases, dynamic imports with template strings and external modules are ignored.
 *
 * Usage: `npm run check:wiring` (run by build.cmd before the Rust tests).
 */

import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const srcDir = resolve(here, '..', 'src');
const ENTRY = 'main.tsx';

/** Links that must exist: [module, expected importer]. */
const REQUIRED_LINKS = [
  // Issue #3: the synchronizer status popup was dead code.
  ['src/modals/SyncStatusModal.tsx', 'src/views/AetherView.tsx'],
  // The popup lives in AetherView, mounted by MainContent (App keeps views
  // always mounted and hides them with display:none).
  ['src/views/AetherView.tsx', 'src/layout/MainContent.tsx'],
  // Shared polling hook: used by the popup and by the Log view.
  ['src/hooks/useVisiblePolling.ts', 'src/modals/SyncStatusModal.tsx'],
  ['src/hooks/useVisiblePolling.ts', 'src/views/LogView.tsx'],
];

const extensions = ['.ts', '.tsx'];

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      // Test modules are Vitest entry points, not production modules reachable
      // from main.tsx; production dead-code analysis intentionally excludes them.
      if (name === 'test' || name === '__tests__') continue;
      walk(path, out);
      continue;
    }
    if (!extensions.some((ext) => path.endsWith(ext))) continue;
    if (path.endsWith('.d.ts')) continue; // ambient modules: nobody imports them
    if (/\.(?:test|spec)\.[^.]+$/.test(path)) continue;
    out.push(path);
  }
  return out;
}

/** Resolve a relative specifier to an existing source file, or null. */
function resolveSpecifier(fromFile, specifier) {
  if (!specifier.startsWith('.')) return null;
  const base = resolve(dirname(fromFile), specifier);
  const candidates = [
    base,
    ...extensions.map((ext) => base + ext),
    ...extensions.map((ext) => join(base, `index${ext}`)),
  ];
  for (const candidate of candidates) {
    if (!existsSync(candidate) || !statSync(candidate).isFile()) continue;
    // TS sources only: `import './style.css'` and other assets are not part
    // of the module graph (otherwise they would enter the reachable count
    // without being scanned files).
    if (!extensions.some((ext) => candidate.endsWith(ext))) continue;
    return candidate;
  }
  return null;
}

/**
 * Extract import/export specifiers from a source file.
 *
 * Covers: `import … from 'x'`, `import 'x'`, `export … from 'x'`,
 * `import type … from 'x'`, `export type … from 'x'`, `import('x')`.
 */
function specifiersOf(source) {
  const found = [];
  const patterns = [
    /(?:^|\n)\s*(?:import|export)\s+(?:type\s+)?[\s\S]*?\sfrom\s*['"]([^'"]+)['"]/g,
    /(?:^|\n)\s*import\s*['"]([^'"]+)['"]/g,
    /\bimport\s*\(\s*['"]([^'"]+)['"]\s*\)/g,
  ];
  for (const pattern of patterns) {
    for (const match of source.matchAll(pattern)) found.push(match[1]);
  }
  return found;
}

function exportedNames(source) {
  const names = new Set();
  for (const match of source.matchAll(/export\s+(?:declare\s+)?(?:const|let|function|class|type|interface|enum)\s+([A-Za-z0-9_$]+)/g)) {
    names.add(match[1]);
  }
  for (const match of source.matchAll(/export\s*\{([^}]*)\}/g)) {
    for (const part of match[1].split(',')) {
      const name = part.trim().split(/\s+as\s+/).pop();
      if (name) names.add(name.trim());
    }
  }
  if (/export\s+default/.test(source)) names.add('default');
  return [...names];
}

function toRepoStyle(path) {
  return 'src/' + relative(srcDir, path).split(sep).join('/');
}

// ---------------------------------------------------------------------------

const files = walk(srcDir);
const sources = new Map(files.map((file) => [file, readFileSync(file, 'utf8')]));

/** importer -> [imported files] */
const graph = new Map();
for (const file of files) {
  const targets = [];
  for (const specifier of specifiersOf(sources.get(file))) {
    const target = resolveSpecifier(file, specifier);
    if (target) targets.push(target);
  }
  graph.set(file, targets);
}

const entry = join(srcDir, ENTRY);
if (!sources.has(entry)) {
  console.error(`[wiring] missing entry point: ${toRepoStyle(entry)}`);
  process.exit(1);
}

/** Reachability from the entry point (BFS). */
const reachable = new Set([entry]);
const queue = [entry];
while (queue.length > 0) {
  const current = queue.shift();
  for (const target of graph.get(current) ?? []) {
    if (reachable.has(target)) continue;
    reachable.add(target);
    queue.push(target);
  }
}

/** Who imports a given file. */
const importersOf = (target) =>
  files.filter((file) => file !== target && (graph.get(file) ?? []).includes(target));

const problems = [];

// 1. Files unreachable from the entry point.
const unreachable = files.filter((file) => !reachable.has(file)).sort();
for (const file of unreachable) {
  const names = exportedNames(sources.get(file));
  problems.push(
    `${toRepoStyle(file)} is unreachable from ${ENTRY}` +
      (names.length > 0 ? ` (exports: ${names.join(', ')})` : ' (no exports)')
  );
}

// 2. Required links.
for (const [modulePath, importerPath] of REQUIRED_LINKS) {
  const target = resolve(here, '..', modulePath);
  const importer = resolve(here, '..', importerPath);
  if (!existsSync(target)) {
    problems.push(`${modulePath} (required link) does not exist`);
    continue;
  }
  if (!existsSync(importer)) {
    problems.push(`${importerPath} (required importer of ${modulePath}) does not exist`);
    continue;
  }
  const direct = (graph.get(importer) ?? []).includes(target);
  const transitive = reachable.has(target);
  if (!direct && !transitive) {
    problems.push(
      `${modulePath} is not wired: neither imported by ${importerPath}, nor reachable from ${ENTRY}`
    );
  } else if (!direct) {
    // Tolerated (the module is still exposed), but the declared link has
    // changed: REQUIRED_LINKS must be updated to stay meaningful.
    problems.push(
      `${modulePath} is reachable but no longer imported by ${importerPath} ` +
        `(current importers: ${importersOf(target).map(toRepoStyle).join(', ') || 'none'}) ` +
        `— update REQUIRED_LINKS in scripts/check_frontend_wiring.mjs`
    );
  }
}

// ---------------------------------------------------------------------------

const summary =
  `${files.length} modules in src/, ${reachable.size} reachable from ${ENTRY}, ` +
  `${REQUIRED_LINKS.length} required links verified.`;

if (problems.length > 0) {
  console.error('[wiring] FAIL');
  for (const problem of problems) console.error(`  - ${problem}`);
  console.error('');
  console.error(`[wiring] ${summary}`);
  console.error(
    'An unreachable module is dead code: wire it into the UI or delete it. ' +
      'If a required link changed on purpose, update REQUIRED_LINKS.'
  );
  process.exit(1);
}

console.log(`[wiring] OK — ${summary}`);
