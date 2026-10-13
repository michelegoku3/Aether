/**
 * ESLint configuration (flat config, ESLint 9 + typescript-eslint 8).
 *
 * Why it exists now: the code already had three `eslint-disable-next-line
 * react-hooks/exhaustive-deps` comments (StoreView) but no installed linter —
 * inert comments that suggested a check nobody ran. Same class of problem as
 * the orphan `SyncStatusModal`: a control mechanism nobody executes.
 *
 * Level policy, designed to fit inside `build.cmd`:
 *
 * - **error** → what breaks behavior or contracts: hooks called outside the
 *   rules, unused variables/imports, `no-undef`, constant conditions.
 *   `npm run lint:ci` (used by the build) runs with `--quiet`, so the build
 *   shows ONLY these and fails on them.
 * - **warn** → worth seeing but must not block a release: missing effect
 *   dependencies (the code has deliberately documented mount-only effects)
 *   and `any` in `catch`, which here is a systematic choice to surface the
 *   IPC error message to the user. `npm run lint` shows them all.
 */
import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';

export default tseslint.config(
  {
    // Build artifacts, the Rust backend (has its own lints via cargo) and Node
    // service scripts: not frontend application code.
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
