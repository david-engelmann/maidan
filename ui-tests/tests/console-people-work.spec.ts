import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

test("an agent's detail shows the work it holds, linked to the thread", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, `#/people/m/${fx.requester_id}`);
  const work = page.getByRole("region", { name: "Work" });
  await expect(work.getByRole("heading", { name: "Now" })).toBeVisible();
  const link = work.getByRole("link").first();
  await expect(link).toBeVisible();
  await expect(link).toHaveAttribute("href", /#\/threads\/t\//);
});

test("a member with no work says so", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, `#/people/m/${fx.rae_member_id}`);
  const work = page.getByRole("region", { name: "Work" });
  await expect(work.getByText("Nothing claimed right now.")).toBeVisible();
});
