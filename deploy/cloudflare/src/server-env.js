// Environment forwarded into the hosted container.
//
// The container keeps whatever `start()` passed it, and an image rollout can
// leave that process running. A setting that first appears in the same deploy
// (dynamic client registration, for one) is then absent until something
// starts the container again: discovery omits `registration_endpoint` and
// `POST /oauth/register` answers 404. `containerEnvPlan` says when to recycle
// before the next request, and to remember the environment only once that
// start has succeeded.
//
// `container.start` accepts only strings. A numeric or boolean binding would
// fail the start, so the previous container would keep running without the
// new value. The hosted configs set the registration switch to `1`; the
// server treats `1` and `true` as on.
//
// The fingerprint is stored only after the container is running. Until then a
// second request on this Durable Object can see `running` and a missing
// fingerprint and would recycle the container the first request just started.
// A start this instance already has in flight for the same fingerprint is
// the environment being applied, so it is left running. Once it has been
// verified, later requests skip the lock.

import { routedEgressHosts } from "./edge.js";

export const ENV_FINGERPRINT_KEY = "containerEnvFingerprint";

// Worker secrets and vars the server reads. Anything else in `env` (the
// Durable Object binding) stays in the Worker.
export const SERVER_ENV = [
  "DATABASE_URL",
  "MAIDAN_CONTENT_KEK",
  "MAIDAN_SESSION_SECRET",
  "MAIDAN_PUBLIC_ORIGIN",
  "MAIDAN_OAUTH_CLIENT_METADATA_HOSTS",
  "MAIDAN_OAUTH_DYNAMIC_REGISTRATION",
  "MAIDAN_OAUTH_DCR_REDIRECT_HOSTS",
  "ARTIFACT_BACKEND",
  "S3_ENDPOINT",
  "S3_BUCKET",
  "S3_REGION",
  "S3_ACCESS_KEY_ID",
  "S3_SECRET_ACCESS_KEY",
  "MAIDAN_OIDC_ENABLED",
  "MAIDAN_OIDC_ISSUER",
  "MAIDAN_OIDC_CLIENT_ID",
  "MAIDAN_OIDC_CLIENT_SECRET",
  "MAIDAN_OIDC_REDIRECT_URI",
  "MAIDAN_OIDC_LINK_EMAIL",
  "MAIDAN_OIDC_EMAILS_VERIFIED",
  "MAIDAN_SIGNUP",
  "MAIDAN_SIGNUP_EMAIL_DOMAINS",
  "MAIDAN_SIGNUP_OPEN",
  "MAIDAN_SIGNUP_PRIVACY_URL",
  "MAIDAN_SIGNUP_TERMS_URL",
  "MAIDAN_FREE_ALLOWANCE",
  "MAIDAN_MEDIA_QUOTA_BYTES",
  "MAIDAN_EXPORT_SIGNING_KEY",
  "MAIDAN_TURNSTILE_SECRET",
  "MAIDAN_TURNSTILE_SITE_KEY",
  "MAIDAN_GITHUB_TOKEN",
  "MAIDAN_GITHUB_WRITE_REPOS",
  "MAIDAN_INTERNAL_EGRESS_HOSTS",
  "MAIDAN_OPS_ALERT_CHANNEL",
  "MAIDAN_SLACK_BOT_TOKEN",
  "MAIDAN_VAPID_PRIVATE_KEY",
  "MAIDAN_VAPID_PUBLIC_KEY",
  "MAIDAN_VAPID_SUBJECT",
];

// Fixed after the forwarded bindings, so a binding cannot override them.
const FIXED_ENV = {
  MAIDAN_BIND: "0.0.0.0:8080",
  MAIDAN_ENV: "production",
  MAIDAN_LOG_FORMAT: "json",
  // The Worker rewrites X-Forwarded-For to the one address Cloudflare saw,
  // so the server trusts exactly that hop for rate limits.
  MAIDAN_TRUSTED_PROXY_HOPS: "1",
};

