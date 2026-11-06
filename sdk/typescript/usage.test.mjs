// The usage normalizers against the shared fixtures in sdk/usage-fixtures/,
// which the other SDKs and the server's ledger read too (no server needed).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { USAGE_PROVIDERS, UsageError, normalizeUsage, usdMicros } from "./index.js";

const ROOT = new URL("../usage-fixtures/", import.meta.url);

function load(dir) {
  const url = new URL(`${dir}/`, ROOT);
  const names = readdirSync(url).filter((n) => n.endsWith(".json")).sort();
  assert.ok(names.length > 0, `no fixtures in ${dir}`);
  return names.map((n) => JSON.parse(readFileSync(new URL(n, url), "utf8")));
}

const providers = load("providers");
const constructed = load("constructed");

for (const f of [...providers, ...constructed]) {
  test(`normalizes ${f.name}`, () => {
    const got = normalizeUsage(f.provider, f.response, f.options ?? {});
    assert.deepEqual(got, {
      model: f.expected.model,
      tokens: f.expected.tokens,
      evidence: f.expected.evidence,
    });
    if (f.expected.usd_micros !== undefined) {
      assert.equal(usdMicros(got.tokens, f.price_snapshot), f.expected.usd_micros);
    }
  });
}

test("every provider has a recorded fixture", () => {
  for (const p of USAGE_PROVIDERS) {
    assert.ok(providers.some((f) => f.provider === p), `no fixture for ${p}`);
  }
  for (const f of providers) assert.ok(["verbatim", "transcribed"].includes(f.recorded), f.name);
  for (const f of constructed) assert.equal(f.recorded, "constructed", f.name);
});

for (const f of load("invalid")) {
  test(`refuses ${f.name}`, () => {
    assert.throws(
      () => normalizeUsage(f.provider, f.response, f.options ?? {}),
      (err) => err instanceof UsageError && err.message.includes(f.expected_error),
    );
  });
}

test("prices every charge fixture as the ledger does", () => {
  const charges = JSON.parse(readFileSync(new URL("charges.json", ROOT), "utf8"));
  for (const c of charges.cases) {
    assert.equal(usdMicros(c.tokens, c.price_snapshot), c.usd_micros, c.name);
  }
});

test("a response that names no model needs one passed in", () => {
  const response = { usage: { input_tokens: 1, output_tokens: 1 } };
  assert.throws(() => normalizeUsage("anthropic", response), /model is required/);
  assert.equal(normalizeUsage("anthropic", response, { model: "m" }).model, "m");
});

test("an unknown provider, a negative count and a fraction are refused", () => {
  assert.throws(() => normalizeUsage("nope", {}), /unknown provider/);
  for (const bad of [-1, 1.5, "7"]) {
    const response = { model: "m", usage: { prompt_tokens: bad } };
    assert.throws(() => normalizeUsage("openai-chat", response), /prompt_tokens must be a non-negative integer/);
  }
});

test("the evidence provider can be overridden for a hosted shape", () => {
  const response = { model: "m", usage: { prompt_tokens: 3, completion_tokens: 1 } };
  const got = normalizeUsage("openai-chat", response, { provider: "azure.ai.openai" });
  assert.equal(got.evidence.provider, "azure.ai.openai");
});

test("blank providers and prototype-polluted usage paths are refused", () => {
  assert.throws(() => normalizeUsage("  ", { model: "m", usage: {} }), /provider is required/);
  assert.throws(() => normalizeUsage(null, { model: "m", usage: {} }), /provider is required/);
  const polluted = { model: "m", usage: JSON.parse('{"prompt_tokens": 3, "completion_tokens": 1}') };
  Object.setPrototypeOf(polluted.usage, { cached_tokens: 99 });
  // Own-property walk ignores inherited decoys on nested detail objects.
  const nested = {
    model: "m",
    usage: {
      prompt_tokens: 3,
      completion_tokens: 1,
      prompt_tokens_details: Object.assign(Object.create({ cached_tokens: 99 }), {}),
    },
  };
  const got = normalizeUsage("openai-chat", nested);
  assert.equal(got.tokens.cache_read, 0);
  assert.throws(
    () => normalizeUsage("openai-chat", { model: "m", usage: { prompt_tokens: 1, completion_tokens: 0 } }, { provider: "  " }),
    /evidence provider is required/,
  );
});
