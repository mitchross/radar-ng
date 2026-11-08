import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { expect, it, vi } from "vitest";

function worker() {
  const handlers: Record<string, (event: unknown) => void> = {};
  const clients = { claim: vi.fn(), matchAll: vi.fn(), openWindow: vi.fn() };
  runInNewContext(readFileSync(new URL("../public/sw.js", import.meta.url), "utf8"), {
    self: { addEventListener: (type: string, callback: (event: unknown) => void) => { handlers[type] = callback; }, clients, skipWaiting: vi.fn() },
    encodeURIComponent,
  });
  return { handlers, clients };
}

it("focuses the existing radar and forwards the warning polygon on a notification tap", async () => {
  const { handlers, clients } = worker();
  const win = { focus: vi.fn(), postMessage: vi.fn() };
  clients.matchAll.mockResolvedValue([win]);
  const data = { kind: "nws-alert", bounds: [-86, 42, -84, 43] };
  let pending: Promise<unknown> | undefined;
  const close = vi.fn();
  handlers.notificationclick({ notification: { close, data }, waitUntil: (p: Promise<unknown>) => { pending = p; } });
  await pending;
  expect(close).toHaveBeenCalledOnce();
  expect(win.postMessage).toHaveBeenCalledWith(data);
  expect(win.focus).toHaveBeenCalledOnce();
});

it("opens a closed radar with polygon bounds encoded in its URL", async () => {
  const { handlers, clients } = worker();
  clients.matchAll.mockResolvedValue([]);
  let pending: Promise<unknown> | undefined;
  handlers.notificationclick({ notification: { close: vi.fn(), data: { kind: "nws-alert", bounds: [-86, 42, -84, 43] } }, waitUntil: (p: Promise<unknown>) => { pending = p; } });
  await pending;
  const url = new URL(clients.openWindow.mock.calls[0][0], "https://radar.example");
  expect(JSON.parse(url.searchParams.get("alertBounds")!)).toEqual([-86, 42, -84, 43]);
});

it("keeps existing rain notification clicks working without polygon data", async () => {
  const { handlers, clients } = worker();
  clients.matchAll.mockResolvedValue([]);
  let pending: Promise<unknown> | undefined;
  handlers.notificationclick({ notification: { close: vi.fn() }, waitUntil: (p: Promise<unknown>) => { pending = p; } });
  await pending;
  expect(clients.openWindow).toHaveBeenCalledWith("/");
});
