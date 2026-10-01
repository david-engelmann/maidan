import type { Page } from "@playwright/test";

// #tools starts closed. Specs that use a tab inside it open the panel the
// same way a person does, via the More tools summary. The command palette
// still opens a tab on its own through openTool.
export async function openMoreTools(page: Page): Promise<void> {
  const tools = page.locator("#tools");
  if (await tools.evaluate((el: HTMLDetailsElement) => el.open)) return;
  // Nested <details> sit inside the panel (audit, purge, DMs, and the rest).
  // locator("summary") matches every one of them, and strict mode will not
  // click fifteen summaries. The More tools summary is the direct child.
  await page.locator("#tools > summary").click();
}
