// Concept 03: scripted interactions for stills and demo videos.
async function dragTo(page, from, to) {
  const a = await page.locator(from).first().boundingBox();
  const b = await page.locator(to).first().boundingBox();
  if (!a || !b) return;
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  const steps = 18;
  for (let i = 1; i <= steps; i++) {
    await page.mouse.move(a.x + a.width / 2 + ((b.x + b.width / 2 - a.x - a.width / 2) * i) / steps, a.y + a.height / 2 + ((b.y + b.height / 2 - a.y - a.height / 2) * i) / steps);
    await page.waitForTimeout(16);
  }
  await page.mouse.up();
}

async function to(page, sel, block = "center") {
  await page.locator(sel).first().evaluate((el, b) => el.scrollIntoView({ block: b, behavior: "instant" }), block);
  await page.waitForTimeout(700);
}

export async function interact(page, name, out) {
  await to(page, "#footprints .fig");
  await dragTo(page, ".chip:has-text('Bo')", ".file:has-text('lease.rs')");
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${out}/${name}_interaction.png` });
  await to(page, "#lease .fig");
  await page.locator("#lease input[type=range]").fill("1.01");
  await page.waitForTimeout(900);
  await page.screenshot({ path: `${out}/${name}_section2.png` });
}

export async function demo(page) {
  await page.waitForTimeout(2500);
  await to(page, "#lease .fig");
  for (const v of ["0.5", "0.8", "0.98", "1.1", "1.3"]) {
    await page.locator("#lease input[type=range]").fill(v);
    await page.waitForTimeout(450);
  }
  await to(page, "#footprints .fig");
  await dragTo(page, ".chip:has-text('Bo')", ".file:has-text('queue.rs')");
  await page.waitForTimeout(700);
  await dragTo(page, ".chip:has-text('Bo')", ".file:has-text('lease.rs')");
  await page.waitForTimeout(1300);
  await to(page, "#review .fig");
  await page.waitForTimeout(1500);
  await page.locator("#review input[type=range]").first().fill("3");
  await page.waitForTimeout(2500);
  await page.locator("#review input[type=range]").first().fill("0");
  await page.waitForTimeout(2000);
  await to(page, "#evidence .fig");
  await page.locator(".close-btn").click();
  await page.waitForTimeout(700);
  for (const b of await page.locator(".att").all()) {
    await b.click();
    await page.waitForTimeout(300);
  }
  await page.locator(".close-btn").click();
  await page.waitForTimeout(1000);
  await to(page, ".finale .fig");
  await page.waitForTimeout(2500);
  await to(page, ".know");
  await page.waitForTimeout(1200);
}
