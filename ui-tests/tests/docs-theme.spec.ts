import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

const root = resolve(__dirname, "../..");
const css = readFileSync(resolve(root, "book/theme/maidan.css"), "utf8");
const head = readFileSync(resolve(root, "book/theme/head.hbs"), "utf8");
const favicon = readFileSync(resolve(root, "book/theme/favicon.svg"), "utf8");
const built = resolve(root, "book/build/html/index.html");
const themes = ["light", "rust", "ayu", "coal", "navy"] as const;

function pageFor(theme: string): string {
  return `<!doctype html><html lang="en" class="${theme}"><head><title>Maidan docs</title><style>${css}</style></head><body>
    <div id="menu-bar"><a href="#book">Maidan</a><button class="icon-button" type="button" aria-label="Open the menu">Menu</button></div>
    <div class="theme-popup"><button class="theme" type="button">Light</button></div>
    <nav class="mobile-nav-chapters" aria-label="Chapters"><a href="#chapter">Chapter</a></nav>
    <main class="content">
      <h1>Maidan</h1>
      <p class="menu-title">Guide</p>
      <p>Read the <a href="#guide">guide</a>.</p>
      <div class="table-wrapper"><table><tr><th>Name</th><td>A long value that should stay inside the screen</td></tr></table></div>
      <pre><code>maidan</code></pre>
      <div class="mermaid"><svg viewBox="0 0 800 40" role="img"><title>diagram</title><text x="0" y="24">diagram</text></svg></div>
      <button class="clip-button" type="button">Copy</button>
    </main>
  </body></html>`;
}

async function fits(page: Page): Promise<boolean> {
  return page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);
}

async function shortTargets(page: Page): Promise<string[]> {
  return page.locator("#menu-bar .icon-button, #menu-bar a, .theme-popup .theme, .mobile-nav-chapters, .mobile-nav-chapters a, .clip-button").evaluateAll((els) =>
    els
      .map((el) => {
        const r = el.getBoundingClientRect();
        return { t: (el.textContent ?? "").trim().slice(0, 24), w: Math.round(r.width), h: Math.round(r.height) };
      })
      .filter(({ w, h }) => w > 0 && h > 0 && (w < 44 || h < 44))
      .map(({ t, w, h }) => `${t} ${w}x${h}`),
  );
}

test("the docs theme files use steel and signal, not pine", () => {
  const files = `${head}\n${css}\n${favicon}`.toLowerCase();
  for (const banned of ["#14532d", "#4ade80", "#b45309"]) expect(files).not.toContain(banned);
  expect(head).toContain('name="theme-color" content="#f2eee6" media="(prefers-color-scheme: light)"');
  expect(head).toContain('name="theme-color" content="#121110" media="(prefers-color-scheme: dark)"');
  expect(head).toContain("maidan-social.png");
  expect(head).toContain("summary_large_image");
  expect(favicon).toContain("#151411");
  expect(favicon).toContain("#f2eee6");
  expect(css).not.toContain("hue-rotate(9");
  expect(css).not.toContain("Palatino");
});

for (const theme of themes) {
  for (const viewport of [
    { width: 390, height: 844 },
    { width: 1280, height: 800 },
  ]) {
    test(`${theme} at ${viewport.width} keeps contrast, a 44px target, and no sideways scroll`, async ({ page }) => {
      await page.setViewportSize(viewport);
      await page.setContent(pageFor(theme));
      const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
      expect(scan.violations.map((v) => v.id)).toEqual([]);
      expect(await fits(page)).toBe(true);
      expect(await shortTargets(page)).toEqual([]);
      const copy = page.locator(".clip-button");
      await copy.hover();
      expect(await copy.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe("none");
      await page.keyboard.press("Tab");
      const outline = await page.evaluate(() => getComputedStyle(document.activeElement as Element).outlineStyle);
      expect(outline).not.toBe("none");
      await page.emulateMedia({ reducedMotion: "reduce" });
      const duration = await copy.evaluate((el) => {
        const raw = getComputedStyle(el).transitionDuration;
        const n = Number.parseFloat(raw);
        return raw.endsWith("ms") ? n : n * 1000;
      });
      expect(duration).toBeLessThanOrEqual(1);
    });
  }
}

test.describe("built book", () => {
  test.skip(!existsSync(built), "mdbook output is produced by the docs job, not by the ui-tests job");

  test("the published home page names both theme colors", () => {
    const html = readFileSync(built, "utf8");
    expect(html).toContain('content="#f2eee6"');
    expect(html).toContain('content="#121110"');
    expect(html.toLowerCase()).not.toContain("#14532d");
  });
});
