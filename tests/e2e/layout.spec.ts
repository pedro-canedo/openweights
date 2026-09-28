import { test, expect } from "@playwright/test";

// Largura ajustável da lista do Descobrir e o tamanho da interface: os dois
// ficam guardados entre visitas.

test("a lista do Descobrir alarga pelas setas e lembra a largura", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Descobrir", exact: true }).click();
  const divisor = page.getByRole("separator", { name: "Largura da lista de modelos" });
  const lista = page.getByRole("heading", { name: /Descobrir/ }).first();
  const antes = (await divisor.boundingBox())!.x;
  await divisor.focus();
  await page.keyboard.press("Shift+ArrowRight");
  await page.keyboard.press("Shift+ArrowRight");
  const depois = (await divisor.boundingBox())!.x;
  expect(depois).toBeGreaterThan(antes);
  await expect(lista).toBeVisible();

  await page.reload();
  await page.getByRole("button", { name: "Descobrir", exact: true }).click();
  await expect
    .poll(async () => Math.round((await page.getByRole("separator", { name: "Largura da lista de modelos" }).boundingBox())!.x))
    .toBe(Math.round(depois));
});

test("o tamanho da interface vale na hora e na próxima abertura", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Configurações", exact: true }).click();
  await page.getByLabel("Tamanho da interface").selectOption("1.25");
  await expect.poll(() => page.evaluate(() => document.documentElement.style.zoom)).toBe("1.25");
  await page.reload();
  await expect.poll(() => page.evaluate(() => document.documentElement.style.zoom)).toBe("1.25");
});
