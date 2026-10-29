// Capture stills and demo videos of the landing concept.
// Usage: node scripts/capture.mjs <url> <outDir>
// Needs PW_CORE (path to a playwright-core install) and PW_CHROME (browser binary).
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, renameSync, rmSync } from "node:fs";
import { join } from "node:path";
import { interact, demo } from "./interact.mjs";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PW_CORE);
const [url, out] = process.argv.slice(2);
mkdirSync(out, { recursive: true });

const devices = {
  iphone: { viewport: { width: 393, height: 852 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true },
  macbook: { viewport: { width: 1512, height: 982 }, deviceScaleFactor: 2 },
};

const browser = await chromium.launch({ executablePath: process.env.PW_CHROME });
for (const [name, dev] of Object.entries(devices)) {
  const ctx = await browser.newContext(dev);
  const page = await ctx.newPage();
  await page.goto(url, { waitUntil: "networkidle" });
  await page.waitForTimeout(1800);
  await page.screenshot({ path: join(out, `${name}_hero.png`) });
  await interact(page, name, out);
  await ctx.close();

  const vdir = join(out, `v-${name}`);
  rmSync(vdir, { recursive: true, force: true });
  const vctx = await browser.newContext({ ...dev, deviceScaleFactor: 1, recordVideo: { dir: vdir, size: dev.viewport } });
  const vp = await vctx.newPage();
  await vp.goto(url, { waitUntil: "networkidle" });
  await demo(vp, name);
  await vctx.close();
  const file = readdirSync(vdir).find((f) => f.endsWith(".webm"));
  const webm = join(out, `demo_${name}.webm`);
  renameSync(join(vdir, file), webm);
  rmSync(vdir, { recursive: true, force: true });
  try {
    execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", webm, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart", join(out, `demo_${name}.mp4`)]);
    rmSync(webm);
  } catch {
    // keep the webm
  }
}
await browser.close();
