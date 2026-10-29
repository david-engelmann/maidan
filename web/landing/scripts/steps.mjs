// Concept 04: living square 3D.
export async function interact(page) {
  await page.waitForTimeout(2500);
  const s = await page.locator(".stage").boundingBox();
  await page.mouse.click(s.x + s.width * 0.3, s.y + s.height * 0.55); await page.waitForTimeout(300);
  await page.getByRole("button", { name: "+ Post a task" }).click(); await page.waitForTimeout(300);
  await page.getByRole("button", { name: "Turn the square right" }).click().catch(() => {});
  await page.waitForTimeout(4500);
}
export async function section2(page) {
  await page.evaluate(() => window.scrollTo(0, window.innerHeight * 0.75)); await page.waitForTimeout(2500);
}
export async function tour(page) {
  await page.waitForTimeout(2500);
  for (let i = 0; i < 3; i++) { await page.getByRole("button", { name: "+ Post a task" }).click(); await page.waitForTimeout(500); }
  await page.getByRole("button", { name: "Turn the square right" }).click().catch(() => {}); await page.waitForTimeout(3500);
  await page.getByRole("button", { name: "Turn the square right" }).click().catch(() => {}); await page.waitForTimeout(3000);
  for (let y = 0; y < 8; y++) { await page.evaluate(() => window.scrollBy({ top: 120, behavior: "smooth" })); await page.waitForTimeout(400); }
  await page.waitForTimeout(2500);
  for (let y = 0; y < 6; y++) { await page.evaluate(() => window.scrollBy({ top: 500, behavior: "smooth" })); await page.waitForTimeout(450); }
}
