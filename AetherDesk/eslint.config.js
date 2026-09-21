/**
 * Configurazione ESLint (flat config, ESLint 9 + typescript-eslint 8).
 *
 * Perché arriva adesso: nel codice c'erano già tre `eslint-disable-next-line
 * react-hooks/exhaustive-deps` (StoreView) ma nessun linter installato —
 * commenti inerti, che facevano credere a una verifica inesistente. È la stessa
 * classe di problema del `SyncStatusModal` orfano: un meccanismo di controllo
 * che nessuno esegue.
 *
 * Politica dei livelli, pensata per stare dentro `build.cmd`:
 *
 * - **error** → ciò che rompe il comportamento o il contratto: hook chiamati
 *   fuori dalle regole, variabili/import inutilizzati, `no-undef`, condizioni
 *   costanti. `npm run lint:ci` (usato dalla build) gira con `--quiet`, quindi
 *   in build compaiono SOLO questi e il build fallisce.
 * - **warn** → ciò che vale la pena vedere ma non deve bloccare un rilascio:
 *   dipendenze mancanti negli effect (il codice ha effect "solo al mount"
 *   volutamente documentati) e `any` nei `catch`, che qui è una scelta
 *   sistematica per riportare il messaggio d'errore IPC all'utente.
 *   `npm run lint` le mostra tutte.
 */
import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';

export default tseslint.config(
  {
    // Build artifacts, backend Rust (ha i propri lint via cargo) e script Node
    // di servizio: non sono codice applicativo frontend.
    ignores: ['dist/**', 'src-tauri/**', 'scripts/**', 'node_modules/**'],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
      ],
      '@typescript-eslint/no-explicit-any': 'warn',
      'no-constant-binary-expression': 'error',
      'no-fallthrough': 'error',
    },
  },
);
