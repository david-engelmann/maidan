import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Wave 4 #49: a file pasted into the composer becomes an artifact attached to
// the selected thread, stored by its sha; the file's name is kept only as a
// display name, without its path. Pasted text stays text.
test("pasting a file into the composer attaches it to the thread", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.channel_id}"]`);
  await page.click(`#board .card[data-id="${fx.thread_id}"]`);

  const uploaded = page.waitForResponse(
    (r) => r.url().includes("/artifacts?") && r.request().method() === "POST",
  );
  await page.locator("#compose-body").evaluate((el) => {
    const data = new DataTransfer();
    data.items.add(new File(["pasted bytes"], "../../etc/evil.png", { type: "image/png" }));
    el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
  });
  const res = await uploaded;
  expect(res.status()).toBe(201);
  const artifact = await res.json();
  expect(artifact.kind).toBe("screenshot");
  expect(artifact.sha256).toMatch(/^[0-9a-f]{64}$/);
  // The name is a name: the path the client sent is gone.
  expect(artifact.filename).toBe("evil.png");
  expect(JSON.stringify(artifact)).not.toContain("etc");

  const card = page.locator(`.artifact-card[data-sha="${artifact.sha256}"]`).first();
  await expect(card.locator(".artifact-name")).toHaveText("📎 evil.png");
  await expect(page.locator("#compose-body")).toHaveValue("");
});

test("pasting text leaves it in the composer", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.locator("#compose-body").focus();
  await page.locator("#compose-body").evaluate((el) => {
    const data = new DataTransfer();
    data.setData("text/plain", "just words");
    const ok = el.dispatchEvent(
      new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }),
    );
    (el as HTMLElement).dataset.pasteDefault = String(ok);
  });
  // The handler did not cancel a text paste.
  await expect(page.locator("#compose-body")).toHaveAttribute("data-paste-default", "true");
});
