import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

async function openFloor(page: Page, token: string) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", token);
  await page.locator("#token").dispatchEvent("change");
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.floor_channel_id}"]`);
  await expect(page.locator("#board-title")).toHaveText("# floor");
}

const mate = (page: Page, id: string) => page.locator(`#team .mate[data-member-id="${id}"]`);

// The team strip names who is on the board and what each one holds, by
// display name, and marks the viewer live.
test("the team strip names each member and what they hold", async ({ page }) => {
  await openFloor(page, fx.token);

  const deployer = mate(page, fx.requester_id);
  await expect(deployer.locator("b")).toHaveText("Deployer");
  await expect(deployer.locator("small")).toHaveText("claimed · Held: the deployer is on this");
  await expect(deployer).toHaveAttribute("data-state", "holding");
  await expect(deployer).toHaveAttribute("title", fx.requester_id);

  const me = mate(page, fx.member_id);
  await expect(me.locator("b")).toHaveText("Operator");
  await expect(me).toHaveAttribute("data-live", "true");
});

// A member seen on the socket turns live: connecting Live puts the viewer in
// the presence snapshot, and holding a task is shown with its state.
test("presence on the socket marks a member live", async ({ page }) => {
  await openFloor(page, fx.live_token);
  await page.click("#ws-connect");
  await expect(page.locator("#ws-status")).toHaveText("connected");
  await expect(mate(page, fx.member_id)).toHaveAttribute("data-live", "true");
  await expect(mate(page, fx.requester_id)).toHaveAttribute("data-live", "false");
});

// A card that changes lane moves there instead of blinking, and nothing moves
// for a reader who asked for reduced motion.
for (const reduced of [false, true]) {
  test(`a claimed card ${reduced ? "jumps (reduced motion)" : "glides"} into Working`, async ({ page }) => {
    if (reduced) await page.emulateMedia({ reducedMotion: "reduce" });
    await openFloor(page, fx.review_token);
    const inLane = (lane: string) =>
      page.locator(`#board .board-col[data-column="${lane}"] .card[data-id="${fx.floor_glide_thread_id}"]`);
    await expect(page.locator(`#board .card[data-id="${fx.floor_glide_thread_id}"]`)).toBeVisible();

    await page.evaluate(() => {
      const w = window as unknown as { __moved: string[] };
      w.__moved = [];
      const orig = Element.prototype.animate;
      Element.prototype.animate = function (this: HTMLElement, k, o) {
        if (this.classList.contains("card")) w.__moved.push(this.dataset.id || "");
        return orig.call(this, k, o);
      };
    });
    if (reduced) {
      // The same move a lane change makes, replayed with reduced motion on:
      // the card is told it came from elsewhere and must not animate.
      await page.evaluate((id) => {
        const w = window as unknown as { glideCards: (m: Map<string, unknown>) => void };
        w.glideCards(new Map([[id, { left: -400, top: -200 }]]));
      }, fx.floor_glide_thread_id);
    } else {
      const res = await page.request.post(`${fx.base_url}/threads/${fx.floor_glide_thread_id}/assignee/claim`, {
        headers: { Authorization: `Bearer ${fx.review_token}` },
        data: {},
      });
      expect(res.ok()).toBeTruthy();
    }
    if (!reduced) {
      await page.evaluate(() => (window as unknown as { loadThreads: () => Promise<void> }).loadThreads());
    }
    const moved = await page.evaluate(() => (window as unknown as { __moved: string[] }).__moved);
    if (reduced) {
      expect(moved).toEqual([]);
    } else {
      await expect(inLane("working")).toBeVisible();
      expect(moved).toContain(fx.floor_glide_thread_id);
      await expect(mate(page, fx.member_id).locator("small")).toContainText("Glide me");
    }
  });
}
