import { describe, expect, it } from 'vitest';
import {
  getErrorCode,
  isSettingsConflict,
  isUserCancellation,
  needsHubcapKey,
  toErrorMessage,
} from './errors';

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

describe('getErrorCode', () => {
  it('parses every control-flow code with its body intact', () => {
    expect(getErrorCode('SETTINGS_CONFLICT: k changed; retry')).toBe('SETTINGS_CONFLICT');
    expect(getErrorCode(new Error('GAME_STATE_CONFLICT: f changed; retry'))).toBe(
      'GAME_STATE_CONFLICT',
    );
    expect(getErrorCode('HUBCAP_KEY_REQUIRED_FOR_UPDATES: need key')).toBe(
      'HUBCAP_KEY_REQUIRED_FOR_UPDATES',
    );
  });

  it('never misfires on ordinary text', () => {
    for (const plain of [
      'Steam installation path is required',
      'C:\\Games\\Foo',
      '12:45:40.924 something happened',
      'ERR: too short',
      '',
    ]) {
      expect(getErrorCode(plain)).toBeNull();
    }
  });

  it('drives the typed predicates', () => {
    expect(isSettingsConflict('SETTINGS_CONFLICT: retry')).toBe(true);
    expect(isSettingsConflict('something else failed')).toBe(false);
    expect(needsHubcapKey('HUBCAP_KEY_REQUIRED_FOR_UPDATES: need key')).toBe(true);
    expect(needsHubcapKey('SETTINGS_CONFLICT: retry')).toBe(false);
  });
});
