import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools } from "./_tools";

const fx = fixtures();

// A search reply that cannot be read is a sentence in the results, not an
// unhandled rejection that leaves the last results on screen.
test("an unreadable search reply says so in the results", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.locator("#token").dispatchEvent("change");
  await page.route(/\/search\?/, (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: "<html>a proxy page</html>" }),
  );
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="search"]');
  await page.fill("#search-q", "anything");
  await page.click("#run-search");
  await expect(page.locator("#search-results")).toContainText("is not JSON");
  expect(errors).toEqual([]);
});
