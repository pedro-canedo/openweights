import { test, expect } from "@playwright/test";

test("one-click optimization presents ready rates and supports restoring", async ({ page }) => {
  await page.goto("/tests/optimization.html");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(page.getByText("Configuração pronta", { exact: true })).toBeVisible();
  await expect(page.getByText("Leitura: 120.0 tokens/s · Geração: 28.0 tokens/s")).toBeVisible();
  await expect(page.getByRole("table")).not.toBeVisible();
  await page.getByRole("button", { name: "Usar configuração" }).click();
  await expect(page.getByRole("button", { name: /Restaurar/ })).toBeVisible();
  await page.getByRole("button", { name: /Restaurar/ }).click();
  await expect(page.getByRole("button", { name: "Usar configuração" })).toBeVisible();
});

test("cancellation releases the interface without presenting an incomplete result", async ({ page }) => {
  await page.goto("/tests/optimization.html?scenario=cancel");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await page.getByRole("button", { name: /Cancelar/ }).click();
  await expect(page.getByRole("button", { name: "Otimizar para meu computador" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Usar configuração" })).toHaveCount(0);
});

test("package failures are explained without hiding successful measurements", async ({ page }) => {
  await page.goto("/tests/optimization.html?scenario=unavailable");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(page.getByText(/Algumas opções não puderam ser verificadas/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Usar configuração" })).toBeEnabled();
});
