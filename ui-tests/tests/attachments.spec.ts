import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// A 1x1 PNG.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

// Upload bytes and cite them from a message on the seeded thread, the way an
// agent attaches a file, then open that thread in the console.
async function attach(
  page: Page,
  body: Buffer | string,
  query: Record<string, string>,
): Promise<string> {
  const auth = { Authorization: `Bearer ${fx.token}` };
  const params = new URLSearchParams({ kind: "attachment", ...query });
  const up = await page.request.post(`/artifacts?${params}`, { headers: auth, data: body });
  expect(up.status()).toBe(201);
  const sha = (await up.json()).sha256 as string;
  const post = await page.request.post(`/threads/${fx.thread_id}/messages`, {
    headers: auth,
    data: { body: "see attached", metadata: { artifacts: [sha] } },
  });
  expect(post.status()).toBe(201);

  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.channel_id}"]`);
  await page.click(`#board .card[data-id="${fx.thread_id}"]`);
  return sha;
}

test("an image attachment renders inline under its filename", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", (r) => requests.push(r.url()));
  const sha = await attach(page, PNG, { mime_type: "image/png", filename: "diagram.png" });

  const card = page.locator(`.artifact-card[data-sha="${sha}"]`).first();
  await expect(card.locator(".artifact-name")).toHaveText("📎 diagram.png");
  const img = card.locator("img");
  await expect(img).toHaveAttribute("alt", "diagram.png");
  await expect(img).toHaveAttribute("src", /^blob:/);
  await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(1);
  // The bytes came with the token in a header; no URL carried it.
  expect(requests.some((u) => u.includes(`/artifacts/${sha}`))).toBe(true);
  expect(requests.filter((u) => u.includes(fx.token))).toEqual([]);
});

test("a file that is not a raster image shows its name and a download", async ({ page }) => {
  const sha = await attach(page, "<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>", {
    mime_type: "image/svg+xml",
    filename: "logo.svg",
  });

  const card = page.locator(`.artifact-card[data-sha="${sha}"]`).first();
  await expect(card.locator(".artifact-name")).toHaveText("📎 logo.svg");
  await expect(card.locator(".artifact-size")).toHaveText(/B$/);
  // An SVG can carry script: it is never drawn in the page.
  await expect(card.locator("img")).toHaveCount(0);

  const download = page.waitForEvent("download");
  await card.locator(".artifact-download").click();
  expect((await download).suggestedFilename()).toBe("logo.svg");
});

test("a hostile filename renders as text", async ({ page }) => {
  // No slash: the server keeps a name's last path segment, and this must
  // arrive whole.
  const hostile = `<img src=x onerror="window.__pwned=1">"><b>bold<b>.txt`;
  const sha = await attach(page, "plain words", { mime_type: "text/plain", filename: hostile });

  const card = page.locator(`.artifact-card[data-sha="${sha}"]`).first();
  await expect(card.locator(".artifact-name")).toHaveText(`📎 ${hostile}`);
  await expect(card.locator(".artifact-name img, .artifact-name b")).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined();
});
