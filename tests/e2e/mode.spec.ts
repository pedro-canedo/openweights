import { test, expect, type Page } from "@playwright/test";

// Modo Simples/Avançado: o que a barra lateral mostra em cada um, a troca em
// Configurações e a tela avançada aberta por um botão, que aparece na barra
// enquanto a pessoa está nela.

const nav = (page: Page) => page.getByRole("navigation").first();

async function abrirNoModo(page: Page, modo: "simples" | "avancado") {
  // Só na primeira carga: um reload tem de ver o que a tela gravou.
  await page.addInitScript((m) => {
    if (!localStorage.getItem("ow.mode")) localStorage.setItem("ow.mode", m);
  }, modo);
  await page.goto("/");
}

test("no Simples, a barra mostra os grupos Começar e Agentes, sem o Avançado", async ({ page }) => {
  await abrirNoModo(page, "simples");
  await expect(nav(page).getByRole("group", { name: "Começar" })).toBeVisible();
  await expect(nav(page).getByRole("group", { name: "Agentes" })).toBeVisible();
  await expect(nav(page).getByRole("group", { name: "Avançado" })).toHaveCount(0);
  for (const tela of ["Servidor Local", "Fontes", "Treinar"]) {
    await expect(nav(page).getByRole("button", { name: tela, exact: true })).toHaveCount(0);
  }
  await expect(nav(page).getByRole("button", { name: "Configurações", exact: true })).toBeVisible();
});

test("trocar para o Avançado em Configurações traz as telas de volta", async ({ page }) => {
  await abrirNoModo(page, "simples");
  await nav(page).getByRole("button", { name: "Configurações", exact: true }).click();
  const modo = page.getByLabel("Modo", { exact: true });
  await expect(modo).toHaveValue("simples");
  await modo.selectOption("avancado");
  const avancado = nav(page).getByRole("group", { name: "Avançado" });
  for (const tela of ["Servidor Local", "Fontes", "Treinar"]) {
    await expect(avancado.getByRole("button", { name: tela, exact: true })).toBeVisible();
  }
  // A escolha sobrevive a reabrir o app.
  await page.reload();
  await expect(nav(page).getByRole("group", { name: "Avançado" })).toBeVisible();
});

test("no Simples, a tela avançada aberta por um botão aparece na barra", async ({ page }) => {
  await page.addInitScript(() => {
    (window as { __owcliSemModelos?: boolean }).__owcliSemModelos = true;
  });
  await abrirNoModo(page, "simples");
  await nav(page).getByRole("button", { name: "OwCLI", exact: true }).click();
  await page.getByRole("button", { name: "Abrir o agente OwCLI" }).click();
  await page.getByRole("button", { name: "Configurar em Fontes" }).first().click();
  const fontes = nav(page).getByRole("button", { name: "Fontes", exact: true });
  await expect(fontes).toHaveAttribute("aria-current", "page");
  // Saindo dela, some de novo.
  await nav(page).getByRole("button", { name: "Chat", exact: true }).click();
  await expect(fontes).toHaveCount(0);
});

test("no Simples, o chat não mostra o seletor de esforço", async ({ page }) => {
  await abrirNoModo(page, "simples");
  await nav(page).getByRole("button", { name: "Chat", exact: true }).click();
  const seletor = page.locator("button[aria-haspopup]").filter({ hasText: /Qwen|Llama|gguf/i }).first();
  await expect(seletor).toBeVisible();
  await seletor.click();
  await expect(page.getByRole("button", { name: /Esforço/ })).toHaveCount(0);
});

test("no Avançado, o mesmo seletor mostra o esforço", async ({ page }) => {
  await abrirNoModo(page, "avancado");
  await nav(page).getByRole("button", { name: "Chat", exact: true }).click();
  const seletor = page.locator("button[aria-haspopup]").filter({ hasText: /Qwen|Llama|gguf/i }).first();
  await seletor.click();
  await expect(page.getByRole("button", { name: /Esforço/ })).toHaveCount(1);
});
