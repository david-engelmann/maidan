// One live connection feeds every screen: the shell owns it and passes each
// frame here, and a screen listens for the kinds it cares about.
import type { EventFrame } from "./live.ts";

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
