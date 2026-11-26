import { beforeEach, describe, expect, it, vi } from 'vitest';
import { enrichPcDrm, type PcDrmEnrichableGame } from './usePcDrmEnrichment';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const game = (appId: string, name: string, extra: Partial<PcDrmEnrichableGame> = {}): PcDrmEnrichableGame => ({
  appId,
  name,
  ...extra,
});

describe('enrichPcDrm', () => {
  beforeEach(() => invokeMock.mockReset());

  it('filters invalid appIds together with names so pairs stay aligned and syncs has_denuvo', async () => {
    invokeMock.mockResolvedValue({ 4435490: 'Denuvo' });

    const out = await enrichPcDrm([
      game('not-a-number', 'Broken Entry'),
      game('4435490', 'Call of Duty: Modern Warfare 4', { has_denuvo: false }),
    ]);

    expect(invokeMock).toHaveBeenCalledWith('check_pc_drm_bulk', {
      appIds: [4435490],
      names: ['Call of Duty: Modern Warfare 4'],
    });
    expect(out[1].drm_protection).toBe('Denuvo');
    expect(out[1].has_denuvo).toBe(true);
  });
});
