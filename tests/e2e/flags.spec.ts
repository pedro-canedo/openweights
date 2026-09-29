import { test, expect } from "@playwright/test";

// O catálogo de flags do motor: categorias em chips e busca que também olha
// o texto de ajuda.

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Chat", exact: true })).toBeVisible();
  await page.keyboard.press("Control+k");
  await page.getByRole("combobox").fill("desempenho");
  await page.keyboard.press("Enter");
  // O catálogo mora em "Ajustes manuais e presets", recolhido por padrão.
  await page.locator("summary", { hasText: "Ajustes manuais e presets" }).click();
});

test("um chip de categoria lista as flags dela", async ({ page }) => {
  const chips = page.getByRole("group", { name: "Categorias de flags" }).first();
  await expect(chips).toBeVisible();
  const contexto = chips.getByRole("button", { name: /Contexto/ });
  await contexto.click();
  await expect(contexto).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("cache-reuse").first()).toBeVisible();
  await expect(page.getByText("swa-full")).toHaveCount(0);
  // Clicar de novo solta o filtro.
  await contexto.click();
  await expect(contexto).toHaveAttribute("aria-pressed", "false");
});

test("a busca acha a flag pelo que ela faz, no texto de ajuda", async ({ page }) => {
  await page.getByRole("textbox", { name: "Buscar flag (ex.: mtp, rope, cache-reuse, jinja…)" }).first().fill("full-size cache");
  await expect(page.getByText("swa-full").first()).toBeVisible();
});

test("uma flag alterada diz que foi modificada e volta ao padrão com um clique", async ({ page }) => {
  await page
    .getByRole("textbox", { name: "Buscar flag (ex.: mtp, rope, cache-reuse, jinja…)" })
    .first()
    .fill("swa-full");
  const linha = page.locator("div.rounded-lg").filter({ hasText: "--swa-full" }).first();
  await expect(linha).toBeVisible();
  // De fábrica não há ponto de "modificada".
  await expect(linha.getByText("Modificada", { exact: true })).toHaveCount(0);

  // Escolher "ligado" (o padrão do llama.cpp é desligado) acende o ponto.
  await linha.getByRole("button", { name: "ligado", exact: true }).first().click();
  await expect(linha.getByText("Modificada", { exact: true })).toBeVisible();

  await linha.getByRole("button", { name: /ao padrão/ }).click();
  await expect(linha.getByText("Modificada", { exact: true })).toHaveCount(0);
});
