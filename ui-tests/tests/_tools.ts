import { expect, type Page } from "@playwright/test";

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

// Paste a token and wait until the page has exchanged it for a session.
// Specs that write as the signed-in person need sessionMemberId, which is
// set only after that exchange.
export async function signIn(page: Page, workspaceId: string, token: string): Promise<void> {
  await page.goto("/ui/");
  // start() checks the session on load. Pasting before that check returns
  // lets the late response clear a sign-in that already succeeded. With no
  // session the line is hidden and still says "Checking session…".
  await expect(page.locator("#session-status")).toBeHidden();
  await page.fill("#workspace", workspaceId);
  await page.fill("#token", token);
  await page.locator("#token").dispatchEvent("change");
  // A successful exchange shows Sign out. The status line is cleared again
  // once /me returns, so it is not a stable signal.
  await expect(page.locator("#logout")).toBeVisible();
}

// The board is ES modules. loadNeedsYou and loadThreads are exports, not
// window globals. import() is built in the page so the test runner does not
// rewrite it, and it is the same module instance the page already loaded.
export async function callUiExport(page: Page, file: string, name: string): Promise<void> {
  await page.evaluate(
    async ({ file, name }: { file: string; name: string }) => {
      const importer = new Function("href", "return import(href)") as (
        href: string,
      ) => Promise<Record<string, unknown>>;
      const mod = await importer(`/ui/static/${file}`);
      const fn = mod[name];
      if (typeof fn !== "function") throw new Error(`${file} does not export ${name}`);
      await fn();
    },
    { file, name },
  );
}
