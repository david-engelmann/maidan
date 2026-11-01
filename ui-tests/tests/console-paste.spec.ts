import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// A file pasted into the console's composer becomes an artifact linked to the
// conversation's thread, stored under its name without the path; pasted text
// stays text.
const fx = fixtures();

test("pasting a file into the composer attaches it to the thread", async ({ page, request }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, `#/threads/t/${fx.thread_id}`);
  const box = page.locator("textarea#compose");
  await expect(box).toBeVisible();
  const uploaded = page.waitForResponse((r) => r.url().includes("/artifacts?") && r.request().method() === "POST");
  const linked = page.waitForResponse((r) => /\/threads\/[^/]+\/artifacts\//.test(r.url()) && r.request().method() === "PUT");
  await box.evaluate((el) => {
    const data = new DataTransfer();
    data.items.add(new File(["pasted bytes"], "../../etc/evil.png", { type: "image/png" }));
    el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
  });
  const res = await uploaded;
  expect(res.status()).toBe(201);
  const artifact = await res.json();
  expect(artifact.sha256).toMatch(/^[0-9a-f]{64}$/);
  expect(artifact.kind).toBe("screenshot");
  expect(artifact.filename).toBe("evil.png");
  expect(JSON.stringify(artifact)).not.toContain("etc");
  expect((await linked).ok()).toBeTruthy();
  await expect(page.locator(".composer-note")).toHaveText("Attached evil.png");
  await expect(box).toHaveValue("");

  const list = await request.get(`/threads/${fx.thread_id}/artifacts`, { headers: { Authorization: `Bearer ${fx.token}` } });
  expect(list.ok()).toBeTruthy();
  expect(JSON.stringify(await list.json())).toContain(artifact.sha256);
});

test("pasting text leaves it in the composer", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, `#/threads/t/${fx.thread_id}`);
  const box = page.locator("textarea#compose");
  await expect(box).toBeVisible();
  const kept = await box.evaluate((el) => {
    const data = new DataTransfer();
    data.setData("text/plain", "just words");
    return el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
  });
  expect(kept).toBe(true);
  await expect(page.locator(".composer-note")).toHaveText("");
});
