// Concept 06: talk to the swarm.
export async function interact(page) {
  await page.waitForTimeout(500);
  for (const c of ["spawn 3 agents", "claim next", "claim next", "try a conflict", "request review"]) {
    await page.getByRole("button", { name: c, exact: true }).click();
    await page.waitForTimeout(700);
  }
  await page.waitForTimeout(600);
}
export async function section2(page) {
  await page.locator("#how").scrollIntoViewIfNeeded();
  await page.waitForTimeout(1200);
}
export async function tour(page) {
  for (const c of ["approve", "close with evidence"]) {
    await page.getByRole("button", { name: c, exact: true }).click();
    await page.waitForTimeout(1100);
  }
  for (let y = 0; y < 14; y++) { await page.evaluate(() => window.scrollBy({ top: 380, behavior: "smooth" })); await page.waitForTimeout(350); }
  await page.waitForTimeout(800);
}
