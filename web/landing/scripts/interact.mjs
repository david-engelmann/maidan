// Concept 07: scripted interactions for stills and demo videos.
async function center(page, sel) {
  const b = await page.locator(sel).first().boundingBox();
  return b ? [b.x + b.width / 2, b.y + b.height / 2] : [0, 0];
}

async function dragBy(page, sel, dx, dy) {
  const [x, y] = await center(page, sel);
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 20; i++) {
    await page.mouse.move(x + (dx * i) / 20, y + (dy * i) / 20);
    await page.waitForTimeout(16);
  }
  await page.mouse.up();
}

async function hold(page, x, y) {
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.waitForTimeout(650);
  await page.mouse.up();
}

async function toScene(page, scene, smooth) {
  await page.locator(`[data-scene=${scene}]`).evaluate((el, b) => el.scrollIntoView({ block: "center", behavior: b }), smooth ? "smooth" : "instant");
}

export async function interact(page, name, out) {
  const vw = page.viewportSize().width;
  await dragBy(page, ".agent >> nth=1", vw > 900 ? 60 : 30, 40);
  await page.waitForTimeout(300);
  await page.locator(".agent").first().click();
  await page.waitForTimeout(700);
  await page.screenshot({ path: `${out}/${name}_interaction.png` });
  await page.locator(".inspect .x").click();
  await toScene(page, "lease", false);
  await page.waitForTimeout(2200);
  await page.screenshot({ path: `${out}/${name}_section2.png` });
}

export async function demo(page) {
  const { width, height } = page.viewportSize();
  await page.waitForTimeout(2500);
  await dragBy(page, ".agent >> nth=2", width > 900 ? -80 : -40, 50);
  await page.waitForTimeout(500);
  await page.locator(".agent").first().click();
  await page.waitForTimeout(1800);
  await page.locator(".inspect .x").click();
  await hold(page, width > 900 ? width * 0.82 : width * 0.8, height * 0.78);
  await page.waitForTimeout(1200);
  for (const s of ["lease", "gate", "evidence", "swarm"]) {
    await toScene(page, s, true);
    await page.waitForTimeout(3000);
  }
}
