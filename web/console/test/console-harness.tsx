import { render, type RenderResult } from "@testing-library/react";
import type { ReactNode } from "react";
import { SessionContext } from "../src/hooks/useSession.ts";
import { SessionStore } from "../src/core/session.ts";

export type Route = { method?: string; match: RegExp; body: () => unknown; status?: number };

/** A fetch that answers the routes a signed-in screen reads, and records each call. */
export class FetchRouter {
  calls: { url: string; method: string }[] = [];
  constructor(private routes: Route[]) {}
  fn = async (url: string, init?: RequestInit): Promise<Response> => {
    const method = (init?.method ?? "GET").toUpperCase();
    this.calls.push({ url: String(url), method });
    const hit = this.routes.find((r) => (r.method ?? "GET") === method && r.match.test(String(url)));
    const status = hit?.status ?? (hit ? 200 : 404);
    const raw = hit ? hit.body() : { error: "not mocked" };
    const json = raw instanceof Promise ? await raw : raw;
    return new Response(JSON.stringify(json), { status, headers: { "content-type": "application/json" } });
  };
}

export function withSession(extra: Route[], member = "me", workspace = "ws"): FetchRouter {
  return new FetchRouter([
    { match: /\/auth\/session$/, body: () => ({ member_id: member, workspace_id: workspace, display_name: "Ada" }) },
    { match: /\/me$/, body: () => ({ member_id: member, workspace_id: workspace, is_bearer: false, capabilities: [], known_capabilities: [] }) },
    ...extra,
  ]);
}

/** Signs in against `fetchFn`, then renders `ui` with that session. */
export async function renderSignedIn(ui: ReactNode, fetchFn: FetchRouter["fn"]): Promise<RenderResult> {
  const store = new SessionStore("https://console.test", fetchFn);
  globalThis.fetch = fetchFn;
  await store.refresh();
  return render(<SessionContext.Provider value={store}>{ui}</SessionContext.Provider>);
}
