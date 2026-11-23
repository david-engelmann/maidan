// A workspace's outbound webhooks: URLs Maidan posts matching events to, each
// signed with a secret the server shows once, when the webhook is created.
// The routes take a token (a browser sign-in has no twin of them), so every
// call here is a bearer call. Plain data in, plain data out.
import { apiPath, bearerRefusal, failure, guarded, headers, writeInit, ApiError, type Credentials, type CredentialMode } from "./api.ts";

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

/** Said in place of the webhooks to a browser sign-in, which the webhook routes do not take. */
export const WEBHOOKS_NEED_TOKEN =
  "Webhooks are managed with a token, not a browser sign-in. Ask a workspace admin for a token that holds workspace:admin, then paste it under Tools, Connect.";

/** Said when the list is refused. Reading webhooks needs workspace:read. */
export const WEBHOOKS_NO_READ = "This token cannot list webhooks. Listing needs workspace:read.";

/** Under the URL field. These are the rules the server holds a webhook URL to. */
export const WEBHOOK_URL_RULE =
  "An absolute http or https URL, with no user name or password in it, at a public network address. Redirects are not followed.";

/** Said above a new webhook's secret, in place of the default sentence about a token's hash. */
export const WEBHOOK_SECRET_NOTE =
  "Shown once. Copy it now, because Maidan will not show it again. It signs every delivery to this webhook. To check one, compute the HMAC-SHA256 of the raw request body with this secret and compare sha256= and its hex digest with the X-Maidan-Signature header.";

/** The server could not make the webhook, and a 500 does not say why. The usual cause is the setting named here. */
export const WEBHOOK_CREATE_UNAVAILABLE =
  "The server could not create the webhook. Creating one needs FEDERATION_ENCRYPTION_KEY to be set on the server, so if this keeps happening, ask whoever runs it to check that.";

/** An event kind in words: `thread_result_set` reads "Thread result set". */
export function eventWord(kind: string): string {
  const words = kind.replace(/_/g, " ").trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : kind;
}

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
    const why = bearerRefusal(c);
    if (why) throw new WebhooksError(why);
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
   * does not know) and a missing permission come back in the server's own
   * sentence. The signing secret is in the answer and is not kept anywhere.
   */
  async create(workspace: string, input: NewWebhook): Promise<MintedWebhook> {
    const { c, url } = this.path("POST", `/workspaces/${workspace}/webhooks`);
    const body = JSON.stringify(webhookBody(input));
    const res = await this.fetchFn(url, writeInit({ method: "POST", headers: headers(c, true), credentials: "include", body }));
    if (!res.ok) return failure(res, this.mode(), WebhooksError, (status, detail) => (status === 500 ? WEBHOOK_CREATE_UNAVAILABLE : (status === 400 || status === 403) && detail ? detail : null));
    return (await res.json()) as MintedWebhook;
  }

  /** Revoke a webhook, so it receives no more events. */
  async revoke(workspace: string, id: string): Promise<void> {
    const { c, url } = this.path("DELETE", `/workspaces/${workspace}/webhooks/${encodeURIComponent(id)}`);
    const res = await this.fetchFn(url, writeInit({ method: "DELETE", headers: headers(c), credentials: "include" }));
    if (!res.ok) return failure(res, this.mode(), WebhooksError, (status, detail) => (status === 403 && detail ? detail : null));
  }
}
