import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// A throwaway token narrowed from the fixture's, so rotating it breaks no
// other spec.
async function throwawayToken(request: APIRequestContext) {
  const minted = await request.post("/tokens/attenuate", {
    headers: { Authorization: `Bearer ${fx.token}` },
    data: {
      capabilities: ["workspace:read", "workspace:write", "message:post"],
      label: "token-session-spec",
    },
  });
  expect(minted.status()).toBe(201);
  return (await minted.json()) as { id: string; secret: string };
}

async function paste(page: Page, secret: string) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", secret);
  const exchanged = page.waitForResponse((r) => r.url().endsWith("/auth/session/from-token"));
  await page.locator("#token").dispatchEvent("change");
  expect((await exchanged).status()).toBe(201);
  await expect(page.locator("#identity-pill")).toBeVisible();
}

// Next #3: the page never keeps the bearer. Pasting it exchanges it for an
// HttpOnly session and empties the field; nothing in storage or document.cookie
// holds it, and the page reads and writes over the session.
test("a pasted token becomes a session the page cannot read", async ({ page, request }) => {
  const child = await throwawayToken(request);
  await paste(page, child.secret);

  await expect(page.locator("#token")).toHaveValue("");
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBeNull();
  const session = (await page.context().cookies()).find((c) => c.name === "maidan_session");
  expect(session?.httpOnly).toBe(true);
  expect(session?.sameSite).toBe("Lax");
  expect(await page.evaluate(() => document.cookie)).not.toContain("maidan_session");
  expect(await page.content()).not.toContain(child.secret);

  // A write over the session, with no bearer on the request.
  await page.click(`#channel-list li[data-id="${fx.channel_id}"]`);
  await page.click(`#board .card[data-id="${fx.thread_id}"]`);
  const body = `over the session ${Date.now()}`;
  await page.fill("#compose-body", body);
  const posted = page.waitForResponse(
    (r) => r.url().endsWith(`/threads/${fx.thread_id}/messages`) && r.request().method() === "POST",
  );
  await page.click("#post-message");
  const res = await posted;
  expect(res.status()).toBe(201);
  expect(res.request().headers()["authorization"]).toBeUndefined();
  await expect(page.locator("#message-list")).toContainText(body);

  // Rotating the token ends the session made from it.
  const rotated = await request.post(`/tokens/${child.id}/rotate`, {
    headers: { Authorization: `Bearer ${child.secret}` },
  });
  expect(rotated.status()).toBe(200);
  expect(await page.evaluate(async () => (await fetch("/me")).status)).toBe(401);
});

test("sign out ends the session and leaves nothing to sign back in with", async ({ page, request }) => {
  const child = await throwawayToken(request);
  await paste(page, child.secret);
  await expect(page.locator("#logout")).toBeVisible();

  await page.click("#logout");
  await page.waitForURL("**/ui/");
  await expect(page.locator("#session-status")).toContainText("Not signed in");
  expect(await page.evaluate(async () => (await fetch("/me")).status)).toBe(401);
  await page.reload();
  await expect(page.locator("#token")).toHaveValue("");
  await expect(page.locator("#session-status")).toContainText("Not signed in");
});
