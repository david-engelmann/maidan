import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// A posted handoff keeps its paragraphs, lists, table and code, and a link
// only navigates when its scheme is http(s) or a console route.
const fx = fixtures();

const CODE = "echo hello\n  keep indent";

function body(mark: string): string {
  return [
    mark,
    "",
    "Second paragraph.",
    "",
    "# Heading one",
    "",
    "## Heading two",
    "",
    "### Heading three",
    "",
    "- bullet **bold** and *italic*",
    "- ~~struck~~ item",
    "",
    "1. first",
    "2. second",
    "",
    "> A quoted `snippet`",
    "",
    "| Col | Val |",
    "| --- | --- |",
    "| alpha | beta-and-a-very-wide-cell-that-must-scroll-inside-the-table |",
    "",
    "[external link](https://example.com/spec)",
    "",
    "[internal link](#/needs)",
    "",
    "[bad](javascript:alert(1))",
    "",
    "```sh",
    CODE,
    "```",
  ].join("\n");
}

async function post(page: Page, text: string): Promise<void> {
  const res = await page.request.post(`/threads/${fx.thread_id}/messages`, {
    headers: { Authorization: `Bearer ${fx.requester_token}` },
    data: { body: text },
  });
  expect(res.ok(), `post: ${res.status()}`).toBeTruthy();
}

test("a message renders structure, safe links and a copyable code block", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const mark = `md probe ${Date.now()}`;
  await post(page, body(mark));
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, `#/threads/t/${fx.thread_id}`);
  const msg = page.locator(".msg", { hasText: mark });
  await expect(msg).toBeVisible();
  await expect(msg.locator(".msg-para").filter({ hasText: mark })).toBeVisible();
  await expect(msg.locator(".msg-para").filter({ hasText: "Second paragraph." })).toBeVisible();
  await expect(msg.locator("h3")).toHaveText("Heading one");
  await expect(msg.locator("h4")).toHaveText("Heading two");
  await expect(msg.locator("h5")).toHaveText("Heading three");
  await expect(msg.locator("h1, h2")).toHaveCount(0);
  await expect(msg.locator("ul.msg-list li")).toHaveCount(2);
  await expect(msg.locator("ul.msg-list strong")).toHaveText("bold");
  await expect(msg.locator("ul.msg-list em")).toHaveText("italic");
  await expect(msg.locator("ul.msg-list del")).toHaveText("struck");
  await expect(msg.locator("ol.msg-list")).toHaveAttribute("start", "1");
  await expect(msg.locator("ol.msg-list li")).toHaveText(["first", "second"]);
  await expect(msg.locator("blockquote code")).toHaveText("snippet");
  await expect(msg.locator(".msg-table th")).toHaveText(["Col", "Val"]);
  await expect(msg.locator(".msg-table td").first()).toHaveText("alpha");
  const external = msg.locator('a[href="https://example.com/spec"]');
  await expect(external).toHaveAttribute("target", "_blank");
  await expect(external).toHaveAttribute("rel", "noopener noreferrer");
  const internal = msg.locator('a[href="#/needs"]');
  await expect(internal).toHaveCount(1);
  expect(await internal.getAttribute("target")).toBeNull();
  await expect(msg.locator('a[href^="javascript:"], a[href^="data:"], a[href^="vbscript:"], a[href^="file:"]')).toHaveCount(0);
  await expect(msg).toContainText("javascript:alert(1)");
  await expect(msg.locator(".msg-lang")).toHaveText("sh");
  await expect(msg.locator("pre code")).toHaveText(CODE);
  const copy = msg.locator("button.msg-copy");
  await expect(copy).toHaveAccessibleName("Copy code");
  await copy.click();
  await expect(copy).toHaveAccessibleName("Copied code");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(CODE);
  // The copied state is where an aria-label over the visible word stopped matching it.
  const copied = await new AxeBuilder({ page }).include(".msg-codehead").analyze();
  expect(copied.violations.map((v) => `${v.id} (${v.impact})`)).toEqual([]);

  let popped = false;
  page.on("popup", () => {
    popped = true;
  });
  await internal.click();
  await expect(page).toHaveURL(/#\/needs(?:$|[/?])/);
  expect(popped).toBe(false);

  await page.goto(`/ui/#/threads/t/${fx.thread_id}`);
  await expect(msg).toBeVisible();
  const axe = await new AxeBuilder({ page }).include(".msg-body").analyze();
  const bad = axe.violations.filter((v) => v.impact === "serious" || v.impact === "critical" || v.id === "heading-order");
  expect(bad.map((v) => `${v.id} (${v.impact})`)).toEqual([]);

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(msg).toBeVisible();
  const extra = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(extra).toBeLessThanOrEqual(0);
  const box = await copy.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.width).toBeGreaterThanOrEqual(44);
  expect(box!.height).toBeGreaterThanOrEqual(44);
  const phone = await new AxeBuilder({ page }).include(".msg-body").analyze();
  const phoneBad = phone.violations.filter((v) => v.impact === "serious" || v.impact === "critical" || v.id === "heading-order");
  expect(phoneBad.map((v) => `${v.id} (${v.impact})`)).toEqual([]);
});

test("a skipped markdown heading still steps one level at a time", async ({ page }) => {
  const mark = `md ranks ${Date.now()}`;
  await post(page, [mark, "", "# Summary", "", "### Findings", "", "##### Notes", "", "# Next", "", "#### Follow-up"].join("\n"));
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, `#/threads/t/${fx.thread_id}`);
  const msg = page.locator(".msg", { hasText: mark });
  await expect(msg).toBeVisible();
  const tags = await msg.locator("h3, h4, h5, h6").evaluateAll((els) => els.map((el) => el.tagName));
  expect(tags).toEqual(["H3", "H4", "H5", "H3", "H4"]);
  await msg.evaluate((el) => el.setAttribute("data-md-ranks", ""));
  const axe = await new AxeBuilder({ page }).include("[data-md-ranks]").withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(axe.violations.map((v) => v.id)).toEqual([]);
});
