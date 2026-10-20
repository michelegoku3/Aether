#!/usr/bin/env node
/**
 * Documentation guard — the "check:wiring" of the docs tree.
 *
 * Why it exists
 * -------------
 * Documents drift silently: an index can point at a plan that was deleted, a
 * relative link can survive a file rename, and a cited path can move without
 * anyone noticing. An agent that follows such a reference lands nowhere, and
 * nothing in the build complains. This guard turns those three cases into a
 * failed build.
 *
 * What it checks
 * --------------
 * 1. **Index coverage**: every Markdown file in the repository is linked from
 *    `docs/INDEX.md` (README, AGENTS and the index itself are the entry points
 *    and may live outside it).
 * 2. **Links**: every relative Markdown link resolves to an existing file, and
 *    every `#anchor` exists as a heading in the target file.
 * 3. **Cited paths**: every backticked path rooted at a top-level folder
 *    (`AetherDesk/`, `AetherDLL/`, `Tools/`, `docs/`, `Licenses/`) exists.
 *    Partial paths (relative to a module) are not checked: they are shorthand,
 *    not references.
 *
 * Usage: `npm run check:docs` from `AetherDesk/`, or `node Tools/check_docs.mjs`
 * from the repository root.
 */

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const INDEX = 'docs/INDEX.md';
const ENTRY_POINTS = new Set(['README.md', 'AGENTS.md', INDEX]);
const ROOT_DIRS = ['AetherDesk', 'AetherDLL', 'tools', 'docs', 'licenses'];
const ROOT_FILES = ['build_desk.cmd', 'build_dll.cmd'];
const SKIP_DIRS = new Set([
  'node_modules',
  'target',
  'out',
  'dist',
  '.git',
  '.github',
  '.vs',
  'bin',
  'obj',
]);

const toRepoPath = (absolute) => relative(root, absolute).split(sep).join('/');

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (SKIP_DIRS.has(name)) continue;
      walk(path, out);
      continue;
    }
    if (name.toLowerCase().endsWith('.md')) out.push(path);
  }
  return out;
}

/** GitHub-style heading anchors, including explicit `{#id}` suffixes. */
function headingAnchors(source) {
  const anchors = new Set();
  for (const line of source.split(/\r?\n/)) {
    const match = /^\s{0,3}#{1,6}\s+(.*)$/.exec(line);
    if (!match) continue;
    const explicit = /\{#([^}]+)\}\s*$/.exec(match[1]);
    const text = explicit ? match[1].replace(/\{#[^}]+\}\s*$/, '') : match[1];
    anchors.add(slug(text));
  }
  return anchors;
}

function slug(text) {
  return text
    .trim()
    .toLowerCase()
    .replace(/<[^>]*>/g, '')
    .replace(/[`*_~]/g, '')
    .replace(/[^\p{L}\p{N}\s-]/gu, '')
    .replace(/ /g, '-');
}

function isExternal(target) {
  return /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(target);
}

const files = walk(root).sort();
const sources = new Map(files.map((file) => [file, readFileSync(file, 'utf8')]));
const problems = [];

// ---------------------------------------------------------------------------
// 1. Every document is reachable from the index.
// ---------------------------------------------------------------------------
const indexPath = join(root, INDEX);
const indexSource = sources.get(indexPath) ?? '';
const indexed = new Set();
for (const match of indexSource.matchAll(/\]\(([^)\s]+)\)/g)) {
  const target = match[1].split('#')[0];
  if (!target || isExternal(target)) continue;
  indexed.add(resolve(dirname(indexPath), target));
}
if (!existsSync(indexPath)) {
  problems.push(`${INDEX} is missing`);
}
for (const file of files) {
  const repoPath = toRepoPath(file);
  if (ENTRY_POINTS.has(repoPath)) continue;
  if (!indexed.has(file)) {
    problems.push(`${repoPath} is not linked from ${INDEX}`);
  }
}

// ---------------------------------------------------------------------------
// 2. Relative links resolve, anchors included.
// ---------------------------------------------------------------------------
for (const [file, source] of sources) {
  for (const match of source.matchAll(/\]\(([^)\s]+)\)/g)) {
    const raw = match[1];
    if (isExternal(raw)) continue;
    const [target, anchor] = raw.split('#');
    const resolved = target ? resolve(dirname(file), target) : file;
    if (target && !existsSync(resolved)) {
      problems.push(`${toRepoPath(file)} links to a missing file: ${raw}`);
      continue;
    }
    if (!anchor) continue;
    if (!resolved.toLowerCase().endsWith('.md') || !sources.has(resolved)) {
      problems.push(`${toRepoPath(file)} links to an anchor outside the docs tree: ${raw}`);
      continue;
    }
    if (!headingAnchors(sources.get(resolved)).has(anchor)) {
      problems.push(`${toRepoPath(file)} links to a missing heading: ${raw}`);
    }
  }
}

// ---------------------------------------------------------------------------
// 3. Cited repository paths exist.
// ---------------------------------------------------------------------------
const rooted = (token) => ROOT_FILES.includes(token) || ROOT_DIRS.includes(token.split('/')[0].toLowerCase());

for (const [file, source] of sources) {
  for (const match of source.matchAll(/`([^`\n]+)`/g)) {
    const token = match[1].trim();
    if (!rooted(token)) continue;
    if (token.includes('<') || token.includes('>') || token.includes(' ')) continue;
    const wildcard = token.indexOf('*');
    const candidate = wildcard === -1 ? token : token.slice(0, wildcard);
    if (!existsSync(join(root, candidate))) {
      problems.push(`${toRepoPath(file)} cites a missing path: ${token}`);
    }
  }
}

// ---------------------------------------------------------------------------

if (problems.length > 0) {
  console.error('[docs] FAIL');
  for (const problem of problems) console.error(`  - ${problem}`);
  console.error('');
  console.error(
    `[docs] ${files.length} document(s) checked, ${indexed.size} linked from ${INDEX}.`
  );
  console.error(
    'Fix the reference, or link the new document from docs/INDEX.md. ' +
      'A document reachable from nowhere is invisible to everyone.'
  );
  process.exit(1);
}

console.log(
  `[docs] OK — ${files.length} document(s), ${indexed.size} linked from ${INDEX}, ` +
    'links and cited paths resolve.'
);
