// One live connection feeds every screen: the shell owns it and passes each
// frame here, and a screen listens for the kinds it cares about.
import type { ControlFrame, EventFrame, LiveStatus } from "./live.ts";

export type BusMessage = { type: "event"; frame: EventFrame } | { type: "resync" };
type Listener = (m: BusMessage) => void;

export class LiveBus {
  private readonly listeners = new Set<Listener>();
  listen = (fn: Listener): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };
  emit(m: BusMessage): void {
    for (const fn of this.listeners) fn(m);
  }
}

export const liveBus = new LiveBus();

type ControlListener = (frame: ControlFrame) => void;

/** Control frames (presence, typing) from the same connection. */
export class ControlBus {
  private readonly listeners = new Set<ControlListener>();
  /** The last presence snapshot, so a screen opened later starts from it. */
  last: ControlFrame | null = null;
  listen = (fn: ControlListener): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };
  emit(frame: ControlFrame): void {
    if (frame.type === "presence_snapshot") this.last = frame;
    for (const fn of this.listeners) fn(frame);
  }
}

export const controlBus = new ControlBus();

/** Set by the shell's connection: send my presence, false when not connected. */
export const liveActions: { setPresence: (status: "online" | "away") => boolean } = { setPresence: () => false };

/** The shell's live-connection status, for screens that warn when it is down. */
export class LiveStatusStore {
  private status: LiveStatus = { kind: "idle" };
  private readonly listeners = new Set<() => void>();
  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };
  getSnapshot = (): LiveStatus => this.status;
  set(status: LiveStatus): void {
    if (status === this.status) return;
    this.status = status;
    for (const fn of this.listeners) fn();
  }
}

export const liveStatus = new LiveStatusStore();
