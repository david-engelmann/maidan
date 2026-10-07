import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

async function openTab(page: Page, tab: "dms" | "group-dms") {
  await openMoreTools(page);
  await page.click(`.tabs button[data-tab="${tab}"]`);
}

const option = (page: Page, list: string, id: string) =>
  page.locator(`#${list} [role="option"][data-member-id="${id}"]`);

// The DM picker searches by display name or handle, marks each member agent
// or human, and never offers the signed-in person.
test("the DM picker searches members by name and handle with agent and human badges", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openTab(page, "dms");

  await page.focus("#dm-member-search");
  await expect(page.locator("#dm-member-search")).toHaveAttribute("aria-expanded", "true");
  await expect(option(page, "dm-member-options", fx.requester_id)).toBeVisible();
  await expect(option(page, "dm-member-options", fx.member_id), "the signed-in person is not offered").toHaveCount(0);

  await expect(option(page, "dm-member-options", fx.requester_id).locator(".kind-badge")).toHaveText("Agent");
  await expect(option(page, "dm-member-options", fx.rae_member_id).locator(".kind-badge")).toHaveText("Human");

  // By handle: "rae" is Rae Reviewer's handle.
  await page.fill("#dm-member-search", "RAE");
  await expect(page.locator('#dm-member-options [role="option"]')).toHaveCount(1);
  await expect(option(page, "dm-member-options", fx.rae_member_id)).toContainText("Rae Reviewer");
  await expect(option(page, "dm-member-options", fx.rae_member_id)).toContainText("@rae");

  await page.fill("#dm-member-search", "nobody-by-this-name");
  await expect(page.locator('#dm-member-options [role="option"]')).toHaveCount(0);
  await expect(page.locator("#dm-member-options")).toContainText("No member matches");

  // A hostile display name is text, never markup.
  await page.fill("#dm-member-search", "mallory");
  await expect(option(page, "dm-member-options", fx.lab_member_id)).toContainText("<script>");
  expect(await page.evaluate(() => (window as unknown as { __xss?: number }).__xss)).toBeUndefined();
});

// The keyboard alone picks a member: arrows move, Enter picks, and Escape
// closes the list.
test("the DM picker works from the keyboard", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openTab(page, "dms");

  await page.focus("#dm-member-search");
  await page.keyboard.type("deploy");
  await expect(option(page, "dm-member-options", fx.requester_id)).toHaveAttribute("aria-selected", "true");
  const active = await page.locator("#dm-member-search").getAttribute("aria-activedescendant");
  expect(active).toBeTruthy();
  await page.keyboard.press("Enter");
  await expect(page.locator(`#dm-picked [data-member-id="${fx.requester_id}"]`)).toContainText("Agent");
  await expect(page.locator("#dm-member-options")).toBeHidden();

  // Picking again replaces the one DM partner.
  await page.keyboard.type("rae");
  await page.keyboard.press("Enter");
  await expect(page.locator("#dm-picked li")).toHaveCount(1);
  await expect(page.locator(`#dm-picked [data-member-id="${fx.rae_member_id}"]`)).toContainText("Human");

  await page.keyboard.type("de");
  await expect(page.locator("#dm-member-options")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#dm-member-options")).toBeHidden();
  await expect(page.locator("#dm-member-search")).toHaveAttribute("aria-expanded", "false");
});

// The group picker keeps several members, does not offer one already picked,
// and a chip's remove button takes that member back out.
test("the group DM picker collects members as removable chips", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openTab(page, "group-dms");

  await page.fill("#gdm-member-search", "deploy");
  await option(page, "gdm-member-options", fx.requester_id).click();
  await expect(page.locator("#gdm-member-search")).toHaveValue("");
  await expect(page.locator("#gdm-member-options"), "the list closes after a pick").toBeHidden();
  await page.click("#gdm-member-search");
  await expect(option(page, "gdm-member-options", fx.rae_member_id)).toBeVisible();
  await expect(option(page, "gdm-member-options", fx.requester_id), "a picked member is not offered twice").toHaveCount(0);
  await option(page, "gdm-member-options", fx.rae_member_id).click();

  const chips = page.locator("#gdm-picked li");
  await expect(chips).toHaveCount(2);
  await expect(chips.nth(0)).toContainText("Deployer");
  await expect(chips.nth(0).locator(".kind-badge")).toHaveText("Agent");
  await expect(chips.nth(1)).toContainText("Rae Reviewer");
  await expect(chips.nth(1).locator(".kind-badge")).toHaveText("Human");

  await page.getByRole("button", { name: "Remove Deployer" }).click();
  await expect(chips).toHaveCount(1);
  await expect(page.locator("#gdm-member-search")).toBeFocused();
  await expect(page.locator("#gdm-member-options")).toBeHidden();
  await page.keyboard.press("ArrowDown");
  await expect(option(page, "gdm-member-options", fx.requester_id), "a removed member is offered again").toBeVisible();
  await expect(option(page, "gdm-member-options", fx.rae_member_id)).toHaveCount(0);

  // Backspace in an empty search box takes the last chip back out.
  await page.keyboard.press("Backspace");
  await expect(chips).toHaveCount(0);
});

