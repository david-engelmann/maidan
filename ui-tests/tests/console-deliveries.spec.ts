import { test, expect, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";
import { laidOutBox } from "./_layout";

// The Deliveries tab as an admin uses it: where each result went, sending a
// target that was not reached again, and the allowlist that decides where a
// result may go. The harness seeds "Deploy v9"'s result as delivered to
// acme/widgets#3, failed at acme/widgets#7, and skipped at a Slack channel the
// workspace never allowed; acme/widgets is the one allowed repository.
const fx = fixtures();

function results(page: Page): Locator {
  return page.getByRole("list", { name: "Result deliveries" });
}

function resultRow(page: Page, target: string): Locator {
  return results(page).locator("li", { hasText: target });
}

function allowlist(page: Page): Locator {
  return page.getByRole("list", { name: "Allowed destinations" });
}

async function openDeliveries(page: Page): Promise<void> {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/tools/deliveries");
  await expect(page.getByRole("heading", { name: "Results sent out" })).toBeVisible();
}

async function allow(page: Page, kind: string, selector: string): Promise<void> {
  const form = page.getByRole("form", { name: "Allow a destination" });
  await form.getByLabel("Kind").selectOption({ label: kind });
  await form.getByRole("textbox").fill(selector);
  await form.getByRole("button", { name: "Allow" }).click();
}

async function remove(page: Page, selector: string): Promise<void> {
  const row = allowlist(page).locator("li", { hasText: selector });
  await row.getByRole("button", { name: "Remove…" }).click();
  await row.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(allowlist(page).locator("li", { hasText: selector })).toHaveCount(0);
}

async function mint(request: APIRequestContext, label: string, capabilities: string[]): Promise<string> {
  const res = await request.post(`/workspaces/${fx.workspace_id}/members/${fx.member_id}/tokens`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: { label, capabilities },
  });
  expect(res.ok(), `mint: ${res.status()}`).toBeTruthy();
  return (await res.json()).secret;
}

// First, while every seeded row still has its Replay button.
test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("the Deliveries tab fits the screen and its actions stay in reach", async ({ page }) => {
    await openDeliveries(page);
    const row = resultRow(page, `Slack · ${fx.result_skipped_selector}`);
    await expect(row).toBeVisible();
    await expect(allowlist(page)).toBeVisible();
    // A phone's layout viewport widens to fit content that overflows, so
    // window.innerWidth would hide the overflow; the visual viewport does not.
    const fit = await page.evaluate(() => {
      const width = window.visualViewport?.width ?? window.innerWidth;
      const right = Math.max(...[...document.querySelectorAll(".tool-panel")].map((p) => p.getBoundingClientRect().right));
      return { width, page: document.documentElement.scrollWidth, right };
    });
    expect(fit.page).toBeLessThanOrEqual(Math.ceil(fit.width));
    expect(fit.right).toBeLessThanOrEqual(fit.width);
    for (const control of [row.getByRole("button", { name: "Replay" }), page.getByRole("button", { name: "Allow", exact: true })]) {
      await control.scrollIntoViewIfNeeded();
      const box = await laidOutBox(control);
      const scrolled = await page.evaluate(() => window.scrollX);
      expect(scrolled).toBe(0);
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(fit.width);
      expect(box.height).toBeGreaterThanOrEqual(44);
    }
  });
});

test("each result delivery shows its target, status, reason and task", async ({ page }) => {
  await openDeliveries(page);
  const delivered = resultRow(page, `GitHub · ${fx.result_delivered_selector}`);
  await expect(delivered.locator(".state-chip")).toHaveText("Delivered");
  await expect(delivered.getByRole("button", { name: "Replay" })).toHaveCount(0);

  const failed = resultRow(page, `GitHub · ${fx.result_failed_selector}`);
  await expect(failed).toBeVisible();
  await expect(failed).toContainText("GitHub answered 502 Bad Gateway");

  const skipped = resultRow(page, `Slack · ${fx.result_skipped_selector}`);
  await expect(skipped).toContainText("target not in the workspace egress allowlist");

  // The task link names the task, not its id, and opens it.
  const link = delivered.getByRole("link", { name: /Deploy v9/ });
  await expect(link).toBeVisible();
  await link.click();
  await expect(page).toHaveURL(new RegExp(`#/threads/t/${fx.thread_id}$`));
});

test("the status filter keeps only the rows in that status", async ({ page }) => {
  await openDeliveries(page);
  await expect(resultRow(page, `GitHub · ${fx.result_delivered_selector}`)).toBeVisible();
  await page.getByRole("radiogroup", { name: "Delivery status" }).getByRole("radio", { name: "Delivered" }).click();
  await expect(resultRow(page, `GitHub · ${fx.result_delivered_selector}`)).toBeVisible();
  await expect(results(page).locator("li", { hasText: `Slack · ${fx.result_skipped_selector}` })).toHaveCount(0);
  for (const chip of await results(page).locator(".state-chip").allInnerTexts()) expect(chip).toBe("Delivered");
});

