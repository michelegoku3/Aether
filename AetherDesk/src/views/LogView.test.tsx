import { act, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LogView } from './LogView';
import { invokeMock } from '../test/tauriMocks';

/** Poll calls only: the view issues other IPC commands from user actions. */
const logCalls = () => invokeMock.mock.calls.filter(([command]) => command === 'get_recent_log_lines');

/** Let the in-flight polling tick settle (invoke resolves on a microtask). */
const flushTick = async () => {
  await act(async () => {
    await Promise.resolve();
  });
};

/** Advance timers inside `act` so the poll chain can reschedule itself. */
const advance = async (ms: number) => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
};

describe('LogView polling', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockResolvedValue([]);
  });

  it('does not poll while another Aether tab is active', async () => {
    render(<LogView active={false} />);

    await advance(6000);

    expect(logCalls()).toHaveLength(0);
  });

  it('polls immediately when the Logs tab becomes active', async () => {
    const { rerender } = render(<LogView active={false} />);
    await advance(6000);
    expect(logCalls()).toHaveLength(0);

    rerender(<LogView active />);
    await flushTick();

    expect(logCalls()).toHaveLength(1);
  });

  it('stops polling when the Logs tab becomes inactive', async () => {
    const { rerender } = render(<LogView active />);
    await flushTick();
    expect(logCalls()).toHaveLength(1);

    rerender(<LogView active={false} />);
    await advance(10000);

    expect(logCalls()).toHaveLength(1);
  });
});

describe('LogView rendering', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  it('keeps the same row elements when a poll returns identical lines', async () => {
    const lines = ['2026-01-01 [INFO ] first', '2026-01-01 [INFO ] second'];
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(command === 'get_recent_log_lines' ? [...lines] : null)
    );

    const { container } = render(<LogView active />);
    await flushTick();

    const before = Array.from(container.querySelectorAll('.log-line'));
    expect(before).toHaveLength(2);

    // A second poll returns the same text; the rows must be updated in place,
    // not remounted, so scroll position and selection survive.
    await advance(2500);
    expect(logCalls()).toHaveLength(2);

    const after = Array.from(container.querySelectorAll('.log-line'));
    expect(after).toEqual(before);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
  });
});
