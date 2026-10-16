import { test, expect } from "@playwright/test";
import { openMoreTools } from "./_tools";

// WCAG-AA keyboard operability in a real browser (Cluster 353.4): the tab bar is
// a proper ARIA tablist (roles + roving tabindex + arrow-key navigation) and a
// skip link jumps to the main content. No fixtures needed — this is static
// chrome, exercised without authentication.
test("the tab bar is a keyboard-operable ARIA tablist with a skip link", async ({ page }) => {
  await page.goto("/ui/");
  await openMoreTools(page);

  // Tabs and panels carry linked ARIA roles. More tools opens on Work.
  const workTab = page.locator("#tab-work");
  await expect(workTab).toHaveAttribute("role", "tab");
  await expect(workTab).toHaveAttribute("aria-selected", "true");
  await expect(workTab).toHaveAttribute("aria-controls", "panel-work");
  await expect(page.locator("#panel-work")).toHaveAttribute("role", "tabpanel");

  // Arrow-key navigation: from the selected tab, ArrowRight moves focus and
  // activates the next tab.
  await workTab.focus();
  await page.keyboard.press("ArrowRight");
  const swarmTab = page.locator("#tab-swarm");
  await expect(swarmTab).toBeFocused();
  await expect(swarmTab).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#panel-swarm")).toHaveClass(/active/);
  // Roving tabindex: only the selected tab is in the tab order.
  await expect(swarmTab).toHaveAttribute("tabindex", "0");
  await expect(workTab).toHaveAttribute("tabindex", "-1");

  // Arrows cross group boundaries: the last Work tab steps into People.
  await page.locator("#tab-notifications").click();
  await page.locator("#tab-notifications").focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.locator("#tab-dms")).toBeFocused();

  // The skip link targets main content and reveals itself on focus.
  const skip = page.locator("a.skip-link");
  await expect(skip).toHaveAttribute("href", "#main-content");
  await skip.focus();
  const box = await skip.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
});
