import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Cluster 368.3 (Wave 2 #16): the waiting-on-you inbox in the Work tab. The seeded
// fixture has a pending approval gate in the workspace, so the member's inbox has
// at least one waiting item — a real fetch→render.
test("the Work tab loads the waiting-on-you inbox", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  // No session in the harness → the inbox resolves the member from the actor id
  // (act-as-any bearer). The actor input lives in a non-active panel, so set its
  // value directly (authorId() reads it regardless of visibility).
  await page.evaluate((id) => {
    (document.getElementById("actor-id") as HTMLInputElement).value = id;
  }, fx.member_id);

  await page.click('.tabs button[data-tab="work"]');
  await page.click("#waiting-refresh");

  await expect(page.locator("#waiting-summary")).toContainText("waiting");
});
