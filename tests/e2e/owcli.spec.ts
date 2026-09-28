import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// A tela do OwCLI no navegador, com o shell simulado de lib/terminals.ts.
// O que se prova aqui é o contrato da interface: abrir, digitar, o terminal
// sobreviver à troca de tela, fechar. O pseudoterminal de verdade tem os
// testes dele no crate lr_pty.

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "OwCLI", exact: true }).click();
});

test("abre um terminal, digita e o terminal sobrevive à troca de tela", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Nenhum terminal aberto" })).toBeVisible();
  await page.getByRole("button", { name: "Novo terminal" }).first().click();

  const sessoes = page.getByRole("list", { name: "Terminais abertos" });
  await expect(sessoes.locator('[aria-current="true"]')).toContainText("bash");
  const tela = page.locator(".xterm-rows");
  await expect(tela).toContainText("voce@navegador");

  await page.keyboard.type("echo ola mundo");
  await page.keyboard.press("Enter");
  await expect(tela).toContainText("ola mundo");

  // Trocar de tela desmonta a tela, não o terminal.
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  await page.getByRole("button", { name: "OwCLI", exact: true }).click();
  await expect(page.locator(".xterm-rows")).toContainText("ola mundo");
});

test("duas sessões, setas trocam e fechar volta ao estado vazio", async ({ page }) => {
  const novo = page.getByRole("button", { name: "Novo terminal" }).first();
  const itens = page.getByRole("list", { name: "Terminais abertos" }).getByRole("listitem");
  await novo.click();
  await expect(itens).toHaveCount(1);
  await page.keyboard.type("echo primeira");
  await page.keyboard.press("Enter");
  await novo.click();
  await expect(itens).toHaveCount(2);
  const botoes = page.locator("[data-sessao]");
  await expect(botoes.nth(1)).toHaveAttribute("aria-current", "true");

  await botoes.nth(1).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(botoes.first()).toHaveAttribute("aria-current", "true");
  await expect(botoes.first()).toBeFocused();
  await expect(page.locator(".xterm-rows")).toContainText("primeira");

  const fechar = page.getByRole("button", { name: /^Fechar bash/ });
  while ((await fechar.count()) > 0) {
    await fechar.first().click();
  }
  await expect(page.getByRole("heading", { name: "Nenhum terminal aberto" })).toBeVisible();
});

test("a tela do OwCLI não tem violação séria de acessibilidade", async ({ page }) => {
  await page.getByRole("button", { name: "Novo terminal" }).first().click();
  await expect(page.locator(".xterm-rows")).toContainText("voce@navegador");
  const r = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const graves = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(graves.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
});
