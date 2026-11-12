import { act, cleanup, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, expect, test, vi } from "vitest";
import { reach } from "../src/core/api.ts";
import { OfflineBanner } from "../src/shell/OfflineBanner.tsx";

// UI-008: one line over every place while the console is offline, gone on reconnect.

function setOnline(online: boolean) {
  Object.defineProperty(window.navigator, "onLine", { configurable: true, get: () => online });
  window.dispatchEvent(new Event(online ? "online" : "offline"));
}

afterEach(() => {
  vi.useRealTimers();
  act(() => setOnline(true));
  reach.answered();
  cleanup();
});

test("going offline shows the stale-data line; coming back takes it away", () => {
  render(<OfflineBanner />);
  expect(screen.queryByRole("status")).toBeNull();
  act(() => setOnline(false));
  expect(screen.getByRole("status").textContent).toMatch(/^You're offline\. What you see may be out of date/);
  act(() => setOnline(true));
  expect(screen.queryByRole("status")).toBeNull();
});

test("an unreachable server shows it too, until the next answer", () => {
  render(<OfflineBanner />);
  act(() => reach.unreachable());
  expect(screen.getByRole("status").textContent).toMatch(/^Can't reach the server\./);
  act(() => reach.answered());
  expect(screen.queryByRole("status")).toBeNull();
});

test("during a steady outage the line adds the last answer's time once it is a minute old, and the timer goes with the line", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 10, 12, 9, 30));
  act(() => reach.answered());
  const { unmount } = render(<OfflineBanner />);
  act(() => reach.unreachable());
  expect(screen.getByRole("status").textContent).toMatch(/^Can't reach the server\. What you see may be out of date/);
  act(() => vi.advanceTimersByTime(59_000));
  expect(screen.getByRole("status").textContent).not.toMatch(/ is from /);
  act(() => vi.advanceTimersByTime(2_000));
  expect(screen.getByRole("status").textContent).toMatch(/What you see is from .+ and may be out of date/);
  expect(vi.getTimerCount()).toBe(0);
  act(() => reach.answered());
  act(() => reach.unreachable());
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});

test("the shell shows it over every signed-in place, and Board does not repeat it", () => {
  const shell = readFileSync("src/shell/Shell.tsx", "utf8");
  const main = shell.slice(shell.indexOf('<main className="content"'));
  expect(main.indexOf("<OfflineBanner />")).toBeGreaterThan(0);
  expect(main.indexOf("<OfflineBanner />")).toBeLessThan(main.indexOf("<motion.section"));
  const board = readFileSync("src/board/Board.tsx", "utf8");
  expect(board).toMatch(/const stale = offline \? null : staleBanner\(/);
});
