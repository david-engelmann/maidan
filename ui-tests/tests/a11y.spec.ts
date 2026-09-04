import { test, expect } from "@playwright/test";

// WCAG-AA keyboard operability in a real browser (Cluster 353.4): the tab bar is
// a proper ARIA tablist (roles + roving tabindex + arrow-key navigation) and a
// skip link jumps to the main content. No fixtures needed — this is static
// chrome, exercised without authentication.
test("the tab bar is a keyboard-operable ARIA tablist with a skip link", async ({ page }) => {
  await page.goto("/ui/");

  // Tabs and panels carry linked ARIA roles.
  const adminTab = page.locator("#tab-admin");
  await expect(adminTab).toHaveAttribute("role", "tab");
  await expect(adminTab).toHaveAttribute("aria-controls", "panel-admin");
  await expect(page.locator("#panel-admin")).toHaveAttribute("role", "tabpanel");

  // Arrow-key navigation: from the selected tab, ArrowRight moves focus and
  // activates the next tab.
  await adminTab.focus();
  await page.keyboard.press("ArrowRight");
  const sessionTab = page.locator("#tab-session");
  await expect(sessionTab).toBeFocused();
  await expect(sessionTab).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#panel-session")).toHaveClass(/active/);
  // Roving tabindex: only the selected tab is in the tab order.
  await expect(sessionTab).toHaveAttribute("tabindex", "0");
  await expect(adminTab).toHaveAttribute("tabindex", "-1");

  // The skip link targets main content and reveals itself on focus.
  const skip = page.locator("a.skip-link");
  await expect(skip).toHaveAttribute("href", "#main-content");
  await skip.focus();
  const box = await skip.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
});
