// Concept 09: restraint.
async function dragTo(page, from, to) {
  const a = await from.boundingBox(); const b = await to.boundingBox();
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2); await page.mouse.down();
  for (let i = 1; i <= 14; i++) { await page.mouse.move(a.x + a.width / 2 + ((b.x + b.width / 2) - (a.x + a.width / 2)) * i / 14, a.y + a.height / 2 + ((b.y + b.height / 2) - (a.y + a.height / 2)) * i / 14); await page.waitForTimeout(16); }
  await page.mouse.up(); await page.waitForTimeout(400);
}
export async function interact(page, name) {
  const wells = page.locator(".well");
  if (name === "macbook") {
    await dragTo(page, page.getByRole("button", { name: "Place atlas in the square" }), wells.nth(4));
    await dragTo(page, page.getByRole("button", { name: "Place birch in the square" }), wells.nth(0));
  }
  await page.getByRole("button", { name: "Place cedar in the square" }).click(); await page.waitForTimeout(300);
  await page.getByRole("button", { name: "Place ember in the square" }).click(); await page.waitForTimeout(700);
}
export async function section2(page) {
  await page.locator("#review").scrollIntoViewIfNeeded();
  await page.evaluate(() => document.querySelector("#review").scrollIntoView());
  await page.getByRole("button", { name: /Author key/ }).click(); await page.waitForTimeout(400);
  await page.getByRole("button", { name: /Reviewer key/ }).click(); await page.waitForTimeout(1200);
}
export async function tour(page) {
  await page.evaluate(() => document.querySelector("#lease").scrollIntoView({ behavior: "smooth" })); await page.waitForTimeout(1400);
  await page.keyboard.press("Tab");
  await page.locator(".dial").focus(); for (let i = 0; i < 5; i++) { await page.keyboard.press("ArrowRight"); await page.waitForTimeout(150); }
  await page.waitForTimeout(600);
  await page.evaluate(() => document.querySelector("#review").scrollIntoView({ behavior: "smooth" })); await page.waitForTimeout(1300);
  await page.getByRole("button", { name: /Author key/ }).click(); await page.waitForTimeout(500);
  await page.getByRole("button", { name: /Reviewer key/ }).click(); await page.waitForTimeout(1300);
  await page.evaluate(() => document.querySelector("#evidence").scrollIntoView({ behavior: "smooth" })); await page.waitForTimeout(1300);
  await page.getByRole("button", { name: "Seal the evidence" }).click(); await page.waitForTimeout(1800);
  for (let y = 0; y < 8; y++) { await page.evaluate(() => window.scrollBy({ top: 400, behavior: "smooth" })); await page.waitForTimeout(350); }
}
