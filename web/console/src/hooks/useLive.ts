import { useEffect, useState, useSyncExternalStore } from "react";
import { STORAGE, wsUrl } from "../core/api.ts";
import { liveActions } from "../core/bus.ts";
import { LiveConnection, type SocketLike, type ControlFrame, type EventFrame, type LiveFilter, type LiveStatus } from "../core/live.ts";
import type { SessionState } from "../core/session.ts";

export type LiveHandlers = {
  onEvent?: (frame: EventFrame) => void;
  onControl?: (frame: ControlFrame) => void;
  onPoll?: () => void;
  onRecovered?: () => void;
};

/**
 * One live connection for the signed-in workspace. It reconnects with
 * backoff, resumes from the stored token, polls while down, and reconnects
 * at once when the page comes back online or into view.
 */
export function useLive(session: SessionState, filter: Omit<LiveFilter, "workspace_id"> = {}, handlers: LiveHandlers = {}): LiveStatus {
  const [conn] = useState(
    () =>
      new LiveConnection({
        url: wsUrl(session.credentials.base),
        // A browser WebSocket is a SocketLike; its handlers take narrower events.
        openSocket: (url) => new WebSocket(url) as unknown as SocketLike,
        storage: window.localStorage,
        resumeKey: STORAGE.wsResume,
      }),
  );
  // The latest callbacks and credentials, for the next frame or reconnect.
  const token = session.credentials.pastedToken;
  const memberId = session.memberId;
  useEffect(() => {
    conn.update({ token: () => token, memberId: () => memberId, ...handlers });
  });

  // Screens send presence through the one connection.
  useEffect(() => {
    liveActions.setPresence = (status) => conn.sendPresence(status);
    return () => {
      liveActions.setPresence = () => false;
    };
  }, [conn]);

  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const key = JSON.stringify(filter);
  useEffect(() => {
    if (!workspace) return;
    conn.connect({ workspace_id: workspace, ...(JSON.parse(key) as Omit<LiveFilter, "workspace_id">) });
    return () => conn.disconnect();
  }, [conn, workspace, key]);

  // Signing out ends the resume point too, as the board's sign-out does.
  const signedOut = session.status === "signed-out";
  useEffect(() => {
    if (signedOut) conn.forgetResume();
  }, [conn, signedOut]);

  useEffect(() => {
    const wake = () => {
      if (document.visibilityState === "visible") conn.reconnectNow();
    };
    window.addEventListener("online", wake);
    document.addEventListener("visibilitychange", wake);
    return () => {
      window.removeEventListener("online", wake);
      document.removeEventListener("visibilitychange", wake);
    };
  }, [conn]);

  return useSyncExternalStore(conn.subscribe, conn.getStatus);
}
