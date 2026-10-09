import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach, vi } from 'vitest';
import { resetTauriMocks } from './tauriMocks';

vi.mock('@tauri-apps/api/core', async () => {
  const mocks = await import('./tauriMocks');
  return { invoke: mocks.invokeMock };
});
vi.mock('@tauri-apps/api/event', async () => {
  const mocks = await import('./tauriMocks');
  return { listen: mocks.listenMock };
});
vi.mock('@tauri-apps/api/window', async () => {
  const mocks = await import('./tauriMocks');
  return {
    getCurrentWindow: () => ({ onCloseRequested: mocks.onCloseRequestedMock }),
  };
});

if (!HTMLElement.prototype.scrollTo) {
  HTMLElement.prototype.scrollTo = vi.fn();
}
if (!HTMLElement.prototype.scrollIntoView) {
  HTMLElement.prototype.scrollIntoView = vi.fn();
}

afterEach(() => {
  cleanup();
  document.getElementById('aether-custom-css')?.remove();
  document.getElementById('aether-personal-wallpaper')?.remove();
  document.getElementById('aether-personal-wallpaper-style')?.remove();
  document.body.classList.remove('aether-wallpaper-enabled');
  resetTauriMocks();
  vi.useRealTimers();
});
