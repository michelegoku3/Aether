import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { StoreCurrency } from '../../hooks/useSettings';
import { SettingsStoreSection } from './SettingsStoreSection';

const baseProps = () => ({
  showStoreDlcs: true,
  setShowStoreDlcs: vi.fn(),
  showStoreDelisted: false,
  setShowStoreDelisted: vi.fn(),
  showStoreNsfw: false,
  setShowStoreNsfw: vi.fn(),
  showStoreFrontGames: true,
  setShowStoreFrontGames: vi.fn(),
  storeFrontFilter: 'trending',
  setStoreFrontFilter: vi.fn(),
  storeCurrency: 'eur' as StoreCurrency,
  setStoreCurrency: vi.fn(),
});

describe('SettingsStoreSection', () => {
  it('groups the store front criterion under the conditional rail', () => {
    const on = render(<SettingsStoreSection {...baseProps()} />);
    const rail = on.container.querySelector('.settings-sub-options');
    expect(rail?.textContent).toContain('Store front criterion');

    const off = render(<SettingsStoreSection {...baseProps()} showStoreFrontGames={false} />);
    expect(off.container.querySelector('.settings-sub-options')).toBeNull();
    expect(off.container.textContent).not.toContain('Store front criterion');
  });
});
