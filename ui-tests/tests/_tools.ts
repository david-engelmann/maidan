import type { Page } from "@playwright/test";

// #tools starts closed. Specs that use a tab inside it open the panel the
// same way a person does, via the More tools summary. The command palette
// still opens a tab on its own through openTool.
export async function openMoreTools(page: Page): Promise<void> {
  const tools = page.locator("#tools");
  if (await tools.evaluate((el: HTMLDetailsElement) => el.open)) return;
  await tools.locator("summary").click();
}
