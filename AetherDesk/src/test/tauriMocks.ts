import { vi } from 'vitest';

type EventHandler = (event: { payload: unknown }) => void;
type CloseHandler = (event: { preventDefault: () => void }) => void;

export const invokeMock = vi.fn();
export const listenMock = vi.fn();
export const onCloseRequestedMock = vi.fn();

const eventHandlers = new Map<string, Set<EventHandler>>();
let closeHandler: CloseHandler | null = null;

listenMock.mockImplementation(async (eventName: string, handler: EventHandler) => {
  const handlers = eventHandlers.get(eventName) ?? new Set<EventHandler>();
  handlers.add(handler);
  eventHandlers.set(eventName, handlers);
  return () => handlers.delete(handler);
});

onCloseRequestedMock.mockImplementation(async (handler: CloseHandler) => {
  closeHandler = handler;
  return () => { if (closeHandler === handler) closeHandler = null; };
});

export function emitTauriEvent<T>(eventName: string, payload: T) {
  for (const handler of eventHandlers.get(eventName) ?? []) handler({ payload });
}

export function requestWindowClose() {
  const preventDefault = vi.fn();
  closeHandler?.({ preventDefault });
  return { preventDefault };
}

export function resetTauriMocks() {
  invokeMock.mockReset();
  listenMock.mockClear();
  onCloseRequestedMock.mockClear();
  eventHandlers.clear();
  closeHandler = null;

  listenMock.mockImplementation(async (eventName: string, handler: EventHandler) => {
    const handlers = eventHandlers.get(eventName) ?? new Set<EventHandler>();
    handlers.add(handler);
    eventHandlers.set(eventName, handlers);
    return () => handlers.delete(handler);
  });
  onCloseRequestedMock.mockImplementation(async (handler: CloseHandler) => {
    closeHandler = handler;
    return () => { if (closeHandler === handler) closeHandler = null; };
  });
}

export function mockInvokeCommands(
  handlers: Record<string, unknown | ((args: unknown) => unknown | Promise<unknown>)>,
) {
  invokeMock.mockImplementation((command: string, args?: unknown) => {
    if (!(command in handlers)) {
      return Promise.reject(new Error(`Unexpected Tauri command: ${command}`));
    }
    const handler = handlers[command];
    return Promise.resolve(typeof handler === 'function'
      ? (handler as (value: unknown) => unknown)(args)
      : handler);
  });
}
