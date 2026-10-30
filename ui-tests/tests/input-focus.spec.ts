import { test, expect } from "@playwright/test";

// A text field took focus with only a 12% green glow, because `input:focus
// { outline: none }` came after the shared focus ring. Keyboard focus on a
// field now draws the same 3px ring as every other control.
for (const width of [390, 1440]) {
  test(`a focused sign-in field shows the 3px focus ring (${width})`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/ui/");
    await page.keyboard.press("Tab");
    const field = page.locator("#workspace");
    await field.focus();
    await expect(field).toBeFocused();
    await expect(field).toHaveCSS("outline-style", "solid");
    await expect(field).toHaveCSS("outline-width", "3px");
  });
}
