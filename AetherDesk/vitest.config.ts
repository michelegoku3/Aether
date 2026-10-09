import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    pool: 'vmThreads',
    // Expected error-path logging stays hidden for passing tests, but Vitest
    // still prints all captured output when a test fails.
    silent: 'passed-only',
    setupFiles: ['./src/test/setup.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'html', 'lcov'],
      reportsDirectory: 'coverage',
      // Gate executable frontend behavior, not static layout/CSS presentation.
      include: [
        'src/app/*.ts',
        'src/hooks/useStoreSearch.ts',
        'src/views/settings/settingsModel.ts',
        'src/views/settings/use*.ts',
        'src/views/store/use*.ts',
        'src/views/store/StoreSearchBar.tsx',
        'src/views/store/StoreDownloadModal.tsx',
      ],
      exclude: ['src/**/*.test.{ts,tsx}', 'src/**/types.ts'],
      thresholds: {
        statements: 75,
        branches: 70,
        functions: 80,
        lines: 75,
      },
    },
  },
});
