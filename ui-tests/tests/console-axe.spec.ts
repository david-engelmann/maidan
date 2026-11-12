import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

// The accessibility gate: every console place, in both themes, at a desktop
// and a phone width, scanned by axe once its data has loaded. A serious or
// critical violation fails the run and names its nodes. Before this, axe ran
// in two specs and on none of the eight places, so "accessible" was a hope.
// Reduced motion keeps the scan off a fade, where text at part opacity reads
// as a contrast failure no reader ever sees.
const PLACES = ["board", "needs", "threads", "people", "activity", "tools", "catalog", "artifacts"] as const;
const THEMES = ["dark", "light"] as const;
const WIDTHS = [
  { tag: "desktop", width: 1440, height: 900, mobile: false },
  { tag: "phone", width: 390, height: 844, mobile: true },
] as const;
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
const GATED = new Set(["serious", "critical"]);

/** Wait until the place has drawn and stopped loading. */
async function settled(page: Page): Promise<void> {
  const main = page.locator("#main");
  await expect(main).not.toBeEmpty();
  await expect(main.locator('[aria-busy="true"], .ghost-line')).toHaveCount(0);
}

for (const vp of WIDTHS) {
  for (const theme of THEMES) {
    test.describe(`${vp.tag}, ${theme}`, () => {
      test.use({ viewport: { width: vp.width, height: vp.height }, isMobile: vp.mobile, hasTouch: vp.mobile, reducedMotion: "reduce" });

      for (const place of PLACES) {
        test(`${place} has no serious or critical axe violation`, async ({ page }) => {
          await page.addInitScript((t) => window.localStorage.setItem("maidan_console_theme", t), theme);
          await signInWithCookie(page, fx);
          await openOnChannel(page, fx.board_channel_id, `#/${place}`);
          await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
          await settled(page);
          const scan = await new AxeBuilder({ page }).withTags(TAGS).analyze();
          const found = scan.violations
            .filter((v) => GATED.has(v.impact ?? ""))
            .map((v) => `${v.impact} ${v.id}: ${v.help} at ${v.nodes.slice(0, 5).map((n) => n.target.join(" ")).join(", ")}`);
          expect(found, `${place}, ${theme}, ${vp.tag}`).toEqual([]);
        });
      }
    });
  }
}
