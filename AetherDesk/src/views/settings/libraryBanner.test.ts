import { describe, expect, it } from 'vitest';
import { formatAppIdList, isValidSteamId64, parseAppIdList } from './libraryBanner';

describe('parseAppIdList', () => {
  it('splits on commas and ignores surrounding spaces', () => {
    expect(parseAppIdList('4656000, 4108000 ,3618390').ids).toEqual([3618390, 4108000, 4656000]);
    expect(parseAppIdList('4656000, 4108000').invalid).toEqual([]);
  });

  it('drops empty tokens and deduplicates', () => {
    expect(parseAppIdList('4656000,,4108000,4656000,').ids).toEqual([4108000, 4656000]);
    expect(parseAppIdList('').ids).toEqual([]);
    expect(parseAppIdList('  ,  ').ids).toEqual([]);
  });

  it('does not treat a space as a separator', () => {
    const result = parseAppIdList('4656000 4108000');
    expect(result.ids).toEqual([]);
    expect(result.invalid).toEqual(['4656000 4108000']);
  });

  it('rejects non-numeric tokens, signed values, zero and overflow', () => {
    expect(parseAppIdList('abc').invalid).toEqual(['abc']);
    expect(parseAppIdList('4656000,abc').invalid).toEqual(['abc']);
    expect(parseAppIdList('-1').invalid).toEqual(['-1']);
    expect(parseAppIdList('+42').invalid).toEqual(['+42']);
    expect(parseAppIdList('0').invalid).toEqual(['0']);
    expect(parseAppIdList('4294967296').invalid).toEqual(['4294967296']);
  });

  it('keeps valid tokens when others are invalid', () => {
    expect(parseAppIdList('4656000, nope, 4108000')).toEqual({
      ids: [4108000, 4656000],
      invalid: ['nope'],
    });
  });
});

describe('formatAppIdList', () => {
  it('renders a canonical comma-separated list', () => {
    expect(formatAppIdList([4108000, 4656000])).toBe('4108000, 4656000');
    expect(formatAppIdList([])).toBe('');
    expect(formatAppIdList(undefined)).toBe('');
  });
});

describe('isValidSteamId64', () => {
  it('accepts a real account id, with surrounding spaces ignored', () => {
    expect(isValidSteamId64('76561199876393402')).toBe(true);
    expect(isValidSteamId64(' 76561199876393402 ')).toBe(true);
    expect(isValidSteamId64('0')).toBe(true);
    expect(isValidSteamId64('18446744073709551615')).toBe(true); // 2^64 - 1
  });

  it('rejects anything that is not a decimal unsigned 64-bit id', () => {
    expect(isValidSteamId64('')).toBe(false);
    expect(isValidSteamId64('   ')).toBe(false);
    expect(isValidSteamId64('steam')).toBe(false);
    expect(isValidSteamId64('-1')).toBe(false);
    expect(isValidSteamId64('+42')).toBe(false);
    expect(isValidSteamId64('76561199876393402.5')).toBe(false);
    expect(isValidSteamId64('18446744073709551616')).toBe(false); // 2^64
  });
});