// A pick made while the member list is still loading stays made, and the
// list does not open again when the load lands.
test("a pick during a slow member load keeps the list closed", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openTab(page, "dms");
  await page.focus("#dm-member-search");
  await expect(option(page, "dm-member-options", fx.requester_id)).toBeVisible();
  await page.keyboard.press("Escape");
  await page.locator("#dm-open").focus();

  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  let seen = false;
  // The held load answers with one extra member, so the test can see when
  // the page has taken the response into its member directory.
  await page.route(/\/workspaces\/[^/]+\/members$/, async (route) => {
    seen = true;
    await held;
    const res = await route.fetch();
    const rows = await res.json();
    rows.push({ id: "late-arrival", handle: "late-arrival", display_name: "Late Arrival", kind: "agent" });
    await route.fulfill({ response: res, json: rows });
  });
  await page.focus("#dm-member-search");
  await page.keyboard.type("rae");
  await page.keyboard.press("Enter");
  await expect(page.locator(`#dm-picked [data-member-id="${fx.rae_member_id}"]`)).toBeVisible();
  await expect(page.locator("#dm-member-options")).toBeHidden();
  expect(seen).toBe(true);
  release();
  // The directory update and the end of the picker's load run in one
  // microtask checkpoint, so once the page shows the extra member the
  // picker has already decided whether to reopen.
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const importer = new Function("href", "return import(href)") as (
          href: string,
        ) => Promise<{ memberDirectory: Map<string, { handle: string }> }>;
        const state = await importer("/ui/static/state.js");
        return state.memberDirectory.has("late-arrival");
      }),
    )
    .toBe(true);
  await expect(page.locator("#dm-member-options")).toBeHidden();
  await expect(page.locator("#dm-member-search")).toHaveAttribute("aria-expanded", "false");
});

// A member picked in one workspace is not offered to the next: changing the
// workspace clears both pickers.
test("changing the workspace clears both pickers", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openTab(page, "dms");
  await page.fill("#dm-member-search", "deploy");
  await option(page, "dm-member-options", fx.requester_id).click();
  await page.click('.tabs button[data-tab="group-dms"]');
  await page.fill("#gdm-member-search", "deploy");
  await option(page, "gdm-member-options", fx.requester_id).click();
  await expect(page.locator("#gdm-picked li")).toHaveCount(1);

  await page.locator("#workspace").evaluate((el: HTMLInputElement, id: string) => {
    el.value = id;
    el.dispatchEvent(new Event("change"));
  }, fx.other_workspace_id);
  await expect(page.locator("#gdm-picked li")).toHaveCount(0);
  await expect(page.locator("#dm-picked li")).toHaveCount(0);
});

// A member load that lands after the workspace changed is dropped: the
// picker never offers the previous workspace's people under the new one.
test("a member load that lands after a workspace change is dropped", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openTab(page, "dms");

  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route(/\/workspaces\/[^/]+\/members$/, async (route) => {
    await held;
    await route.continue();
  });
  const firstLoad = page.waitForResponse(new RegExp(`/workspaces/${fx.workspace_id}/members$`));
  await page.focus("#dm-member-search");
  await page.locator("#dm-open").focus();
  await page.locator("#workspace").evaluate((el: HTMLInputElement, id: string) => {
    el.value = id;
    el.dispatchEvent(new Event("change"));
  }, fx.other_workspace_id);
  release();
  await firstLoad;

  await page.focus("#dm-member-search");
  await expect(page.locator("#dm-member-options")).toContainText("Could not load this workspace's members");
  await expect(option(page, "dm-member-options", fx.requester_id)).toHaveCount(0);
});

