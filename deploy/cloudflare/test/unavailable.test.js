import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  HOSTED_FOOTER_LINKS,
  isContainerUnavailable,
  UNAVAILABLE_PAGE,
  UNAVAILABLE_RETRY_SECS,
  unavailableProblem,
} from "../src/edge.js";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const plain = (status, body) => new Response(body, { status, headers: { "content-type": "text/plain;charset=UTF-8" } });

test("the runtime's no-container answer counts as unavailable", async () => {
  assert.ok(await isContainerUnavailable(plain(500, "Error proxying request to container: The container is not running, consider calling start()")));
});

test("the server's own errors and other plain text pass through", async () => {
  const problem = new Response('{"type":"https://maidan.world/problems/internal"}', {
    status: 500,
    headers: { "content-type": "application/problem+json" },
  });
  assert.equal(await isContainerUnavailable(problem), false);
  assert.equal(await isContainerUnavailable(plain(500, "something else")), false);
  assert.equal(await isContainerUnavailable(plain(200, "Error proxying request to container")), false);
});

test("reading the body to decide leaves it for the caller", async () => {
  const res = plain(500, "Error proxying request to container: gone");
  await isContainerUnavailable(res);
  assert.match(await res.text(), /^Error proxying/);
});

test("an API client gets the overloaded problem with Retry-After", async () => {
  const res = unavailableProblem();
  assert.equal(res.status, 503);
  assert.equal(res.headers.get("content-type"), "application/problem+json");
  assert.equal(res.headers.get("retry-after"), String(UNAVAILABLE_RETRY_SECS));
  const body = await res.json();
  assert.equal(body.type, "https://maidan.world/problems/overloaded");
  assert.equal(body.status, 503);
});

test("the unavailable page retries itself and points at status and support", () => {
  const page = read(`../site${UNAVAILABLE_PAGE}.html`);
  assert.match(page, /<title>Back in a moment · Maidan<\/title>/);
  assert.match(page, new RegExp(`<meta http-equiv="refresh" content="${UNAVAILABLE_RETRY_SECS}">`));
  assert.match(page, /href="\/status"/);
  assert.match(page, /href="\/support"/);
});

test("the waitlist footer gains the hosted support and status pages, which exist", () => {
  assert.deepEqual(HOSTED_FOOTER_LINKS.map(([href]) => href), ["/support", "/status"]);
  read("../site/support.html");
});

test("a problem body sent to a browser page counts, and not to a script", async () => {
  const { isProblemForAPerson } = await import("../src/edge.js");
  const problem = new Response("{}", { status: 403, headers: { "content-type": "application/problem+json" } });
  const page = new Request("https://maidan.world/auth/oidc/callback", { headers: { "sec-fetch-dest": "document" } });
  const script = new Request("https://maidan.world/auth/oidc/callback", { headers: { "sec-fetch-dest": "empty" } });
  assert.ok(isProblemForAPerson(page, problem));
  assert.equal(isProblemForAPerson(script, problem), false);
  const ok = new Response("<html>", { status: 200, headers: { "content-type": "text/html" } });
  assert.equal(isProblemForAPerson(page, ok), false);
});

test("the problem page says the title and detail plainly and escapes them", async () => {
  const { problemPage } = await import("../src/edge.js");
  const html = problemPage(
    { title: "Forbidden", detail: "this invite is for another email address <b>" },
    "req-123",
  );
  assert.match(html, /<h1>Forbidden<\/h1>/);
  assert.match(html, /This invite is for another email address &lt;b&gt;\./);
  assert.match(html, /<code>req-123<\/code>/);
  assert.match(html, /href="\/support"/);
  assert.doesNotMatch(html, /<b>/);
  assert.match(problemPage(null, null), /Something went wrong/);
});
