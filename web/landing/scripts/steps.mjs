// Concept 08: it feels like an app.
async function swipe(page, loc, dx, dy) {
  const a = await loc.boundingBox(); const x = a.x + a.width / 2, y = a.y + a.height / 2;
  await page.mouse.move(x, y); await page.mouse.down();
  for (let i = 1; i <= 12; i++) { await page.mouse.move(x + dx * i / 12, y + dy * i / 12); await page.waitForTimeout(16); }
  await page.mouse.up(); await page.waitForTimeout(700);
}
const card = (page, id) => page.locator(".tcard", { hasText: id });
export async function interact(page) {
  await swipe(page, card(page, "T-21"), 160, 0);
  await swipe(page, card(page, "T-22"), 160, 0);
  await swipe(page, card(page, "T-21"), 0, -90);
  await page.waitForTimeout(900);
}
export async function section2(page) {
  await page.getByRole("button", { name: /Approve T-21/ }).click(); await page.waitForTimeout(500);
  await page.evaluate(() => document.querySelector("#receipts").scrollIntoView());
  await page.waitForTimeout(500);
  await page.getByRole("button", { name: /Fan out/ }).click(); await page.waitForTimeout(1200);
}
export async function tour(page) {
  await page.waitForTimeout(1500);
  await swipe(page, card(page, "T-21"), 160, 0); await page.waitForTimeout(500);
  await swipe(page, card(page, "T-23"), 160, 0); await page.waitForTimeout(500);
  await swipe(page, card(page, "T-21"), 0, -90); await page.waitForTimeout(1200);
  await page.getByRole("button", { name: /Approve T-21/ }).click(); await page.waitForTimeout(1200);
  await swipe(page, card(page, "T-23"), 0, -90); await page.waitForTimeout(1000);
  await page.getByRole("button", { name: /Approve T-23/ }).click(); await page.waitForTimeout(1200);
  for (let y = 0; y < 5; y++) { await page.evaluate(() => window.scrollBy({ top: 450, behavior: "smooth" })); await page.waitForTimeout(400); }
  await page.evaluate(() => document.querySelector("#receipts").scrollIntoView({ behavior: "smooth" })); await page.waitForTimeout(1200);
  await page.getByRole("button", { name: /Fan out/ }).click(); await page.waitForTimeout(2000);
}
