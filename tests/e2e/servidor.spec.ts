import { expect, test, type Page } from "@playwright/test";

// Servidor Local: a otimização e os números que dividem a placa moram em
// Desempenho, e Meus Modelos leva até lá.

const nav = (page: Page) => page.getByRole("navigation").first();

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(nav(page).getByRole("button", { name: "Chat", exact: true })).toBeVisible();
});

test("Ajustar para esta máquina, em Meus Modelos, abre o painel em Servidor Local › Desempenho", async ({ page }) => {
  await nav(page).getByRole("button", { name: "Meus Modelos", exact: true }).click();
  await page.locator(".model-library-card").getByRole("button", { name: "Ajustar para esta máquina" }).click();

  await expect(nav(page).getByRole("button", { name: "Servidor Local", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.getByRole("tab", { name: "Desempenho" })).toHaveAttribute("aria-selected", "true");
  // O painel já vem aberto para o modelo que se clicou.
  await expect(page.getByText("Ajustar para esta máquina", { exact: true })).toBeVisible();
  // A comparação vem dentro do painel: não há um segundo cartão dela.
  await expect(page.getByRole("button", { name: "Otimizar para meu computador" })).toHaveCount(1);
});

test("modelos simultâneos e conversas ao mesmo tempo estão em Desempenho, não em Rede", async ({ page }) => {
  await nav(page).getByRole("button", { name: "Servidor Local", exact: true }).click();
  await page.getByRole("tab", { name: "Rede" }).click();
  await expect(page.getByText("Porta", { exact: true })).toBeVisible();
  await expect(page.getByText("Conversas ao mesmo tempo")).toHaveCount(0);

  await page.getByRole("tab", { name: "Desempenho" }).click();
  const card = page.getByRole("heading", { name: "Concorrência" });
  await expect(card).toBeVisible();
  await expect(page.getByText("o dele vale sobre este")).toBeVisible();
  await expect(page.getByText("Conversas ao mesmo tempo")).toBeVisible();
});
