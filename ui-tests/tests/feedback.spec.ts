import { test, expect, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// One feedback surface: showError(message, severity) puts an error, a
// warning or a success in #toasts. There is no #status line at the bottom of
// a closed panel. #toasts and the session line are live regions, so a screen
// reader hears both; an error or a warning interrupts, a success does not.

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });

interface Provisioned {
  workspace: { id: string; name: string };
  token: { secret: string };
}

// A fresh workspace and its first admin, so a spec can write there and show
// that the first workspace sees none of it.
async function provisionWorkspace(request: APIRequestContext, name: string): Promise<Provisioned> {
  const res = await request.post("/operator/workspaces", {
    headers: bearer(fx.admin_token),
    data: { name, admin_handle: "keeper" },
  });
  expect(res.status()).toBe(201);
  return res.json();
}

async function channelNames(request: APIRequestContext, token: string, workspaceId: string): Promise<string[]> {
  const res = await request.get(`/workspaces/${workspaceId}/channels`, { headers: bearer(token) });
  expect(res.ok()).toBeTruthy();
  return (await res.json()).map((c: { name: string }) => c.name);
}

test("the feedback line and the session line are live regions, and #status is gone", async ({ page }) => {
  await page.goto("/ui/");
  const toasts = page.locator("#toasts");
  await expect(toasts).toHaveAttribute("role", "status");
  await expect(toasts).toHaveAttribute("aria-live", "polite");
  await expect(toasts).toHaveAttribute("aria-atomic", "false");
  const session = page.locator("#session-status");
  await expect(session).toHaveAttribute("role", "status");
  await expect(session).toHaveAttribute("aria-live", "polite");
  await expect(page.locator("#status")).toHaveCount(0);

  // A token that is not accepted is said in that live line.
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", "not-a-real-token");
  await page.click("#token-signin");
  await expect(session).toBeVisible();
  await expect(session).toHaveClass(/\berr\b/);
  await expect(session).toContainText("That token was not accepted");
});

test("a refusal is an alert in words, with no status code", async ({ page }) => {
  // The review token holds workspace:write, not token:admin, so the server
  // really refuses the mint.
  await signIn(page, fx.workspace_id, fx.review_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="tokens"]');
  await page.fill("#token-label", "feedback-spec");
  await page.fill("#token-caps", "workspace:read");
  await page.click("#mint-member-token");

  const refused = page.locator("#toasts .toast-error");
  await expect(refused).toHaveCount(1);
  await expect(refused).toHaveAttribute("role", "alert");
  await expect(refused).toHaveAttribute("data-severity", "error");
  await expect(refused.locator("span")).toHaveText(/^Could not mint the token: Your token is not allowed to do this/);
  await expect(refused).not.toContainText("HTTP");
  await expect(refused).not.toContainText("403");

});

test("asking to widen a grant is a warning, before any request", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="tokens"]');
  await expect(page.locator("#attenuation-ceiling")).toContainText("workspace:read");
  let posted = false;
  await page.route(/\/members\/[^/]+\/tokens$/, (route) => {
    if (route.request().method() === "POST") posted = true;
    return route.continue();
  });
  await page.fill("#token-caps", "token:admin");
  await page.click("#mint-member-token");
  const warning = page.locator("#toasts .toast-warning");
  await expect(warning).toHaveAttribute("role", "alert");
  await expect(warning.locator("span")).toHaveText("Attenuation: request exceeds your grant");
  expect(posted).toBe(false);
});

test("a second workspace's write lands there, and a write into the first is refused in words", async ({ page, request }) => {
  const other = await provisionWorkspace(request, `Feedback elsewhere ${Date.now()}`);
  const ours = `elsewhere-${Date.now()}`;
  const intruder = `intruder-${Date.now()}`;

  await signIn(page, other.workspace.id, other.token.secret);
  await page.fill("#new-channel-name", ours);
  await page.click("#create-channel");
  // A success says so politely and is gone in a few seconds.
  const done = page.locator("#toasts .toast-success", { hasText: "Channel created" });
  await expect(done).toHaveAttribute("role", "status");
  await expect(done).toHaveAttribute("data-severity", "success");
  await expect(page.locator("#channel-list")).toContainText(ours);
  await expect(done).toHaveCount(0, { timeout: 8_000 });

  // Through the API: the channel is the second workspace's alone.
  expect(await channelNames(request, other.token.secret, other.workspace.id)).toContain(ours);
  expect(await channelNames(request, fx.admin_token, fx.workspace_id)).not.toContain(ours);
  const peek = await request.get(`/workspaces/${other.workspace.id}/channels`, { headers: bearer(fx.admin_token) });
  expect([403, 404]).toContain(peek.status());

  // On the page: pointing the second workspace's token at the first and
  // creating a channel is refused, as an alert in words, and writes nothing.
  await page.click("#conn-edit");
  await page.fill("#workspace", fx.workspace_id);
  await page.locator("#workspace").dispatchEvent("change");
  await page.fill("#new-channel-name", intruder);
  await page.click("#create-channel");
  const refused = page.locator("#toasts .toast-error", { hasText: "Could not create the channel" });
  await expect(refused).toHaveAttribute("role", "alert");
  await expect(refused).not.toContainText("HTTP");
  await expect(refused).not.toContainText(/\b40[34]\b/);
  expect(await channelNames(request, fx.admin_token, fx.workspace_id)).not.toContain(intruder);
  expect(await channelNames(request, other.token.secret, other.workspace.id)).not.toContain(intruder);
});
