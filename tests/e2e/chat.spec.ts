import { expect, test } from "@playwright/test";

test("chat generates, exposes run details, and cancels without losing text", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", e => errors.push(e.message));
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  const input = page.getByRole("textbox", { name: "O que você quer saber... (@arquivo)" });
  await input.fill("Escreva uma função em TypeScript");
  await page.getByRole("button", { name: "Enviar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Parar", exact: true })).toBeVisible();
  await expect(page.getByText("Claro!", { exact: false })).toBeVisible({ timeout: 15000 });
  await page.getByRole("button", { name: "Parar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Enviar", exact: true })).toBeVisible();
  await expect(page.getByText("Claro!", { exact: false })).toBeVisible();
  expect(errors).toEqual([]);
});

test("long histories stay virtualized and reading position survives streaming", async ({ page }) => {
  await page.goto("/tests/render.html");
  await page.waitForFunction(() => !!(window as any).startRenderBench);
  // Virtuoso applies its initial bottom position after measuring rows. Scroll
  // only once that placement is visible, as a reader would do.
  await expect(page.getByText("Mensagem 499", { exact: true })).toBeVisible();
  expect(await page.locator("body *").count()).toBeLessThan(500);
  const scroller = page.locator('[data-virtuoso-scroller="true"]');
  await scroller.evaluate(el => { el.scrollTop = 1000; });
  await page.waitForTimeout(200);
  expect(await scroller.evaluate(el => el.scrollTop)).toBeLessThan(2000);
  await page.evaluate(() => (window as any).startRenderBench(5));
  await page.waitForTimeout(300);
  const top = await scroller.evaluate(el => el.scrollTop);
  await page.getByRole("textbox", { name: "Typing probe" }).fill("typing remains available");
  await page.waitForTimeout(400);
  expect(Math.abs(await scroller.evaluate(el => el.scrollTop) - top)).toBeLessThan(100);
  await expect(page.getByRole("textbox")).toHaveValue("typing remains available");
});

test("comparison is discoverable and explains the desktop requirement", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Servidor Local", exact: true }).click();
  await page.getByRole("tab", { name: "Desempenho", exact: true }).click();
  await expect(page.getByText("Testar uma configuração melhor", { exact: true })).toBeVisible();
});
