import { describe, expect, it } from 'vitest';
import { matchesStoreBadgeFilter } from './useStoreBadgeFilter';

const game = (overrides: Partial<Parameters<typeof matchesStoreBadgeFilter>[0]> = {}) =>
  ({
    id: 1,
    name: 'Test',
    appId: '1',
    has_manifest: false,
    has_denuvo: false,
    ...overrides,
  }) as Parameters<typeof matchesStoreBadgeFilter>[0];

describe('matchesStoreBadgeFilter', () => {
  it('accepts any game when the filter is all', () => {
    expect(matchesStoreBadgeFilter(game({ has_denuvo: true }), 'all')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ uco2_supported: true }), 'all')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ crack_status: 'cracked' }), 'all')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'None' }), 'all')).toBe(true);
  });

  it('matches denuvo by the explicit flag', () => {
    expect(matchesStoreBadgeFilter(game({ has_denuvo: true }), 'denuvo')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ has_denuvo: false }), 'denuvo')).toBe(false);
  });

  it('matches denuvo by drm_protection containing "denuvo"', () => {
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'Denuvo' }), 'denuvo')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'Steam + Denuvo' }), 'denuvo')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'None' }), 'denuvo')).toBe(false);
    expect(matchesStoreBadgeFilter(game(), 'denuvo')).toBe(false);
  });

  it('matches uco2 when supported', () => {
    expect(matchesStoreBadgeFilter(game({ uco2_supported: true }), 'uco2')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ uco2_supported: false }), 'uco2')).toBe(false);
    expect(matchesStoreBadgeFilter(game(), 'uco2')).toBe(false);
  });

  it('matches cracked statuses', () => {
    expect(matchesStoreBadgeFilter(game({ crack_status: 'cracked' }), 'cracked')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ crack_status: 'hypervisor' }), 'cracked')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ crack_status: 'hv-standalone' }), 'cracked')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ crack_status: 'uncracked' }), 'cracked')).toBe(false);
    expect(matchesStoreBadgeFilter(game({ crack_status: 'unreleased' }), 'cracked')).toBe(false);
    expect(matchesStoreBadgeFilter(game(), 'cracked')).toBe(false);
  });

  it('matches drm_free only when drm_protection is explicitly none', () => {
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'None' }), 'drm_free')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'none' }), 'drm_free')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'STEAM' }), 'drm_free')).toBe(false);
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'Denuvo' }), 'drm_free')).toBe(false);
    expect(matchesStoreBadgeFilter(game(), 'drm_free')).toBe(false);
  });

  it('is case-insensitive for drm_protection', () => {
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'DENUVO' }), 'denuvo')).toBe(true);
    expect(matchesStoreBadgeFilter(game({ drm_protection: 'NONE' }), 'drm_free')).toBe(true);
  });

  it('treats missing drm_protection as unknown, not drm_free', () => {
    expect(matchesStoreBadgeFilter(game({ drm_protection: undefined }), 'drm_free')).toBe(false);
  });
});
