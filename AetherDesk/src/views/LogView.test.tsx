import { act, fireEvent, render, screen } from '@testing-library/react';
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

  it('supports Pause/Resume, quick WARN/ERROR counter toggles, and Copy filtered lines', async () => {
    const lines = ['2026-01-01 [INFO ] ok', '2026-01-01 [WARN ] heads up', '2026-01-01 [ERROR] boom'];
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(command === 'get_recent_log_lines' ? [...lines] : null)
    );
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    });

    render(<LogView active />);
    await flushTick();
    expect(logCalls()).toHaveLength(1);

    // Pause halts further polling; Resume immediately polls again.
    fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
    await advance(5000);
    expect(logCalls()).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    await flushTick();
    expect(logCalls()).toHaveLength(2);

    // Quick ERROR counter filters down to ERROR lines; clicking again resets to TRACE.
    const errorPill = screen.getByRole('button', { name: 'ERROR (1)' });
    fireEvent.click(errorPill);
    expect(screen.queryByText(/\[INFO \]/)).not.toBeInTheDocument();
    expect(screen.queryByText(/\[WARN \]/)).not.toBeInTheDocument();
    expect(screen.getByText(/\[ERROR\]/)).toBeInTheDocument();

    // Copy sends filtered lines to clipboard.
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await flushTick();
    expect(writeText).toHaveBeenCalledWith('2026-01-01 [ERROR] boom');
    expect(screen.getByText('Copied 1 log line(s) to clipboard.')).toBeInTheDocument();

    fireEvent.click(errorPill);
    expect(screen.getByText(/\[INFO \]/)).toBeInTheDocument();
  });
});
