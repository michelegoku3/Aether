#!/usr/bin/env node
/**
 * Guardia di cablaggio del frontend.
 *
 * Perché esiste
 * -------------
 * `SyncStatusModal.tsx` è rimasto per mesi nel repo senza che nessun file lo
 * importasse: componente scritto, tipizzato, mai raggiungibile dalla UI. Né
 * `tsc` né Vite lo segnalano, perché un modulo non importato non è un errore —
 * è solo codice morto. Questa guardia rende quel caso un fallimento di build.
 *
 * Cosa verifica
 * -------------
 * 1. **Raggiungibilità**: partendo da `src/main.tsx`, ogni file in `src/` deve
 *    essere raggiungibile attraverso la catena di import. I non raggiungibili
 *    sono dead code (o un cablaggio dimenticato).
 * 2. **Collegamenti obbligatori** (`REQUIRED_LINKS`): coppie modulo →
 *    importatore che devono esistere perché una funzionalità sia davvero
 *    esposta all'utente. È la regressione puntuale sui casi già accaduti.
 *
 * Non è un type checker: risolve solo gli import relativi (`./`, `../`).
 * Alias, import dinamici con template string e moduli esterni sono ignorati.
 *
 * Uso: `npm run check:wiring` (eseguito da build.cmd prima dei test Rust).
 */

import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const srcDir = resolve(here, '..', 'src');
const ENTRY = 'main.tsx';

/** Collegamenti che devono esistere: [modulo, importatore atteso]. */
const REQUIRED_LINKS = [
  // Issue #3: il popup di stato del sincronizzatore era dead code.
  ['src/modals/SyncStatusModal.tsx', 'src/views/AetherView.tsx'],
  // Il popup vive in AetherView, montata da MainContent (App tiene le viste
  // sempre montate e le nasconde con display:none).
  ['src/views/AetherView.tsx', 'src/layout/MainContent.tsx'],
  // Hook di polling condiviso: usato dal popup e dal Log.
  ['src/hooks/useVisiblePolling.ts', 'src/modals/SyncStatusModal.tsx'],
  ['src/hooks/useVisiblePolling.ts', 'src/views/LogView.tsx'],
];

const extensions = ['.ts', '.tsx'];

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      walk(path, out);
      continue;
    }
    if (!extensions.some((ext) => path.endsWith(ext))) continue;
    if (path.endsWith('.d.ts')) continue; // moduli ambient: nessuno li importa
    out.push(path);
  }
  return out;
}

/** Risolve uno specifier relativo a un file sorgente esistente, o null. */
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
    // Solo sorgenti TS: `import './style.css'` e le altre risorse non fanno
    // parte del grafo dei moduli (altrimenti entrerebbero nel conteggio dei
    // raggiungibili senza essere file scansionati).
    if (!extensions.some((ext) => candidate.endsWith(ext))) continue;
    return candidate;
  }
  return null;
}

/**
 * Estrae gli specifier di import/export da un file sorgente.
 *
 * Copre: `import … from 'x'`, `import 'x'`, `export … from 'x'`,
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

/** importatore -> [file importati] */
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
  console.error(`[wiring] entry point mancante: ${toRepoStyle(entry)}`);
  process.exit(1);
}

/** Raggiungibilità dall'entry point (BFS). */
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

/** Chi importa un dato file. */
const importersOf = (target) =>
  files.filter((file) => file !== target && (graph.get(file) ?? []).includes(target));

const problems = [];

// 1. File non raggiungibili dall'entry point.
const unreachable = files.filter((file) => !reachable.has(file)).sort();
for (const file of unreachable) {
  const names = exportedNames(sources.get(file));
  problems.push(
    `${toRepoStyle(file)} non è raggiungibile da ${ENTRY}` +
      (names.length > 0 ? ` (esporta: ${names.join(', ')})` : ' (nessuna export)')
  );
}

// 2. Collegamenti obbligatori.
for (const [modulePath, importerPath] of REQUIRED_LINKS) {
  const target = resolve(here, '..', modulePath);
  const importer = resolve(here, '..', importerPath);
  if (!existsSync(target)) {
    problems.push(`${modulePath} (collegamento obbligatorio) non esiste`);
    continue;
  }
  if (!existsSync(importer)) {
    problems.push(`${importerPath} (importatore obbligatorio di ${modulePath}) non esiste`);
    continue;
  }
  const direct = (graph.get(importer) ?? []).includes(target);
  const transitive = reachable.has(target);
  if (!direct && !transitive) {
    problems.push(
      `${modulePath} non è collegato: né importato da ${importerPath}, né raggiungibile da ${ENTRY}`
    );
  } else if (!direct) {
    // Ammesso (il modulo è comunque esposto), ma il collegamento dichiarato è
    // cambiato: va aggiornato REQUIRED_LINKS perché resti significativo.
    problems.push(
      `${modulePath} è raggiungibile ma non più importato da ${importerPath} ` +
        `(importatori attuali: ${importersOf(target).map(toRepoStyle).join(', ') || 'nessuno'}) ` +
        `— aggiornare REQUIRED_LINKS in scripts/check_frontend_wiring.mjs`
    );
  }
}

// ---------------------------------------------------------------------------

const summary =
  `${files.length} moduli in src/, ${reachable.size} raggiungibili da ${ENTRY}, ` +
  `${REQUIRED_LINKS.length} collegamenti obbligatori verificati.`;

if (problems.length > 0) {
  console.error('[wiring] FAIL');
  for (const problem of problems) console.error(`  - ${problem}`);
  console.error('');
  console.error(`[wiring] ${summary}`);
  console.error(
    'Un modulo non raggiungibile è dead code: collegalo alla UI o eliminalo. ' +
      'Se un collegamento obbligatorio è cambiato di proposito, aggiorna REQUIRED_LINKS.'
  );
  process.exit(1);
}

console.log(`[wiring] OK — ${summary}`);
