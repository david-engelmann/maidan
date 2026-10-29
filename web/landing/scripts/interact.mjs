// Concept 05: scripted play for stills and demo videos.
async function drag(page, from, to) {
  const a = await page.locator(from).first().boundingBox();
  const b = await page.locator(to).first().boundingBox();
  if (!a || !b) return false;
  const ax = a.x + a.width / 2, ay = a.y + a.height / 2, bx = b.x + b.width / 2, by = b.y + b.height / 2;
  await page.mouse.move(ax, ay);
  await page.mouse.down();
  for (let i = 1; i <= 16; i++) {
    await page.mouse.move(ax + ((bx - ax) * i) / 16, ay + ((by - ay) * i) / 16);
    await page.waitForTimeout(14);
  }
  await page.mouse.up();
  return true;
}

async function start(page) {
  await page.locator(".play-btn").click();
  await page.waitForTimeout(400);
  await page.locator(".go").first().click();
  await page.waitForTimeout(1500);
}

async function playFor(page, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await page.locator(".overlay .go").count()) {
      await page.locator(".overlay .go").first().click();
      await page.waitForTimeout(600);
      continue;
    }
    const done = page.locator(".agent .tile.done");
    if (await done.count()) {
      const t = done.first();
      const cls = (await t.getAttribute("class")) || "";
      if (cls.includes("no-proof")) await t.click();
      else await drag(page, ".agent .tile.done", cls.includes("needs-review") ? "[data-drop=review]" : "[data-drop=ship]");
      await page.waitForTimeout(350);
      continue;
    }
    const agents = await page.locator(".agent:not(.busy)").count();
    if (agents && (await page.locator(".lane .tile").count())) {
      const free = await page.locator(".agent:not(.busy)").first().getAttribute("data-drop");
      await drag(page, ".lane .tile", `[data-drop="${free}"]`);
      await page.waitForTimeout(300);
      continue;
    }
    await page.waitForTimeout(250);
  }
}

export async function interact(page, name, out) {
  await start(page);
  await drag(page, ".lane .tile", "[data-drop=agent-0]");
  await page.waitForTimeout(900);
  await drag(page, ".lane .tile", "[data-drop=agent-0]");
  await page.waitForTimeout(650);
  await page.screenshot({ path: `${out}/${name}_interaction.png` });
  await page.locator(".after").evaluate((el) => el.scrollIntoView({ block: "start", behavior: "instant" }));
  await page.waitForTimeout(1200);
  await page.screenshot({ path: `${out}/${name}_section2.png` });
}

export async function demo(page) {
  await page.waitForTimeout(5000);
  await start(page);
  await drag(page, ".lane .tile", "[data-drop=agent-0]");
  await page.waitForTimeout(700);
  await drag(page, ".lane .tile", "[data-drop=agent-0]");
  await page.waitForTimeout(900);
  await playFor(page, 11000);
  await page.locator(".after").evaluate((el) => el.scrollIntoView({ block: "start", behavior: "smooth" }));
  await page.waitForTimeout(2500);
}
