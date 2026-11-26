import { describe, expect, it } from 'vitest';
import { formatDrmNotice, priceSummary, steamDeckCompatSummary } from './gameInfoFormat';

describe('priceSummary', () => {
  it('prefers the Steam-formatted string verbatim', () => {
    expect(priceSummary({ currency: 'IDR', formattedFinal: 'Rp 150.000' })).toBe('Rp 150.000');
  });

  it('uses the currency code when no symbol mapping exists', () => {
    expect(priceSummary({ currency: 'IDR', finalCents: 15000000 })).toBe('IDR 150000.00');
  });

  it('never invents a currency for a missing code', () => {
    expect(priceSummary({ finalCents: 1999 })).toBe('19.99');
  });

  it('keeps the known symbols', () => {
    expect(priceSummary({ currency: 'usd', finalCents: 1999 })).toBe('$19.99');
    expect(priceSummary({ currency: 'JPY', finalCents: 980 })).toBe('¥980');
  });

  it('reports free games and missing prices', () => {
    expect(priceSummary(null, true)).toBe('Free to Play');
    expect(priceSummary(null)).toBe('N/A');
  });
});

describe('steamDeckCompatSummary', () => {
  it('maps Steam Deck categories and falls back to N/A', () => {
    expect(steamDeckCompatSummary(3)).toBe('Verified');
    expect(steamDeckCompatSummary(2)).toBe('Playable');
    expect(steamDeckCompatSummary(1)).toBe('Unsupported');
    expect(steamDeckCompatSummary(0)).toBe('N/A');
    expect(steamDeckCompatSummary(null)).toBe('N/A');
  });
});

describe('formatDrmNotice', () => {
  it('turns HTML breaks and entities into readable plain text', () => {
    expect(formatDrmNotice('Denuvo Anti-Tamper<br>5 machine activation limit')).toBe(
      'Denuvo Anti-Tamper · 5 machine activation limit'
    );
    expect(formatDrmNotice('EA App &amp; <strong>Denuvo</strong>')).toBe('EA App & Denuvo');
    expect(formatDrmNotice('   ')).toBe('N/A');
    expect(formatDrmNotice(null)).toBe('N/A');
  });
});