test("a failed delivery to an allowed target goes out again on Replay", async ({ page }) => {
  await openDeliveries(page);
  const row = resultRow(page, `GitHub · ${fx.result_failed_selector}`);
  const chip = row.locator(".state-chip");
  await expect(chip).toHaveText(/Failed|Pending/);
  test.skip((await chip.innerText()) !== "Failed", "an earlier attempt of this test already replayed it");
  await row.getByRole("button", { name: "Replay" }).click();
  await expect(chip).toHaveText("Pending");
  await expect(row.getByRole("button", { name: "Replay" })).toHaveCount(0);
  await expect(page.getByText(`Sending again to GitHub · ${fx.result_failed_selector}`).first()).toBeVisible();
  // The server kept it: a fresh read says pending too.
  await page.reload();
  await expect(resultRow(page, `GitHub · ${fx.result_failed_selector}`).locator(".state-chip")).toHaveText("Pending");
});

test("a skipped target stays skipped until it is allowed, then Replay sends it", async ({ page }) => {
  await openDeliveries(page);
  const target = `Slack · ${fx.result_skipped_selector}`;
  const row = resultRow(page, target);
  await expect(row.locator(".state-chip")).toHaveText("Skipped");

  await row.getByRole("button", { name: "Replay" }).click();
  await expect(page.getByText("Still skipped: target not in the workspace egress allowlist").first()).toBeVisible();
  await expect(row.locator(".state-chip")).toHaveText("Skipped");

  await allow(page, "Slack channel", fx.result_skipped_selector);
  await expect(allowlist(page).locator("li", { hasText: fx.result_skipped_selector })).toBeVisible();
  await row.getByRole("button", { name: "Replay" }).click();
  await expect(row.locator(".state-chip")).toHaveText("Pending");
  await expect(page.getByText(`Sending again to ${target}`).first()).toBeVisible();

  // Leave the allowlist as the seed had it.
  await remove(page, fx.result_skipped_selector);
});

test("the allowlist lists, refuses a bad selector, adds and removes with a confirmation", async ({ page }) => {
  await openDeliveries(page);
  const seeded = allowlist(page).locator("li", { hasText: fx.allowed_repo }).first();
  await expect(seeded).toContainText("GitHub");
  await expect(seeded).toContainText(/Added .* by Operator/);

  // A #name is refused here, before anything is sent.
  const posts: string[] = [];
  page.on("request", (r) => {
    if (r.method() === "POST" && r.url().includes("/egress-targets")) posts.push(r.url());
  });
  await allow(page, "Slack channel", "#general");
  await expect(page.getByRole("alert").filter({ hasText: "not its #name" })).toBeVisible();
  await expect(page.getByRole("form", { name: "Allow a destination" }).getByRole("textbox")).toHaveAttribute("aria-invalid", "true");
  expect(posts).toEqual([]);

  // A shape the console accepts but the server never does comes back in the server's words.
  await allow(page, "GitHub branch", "acme/widgets@prod");
  await expect(page.getByRole("alert").filter({ hasText: "`prod` is never an allowed base" })).toBeVisible();
  expect(posts).toHaveLength(1);

  const repo = `ui-test/allowlist-${Date.now()}`;
  await allow(page, "GitHub repository", repo);
  const added = allowlist(page).locator("li", { hasText: repo });
  await expect(added).toContainText("by Operator");
  await expect(page.getByRole("form", { name: "Allow a destination" }).getByRole("textbox")).toHaveValue("");

  // Remove asks first, and Keep leaves it.
  await added.getByRole("button", { name: "Remove…" }).click();
  await added.getByRole("button", { name: "Keep" }).click();
  await expect(added).toBeVisible();
  await remove(page, repo);
  await expect(seeded).toBeVisible();
});

test("a token without token:admin is told who sees the allowlist, and never asks for it", async ({ page, request }) => {
  const secret = await mint(request, "deliveries-viewer", ["workspace:read"]);
  const asked: string[] = [];
  page.on("request", (r) => {
    if (r.url().includes("/egress-targets")) asked.push(r.url());
  });
  await page.goto("/ui/#/tools/deliveries");
  await page.getByLabel(/paste a token/i).fill(secret);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByText("Only a workspace admin sees where results may be delivered.")).toBeVisible();
  // A reader sees where results went, but cannot send one again.
  await expect(resultRow(page, `GitHub · ${fx.result_delivered_selector}`)).toBeVisible();
  await expect(results(page).getByRole("button", { name: "Replay" })).toHaveCount(0);
  expect(asked).toEqual([]);
});
