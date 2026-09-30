import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

// A mistake the user can fix (a missing field, no token) is reported in the
// page, in a role="alert" toast, never with a blocking alert() dialog.
const fx = fixtures();

test("a missing field is a dismissible toast, not a dialog", async ({ page }) => {
  const dialogs: string[] = [];
  page.on("dialog", async (d) => {
    dialogs.push(d.message());
    await d.dismiss();
  });
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.review_token);
  await page.locator("#token").dispatchEvent("change");
  await page.fill("#new-channel-name", "");
  await page.locator("#create-channel").click();

  const toast = page.locator("#toasts [role=alert]");
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText("Channel name required");
  await page.locator("#create-channel").click();
  await expect(toast, "the same message refreshes, it does not stack").toHaveCount(1);
  await toast.getByRole("button", { name: "Dismiss" }).click();
  await expect(toast).toHaveCount(0);
  expect(dialogs).toEqual([]);
});
