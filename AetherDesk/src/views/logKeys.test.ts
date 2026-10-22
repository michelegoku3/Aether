import { describe, expect, it } from 'vitest';
import { stableLineKeys } from './logKeys';

describe('stableLineKeys', () => {
  it('produces identical keys across calls for the same input', () => {
    const lines = ['2026-01-01 [INFO ] first', '2026-01-01 [WARN ] second'];

    expect(stableLineKeys(lines)).toEqual(stableLineKeys(lines));
  });

  it('keeps existing keys when a line is appended', () => {
    const before = stableLineKeys(['first', 'second']);
    const after = stableLineKeys(['first', 'second', 'third']);

    expect(after.slice(0, 2)).toEqual(before);
  });

  it('gives duplicate lines distinct keys', () => {
    const keyed = stableLineKeys(['repeated', 'repeated', 'repeated']);

    expect(new Set(keyed.map((entry) => entry.key)).size).toBe(3);
  });

  it('keeps each key paired with its line', () => {
    const lines = ['alpha', 'beta', 'alpha'];

    expect(stableLineKeys(lines).map((entry) => entry.line)).toEqual(lines);
  });
});
