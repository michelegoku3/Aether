import { describe, expect, it } from 'vitest';
import { isUserCancellation, toErrorMessage } from './errors';

describe('toErrorMessage', () => {
  it('returns Error messages verbatim', () => {
    expect(toErrorMessage(new Error('boom'))).toBe('boom');
  });

  it('returns strings as-is (Tauri backend errors)', () => {
    expect(toErrorMessage('backend says no')).toBe('backend says no');
  });

  it('stringifies numbers and nullish values', () => {
    expect(toErrorMessage(42)).toBe('42');
    expect(toErrorMessage(null)).toBe('null');
    expect(toErrorMessage(undefined)).toBe('undefined');
  });

  it('never throws, even on hostile values', () => {
    const hostile = { toString() { throw new Error('nope'); } };
    expect(toErrorMessage(hostile)).toBe('Unknown error');
  });
});

describe('isUserCancellation', () => {
  it('detects dialog cancellation', () => {
    expect(isUserCancellation('No file selected')).toBe(true);
    expect(isUserCancellation(new Error('No file selected'))).toBe(true);
  });

  it('rejects real failures', () => {
    expect(isUserCancellation('Failed to read theme')).toBe(false);
    expect(isUserCancellation(new Error('boom'))).toBe(false);
  });
});
