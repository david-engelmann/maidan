import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// Open a group DM with two other members, picked by name, and post into it.
// The signed-in person is added, which makes the three members the store
// requires.
test("opening a group DM selects it and a message shows in the conversation", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="group-dms"]');

  await page.fill("#gdm-member-search", "deploy");
  await page.click(`#gdm-member-options [role="option"][data-member-id="${fx.requester_id}"]`);
  await page.fill("#gdm-member-search", "rae");
  await page.click(`#gdm-member-options [role="option"][data-member-id="${fx.rae_member_id}"]`);
  await expect(page.locator("#gdm-picked li")).toHaveCount(2);
  await page.fill("#gdm-title", "release desk");
  await page.click("#gdm-open");
  await expect(page.locator("#gdm-selected")).toContainText("release desk");
  await expect(page.locator("#toasts .toast-success", { hasText: "Group DM opened" })).toHaveAttribute("role", "status");
  await expect(page.locator("#gdm-picked li")).toHaveCount(0);

  const body = `hello group ${Date.now()}`;
  await page.fill("#gdm-body", body);
  await page.click("#gdm-send");
  await expect(page.locator("#gdm-messages")).toContainText(body);
  await expect(page.locator("#gdm-messages")).toContainText("Operator");
});

// One other member is not a group: the page says so before it posts.
test("a group DM with one other member is refused before the request", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="group-dms"]');

  let posted = false;
  await page.route(/\/group-dms$/, (route) => {
    if (route.request().method() === "POST") posted = true;
    return route.continue();
  });
  await page.fill("#gdm-member-search", "deploy");
  await page.click(`#gdm-member-options [role="option"][data-member-id="${fx.requester_id}"]`);
  await page.click("#gdm-open");
  await expect(page.locator("#toasts")).toContainText("A group DM needs at least 3 members");
  expect(posted).toBe(false);
});