function envString(value) {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

export function serverEnv(env) {
  const passed = {};
  for (const name of SERVER_ENV) {
    const value = env?.[name];
    if (value === undefined || value === null) continue;
    passed[name] = envString(value);
  }
  // Only the internal names this Worker actually routes reach the server.
  const routed = routedEgressHosts(env?.MAIDAN_INTERNAL_EGRESS_HOSTS, env?.MAIDAN_INTERNAL_HOSTS, env ?? {});
  if (routed === undefined) {
    delete passed.MAIDAN_INTERNAL_EGRESS_HOSTS;
  } else {
    passed.MAIDAN_INTERNAL_EGRESS_HOSTS = routed;
  }
  return { ...passed, ...FIXED_ENV };
}

// Order-independent. The separator cannot appear in a name, so two different
// maps cannot hash the same.
export async function envFingerprint(envVars) {
  const body = Object.keys(envVars)
    .sort()
    .map((name) => `${name}=${envVars[name]}`)
    .join("\0");
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(body));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

// `remember` is written only after the container is actually running, so a
// failed start is not recorded as applied.
export function containerEnvPlan({ running, startedFingerprint, fingerprint }) {
  if (startedFingerprint === fingerprint) {
    return { destroy: false, remember: false };
  }
  return { destroy: running === true, remember: true };
}

// In-memory coordination for one container Durable Object (the hosted
// server, or the demo). Storage is shared across isolates; this state is
// not, and that is what stops a concurrent request from recycling a start
// that has not been recorded yet.
export class ContainerEnvSession {
  constructor() {
    this.verifiedFingerprint = null;
    this.startingFingerprint = null;
    this.inFlightStarts = 0;
    // Bumped when the container is destroyed from outside serve() (the
    // demo's daily reset). A request already inside forward() belongs to
    // the previous epoch: it must not record that process, or decrement a
    // start that began after the reset.
    this.epoch = 0;
  }

  invalidate() {
    this.epoch += 1;
    this.verifiedFingerprint = null;
    this.startingFingerprint = null;
    this.inFlightStarts = 0;
  }

  async serve({ fingerprint, container, storage, blockConcurrencyWhile, destroy, forward }) {
    if (this.verifiedFingerprint === fingerprint && container.running === true) {
      const epoch = this.epoch;
      try {
        return await forward();
      } finally {
        if (this.epoch === epoch && container.running !== true) this.verifiedFingerprint = null;
      }
    }

    const joined = await blockConcurrencyWhile(() =>
      this.#enter(fingerprint, container, storage, destroy),
    );
    if (!joined) {
      const epoch = this.epoch;
      try {
        return await forward();
      } finally {
        if (this.epoch === epoch && container.running !== true) this.verifiedFingerprint = null;
      }
    }

    const epoch = this.epoch;
    try {
      const response = await forward();
      // A reset landed while this start was in flight. The container it
      // saw is gone; recording it would pin the next request to a process
      // that no longer exists.
      if (this.epoch !== epoch) return response;
      if (container.running === true) {
        if (this.verifiedFingerprint !== fingerprint) {
          await storage.put(ENV_FINGERPRINT_KEY, fingerprint);
          this.verifiedFingerprint = fingerprint;
        }
      } else {
        this.verifiedFingerprint = null;
      }
      return response;
    } finally {
      // A return here would swallow the forward() error. Only the counter
      // this epoch owns is touched.
      if (this.epoch === epoch) {
        this.inFlightStarts -= 1;
        if (this.inFlightStarts === 0) this.startingFingerprint = null;
      }
    }
  }

  async #enter(fingerprint, container, storage, destroy) {
    const running = container.running === true;
    // A start this instance already began flips `running` before the
    // fingerprint is stored. That container has this environment.
    if (!(running && this.startingFingerprint === fingerprint)) {
      const started = await storage.get(ENV_FINGERPRINT_KEY);
      if (running && started === fingerprint) {
        this.verifiedFingerprint = fingerprint;
      } else if (running && started !== fingerprint) {
        await destroy();
        this.verifiedFingerprint = null;
      }
    }
    if (container.running !== true || this.startingFingerprint === fingerprint) {
      this.startingFingerprint = fingerprint;
      this.inFlightStarts += 1;
      return true;
    }
    return false;
  }
}