// Two workspaces. The first opens a DM and a group DM; the second, signed in
// with its own token, is offered only its own members and lists none of the
// first's conversations. The first is never offered the second's members.
test("a second workspace's picker and DM lists show nothing of the first", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openTab(page, "dms");
  await page.focus("#dm-member-search");
  await expect(option(page, "dm-member-options", fx.requester_id)).toBeVisible();
  await expect(option(page, "dm-member-options", fx.outsider_member_id)).toHaveCount(0);
  await expect(option(page, "dm-member-options", fx.other_member_id)).toHaveCount(0);
  await page.fill("#dm-member-search", "outsider");
  await expect(page.locator("#dm-member-options")).toContainText("No member matches");
  await page.fill("#dm-member-search", "deploy");
  await option(page, "dm-member-options", fx.requester_id).click();
  await page.click("#dm-open");
  await expect(page.locator("#toasts .toast-success", { hasText: "DM opened" })).toBeVisible();

  await page.click('.tabs button[data-tab="group-dms"]');
  await page.fill("#gdm-member-search", "deploy");
  await option(page, "gdm-member-options", fx.requester_id).click();
  await page.fill("#gdm-member-search", "rae");
  await option(page, "gdm-member-options", fx.rae_member_id).click();
  await page.fill("#gdm-title", "first workspace only");
  await page.click("#gdm-open");
  await expect(page.locator("#toasts .toast-success", { hasText: "Group DM opened" })).toBeVisible();

  await page.click("#logout");
  await expect(page.locator("#logout")).toBeHidden();
  await signIn(page, fx.other_workspace_id, fx.other_token);
  await openTab(page, "dms");
  await page.focus("#dm-member-search");
  const offered = page.locator('#dm-member-options [role="option"]');
  await expect(offered).toHaveCount(1);
  await expect(option(page, "dm-member-options", fx.outsider_member_id)).toContainText("Outsider");
  for (const id of [fx.member_id, fx.requester_id, fx.rae_member_id, fx.lab_member_id]) {
    await expect(option(page, "dm-member-options", id)).toHaveCount(0);
  }
  await page.fill("#dm-member-search", "deploy");
  await expect(page.locator("#dm-member-options")).toContainText("No member matches");
  await page.click("#dm-refresh");
  await expect(page.locator("#dm-list")).toHaveText("No DMs");

  await page.click('.tabs button[data-tab="group-dms"]');
  await page.click("#gdm-refresh");
  await expect(page.locator("#gdm-list")).toHaveText("No group DMs");
  await expect(page.locator("#gdm-list")).not.toContainText("first workspace only");
});

// The same three calls the picker makes, straight at the API with the second
// workspace's token: its member list holds only its own people, it cannot
// read the first's, and it cannot open a DM or a group DM with the first's
// members.
test("the second workspace's token reaches no member or conversation of the first", async ({ request }) => {
  const auth = { Authorization: `Bearer ${fx.other_token}` };

  const own = await request.get(`/workspaces/${fx.other_workspace_id}/members`, { headers: auth });
  expect(own.ok()).toBeTruthy();
  const ids = ((await own.json()) as { id: string }[]).map((m) => m.id).sort();
  expect(ids).toEqual([fx.other_member_id, fx.outsider_member_id].sort());

  const first = await request.get(`/workspaces/${fx.workspace_id}/members`, { headers: auth });
  expect(first.status()).toBeGreaterThanOrEqual(400);
  expect(await first.text()).not.toContain("Deployer");

  const dm = await request.post(`/workspaces/${fx.other_workspace_id}/dm`, {
    headers: auth,
    data: { other_member_id: fx.requester_id },
  });
  expect(dm.status()).toBeGreaterThanOrEqual(400);

  const gdm = await request.post(`/workspaces/${fx.other_workspace_id}/group-dms`, {
    headers: auth,
    data: { member_ids: [fx.other_member_id, fx.outsider_member_id, fx.requester_id], title: null },
  });
  expect(gdm.status()).toBeGreaterThanOrEqual(400);

  const dms = await request.get(`/workspaces/${fx.other_workspace_id}/dm?member_id=${fx.other_member_id}`, {
    headers: auth,
  });
  expect(dms.ok()).toBeTruthy();
  expect(await dms.json()).toEqual([]);
  const firstDms = await request.get(`/workspaces/${fx.workspace_id}/dm?member_id=${fx.member_id}`, {
    headers: auth,
  });
  expect(firstDms.status()).toBeGreaterThanOrEqual(400);
});
