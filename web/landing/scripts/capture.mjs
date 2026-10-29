// Usage: node scripts/capture.mjs <NN> <url>. Steps live in scripts/steps.mjs.
import { createRequire } from "node:module";
import { mkdirSync, readdirSync, renameSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import os from "node:os";
const require = createRequire(os.homedir() + "/bg/landing-captures/tool-b/");
const { chromium, webkit } = require("playwright");
const steps = await import(new URL("./steps.mjs", import.meta.url));
const [nn, url] = process.argv.slice(2);
const out = `${os.homedir()}/bg/landing-captures/${nn}`;
mkdirSync(out, { recursive: true });
const exe = `${os.homedir()}/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing`;
const browser = await chromium.launch({ executablePath: exe, args: ["--enable-gpu", "--use-angle=metal", "--ignore-gpu-blocklist"] });
const devices = {
  iphone: { viewport: { width: 393, height: 852 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true, userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1" },
  macbook: { viewport: { width: 1512, height: 982 }, deviceScaleFactor: 2 },
};
const only = process.env.ONLY;
for (const [name, opts] of Object.entries(devices)) {
  if (only && only !== name) continue;
  // Stills
  const ctx = await browser.newContext({ ...opts });
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  await page.goto(url, { waitUntil: "networkidle" });
  await page.waitForTimeout(1800);
  await page.screenshot({ path: `${out}/${name}_hero.png` });
  await steps.interact(page, name);
  await page.screenshot({ path: `${out}/${name}_interaction.png` });
  if (name === "iphone") { await steps.section2(page, name); await page.screenshot({ path: `${out}/${name}_section2.png` }); }
  if (errors.length) console.log(name, "errors:", errors.slice(0, 5));
  await ctx.close();
  if (process.env.NOVIDEO) continue;
  // Video
  const vdir = `${out}/.v-${name}`;
  rmSync(vdir, { recursive: true, force: true });
  const vctx = await browser.newContext({ ...opts, deviceScaleFactor: 1, recordVideo: { dir: vdir, size: opts.viewport } });
  const vp = await vctx.newPage();
  await vp.goto(url, { waitUntil: "networkidle" });
  await vp.waitForTimeout(2200);
  await steps.interact(vp, name);
  await vp.waitForTimeout(1200);
  await steps.tour(vp, name);
  await vctx.close();
  const file = readdirSync(vdir).find((f) => f.endsWith(".webm"));
  const webm = `${out}/demo_${name}.webm`;
  renameSync(`${vdir}/${file}`, webm);
  rmSync(vdir, { recursive: true, force: true });
  try {
    execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", webm, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "veryfast", "-crf", "24", "-movflags", "+faststart", `${out}/demo_${name}.mp4`]);
    rmSync(webm);
  } catch (e) { console.log("ffmpeg failed", String(e).slice(0, 200)); }
}
await browser.close();
console.log("captured", out);
