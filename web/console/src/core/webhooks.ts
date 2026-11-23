// A workspace's outbound webhooks: URLs Maidan posts matching events to, each
// signed with a secret the server shows once, when the webhook is created.
// A token calls the routes directly and a browser sign-in calls their
// /ui/api twin, as the allowlist does. Plain data in, plain data out.
import { apiPath, failure, guarded, headers, writeInit, ApiError, type Credentials, type CredentialMode } from "./api.ts";

/** One webhook, as `GET /workspaces/{wid}/webhooks` lists it. Revoked ones are not listed. */
export type Webhook = {
  id: string;
  url: string;
  label?: string | null;
  event_kinds: string[];
  enabled: boolean;
  created_at: string;
  revoked_at?: string | null;
};

/** What creating one answers with. The secret is in this answer and nowhere else. */
export type MintedWebhook = { webhook: Webhook; secret: string };

export type NewWebhook = { url: string; label?: string | null; event_kinds: string[] };

export type WebhooksRead = { allowed: true; value: Webhook[] } | { allowed: false };

export class WebhooksError extends ApiError {}

type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

/** Said when the list is refused. Reading webhooks needs workspace:read. */
export const WEBHOOKS_NO_READ = "This sign-in cannot list webhooks. Listing needs workspace:read.";

/** Under the URL field. These are the rules the server holds a webhook URL to. */
export const WEBHOOK_URL_RULE =
  "An absolute http or https URL, with no user name or password in it, at a public network address. Redirects are not followed.";

/** Said above a new webhook's secret, in place of the default sentence about a token's hash. */
export const WEBHOOK_SECRET_NOTE =
  "Shown once. Copy it now, because Maidan will not show it again. It signs every delivery to this webhook. To check one, compute the HMAC-SHA256 of the raw request body with this secret and compare sha256= and its hex digest with the X-Maidan-Signature header.";

/** A bare 500 on create does not say why. The usual cause is the setting named here. A 503 says its own reason. */
export const WEBHOOK_CREATE_UNAVAILABLE =
  "The server could not create the webhook. Creating one needs FEDERATION_ENCRYPTION_KEY to be set on the server, so if this keeps happening, ask whoever runs it to check that.";

/** What a webhook is called in a sentence: its label, else its URL. */
export function webhookName(w: Pick<Webhook, "label" | "url">): string {
  return w.label?.trim() || w.url;
}

/** The request body: the URL trimmed, an empty label left out. */
export function webhookBody(input: NewWebhook): NewWebhook {
  const label = input.label?.trim();
  return { url: input.url.trim(), ...(label ? { label } : {}), event_kinds: input.event_kinds };
}

export class WebhooksApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = guarded(fetchFn, WebhooksError);
    this.creds = creds;
    this.mode = mode;
  }

  // The method names the call on the line that holds the path: check:spec reads it from there.
  private path(_method: "GET" | "POST" | "DELETE", suffix: string): { c: Credentials; url: string } {
    const c = this.creds();
    return { c, url: apiPath(c, suffix) };
  }

  /** The workspace's webhooks. A token without workspace:read is `allowed: false`, not an error. */
  async list(workspace: string): Promise<WebhooksRead> {
    const { c, url } = this.path("GET", `/workspaces/${workspace}/webhooks`);
    const res = await this.fetchFn(url, { headers: headers(c), credentials: "include" });
    if (res.status === 403) return { allowed: false };
    if (!res.ok) return failure(res, this.mode(), WebhooksError);
    return { allowed: true, value: (await res.json()) as Webhook[] };
  }

  /**
   * Add a webhook. A refusal (a URL the server will not send to, an event it
   * does not know), a missing permission and a server that cannot keep the
   * secret (503) come back in the server's own sentence. The signing secret is in the answer and is not kept anywhere.
   */
  async create(workspace: string, input: NewWebhook): Promise<MintedWebhook> {
    const { c, url } = this.path("POST", `/workspaces/${workspace}/webhooks`);
    const body = JSON.stringify(webhookBody(input));
    const res = await this.fetchFn(url, writeInit({ method: "POST", headers: headers(c, true), credentials: "include", body }));
    if (!res.ok) return failure(res, this.mode(), WebhooksError, (status, detail) => (status === 500 ? WEBHOOK_CREATE_UNAVAILABLE : (status === 400 || status === 403 || status === 503) && detail ? detail : null));
    return (await res.json()) as MintedWebhook;
  }

  /** Revoke a webhook, so it receives no more events. */
  async revoke(workspace: string, id: string): Promise<void> {
    const { c, url } = this.path("DELETE", `/workspaces/${workspace}/webhooks/${encodeURIComponent(id)}`);
    const res = await this.fetchFn(url, writeInit({ method: "DELETE", headers: headers(c), credentials: "include" }));
    if (!res.ok) return failure(res, this.mode(), WebhooksError, (status, detail) => (status === 403 && detail ? detail : null));
  }
}
