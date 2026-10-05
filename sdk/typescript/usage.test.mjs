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

for (const f of providers) {
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

test("every provider has a fixture", () => {
  for (const p of USAGE_PROVIDERS) {
    assert.ok(providers.some((f) => f.provider === p), `no fixture for ${p}`);
  }
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
