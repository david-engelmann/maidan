import { expect, type Locator } from "@playwright/test";

type Box = { x: number; y: number; width: number; height: number };

/**
 * A control can be reported visible before layout gives it a box (a phone-width
 * reflow, or a row redrawing as its packet arrives). Read the box only once
 * width and height are both non-zero.
 */
export async function laidOutBox(locator: Locator): Promise<Box> {
  const got: { box: Box | null } = { box: null };
  await expect.poll(async () => {
    got.box = await locator.boundingBox();
    return Boolean(got.box && got.box.width > 0 && got.box.height > 0);
  }).toBe(true);
  const box = got.box;
  if (!box) throw new Error("layout reported a box, then dropped it");
  return box;
}
