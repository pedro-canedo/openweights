import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// A paleta de comandos (Ctrl+K), os atalhos e a folha de atalhos (?).

const nav = (page: Page) => page.getByRole("navigation").first();
const paleta = (page: Page) => page.getByRole("dialog", { name: "Comandos" });

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(nav(page).getByRole("button", { name: "Chat", exact: true })).toBeVisible();
});

test("Ctrl+K abre a paleta, a busca acha a tela e Enter vai até ela", async ({ page }) => {
  await page.keyboard.press("Control+k");
  const busca = paleta(page).getByRole("combobox");
  await expect(busca).toBeFocused();
  await busca.fill("configuracoes");
  await expect(paleta(page).getByRole("option").first()).toHaveText(/Ir para Configurações/);
  await busca.press("Enter");
  await expect(paleta(page)).toBeHidden();
  await expect(nav(page).getByRole("button", { name: "Configurações", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("as setas andam pela lista e Esc fecha", async ({ page }) => {
  await page.keyboard.press("Control+k");
  const busca = paleta(page).getByRole("combobox");
  await busca.fill("servidor");
  const opcoes = paleta(page).getByRole("option");
  await expect(opcoes.first()).toHaveAttribute("aria-selected", "true");
  await busca.press("ArrowDown");
  await expect(opcoes.nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(busca).toHaveAttribute("aria-activedescendant", (await opcoes.nth(1).getAttribute("id"))!);
  await busca.press("ArrowUp");
  await expect(opcoes.first()).toHaveAttribute("aria-selected", "true");
  await busca.press("Escape");
  await expect(paleta(page)).toBeHidden();
});

test("uma aba do Servidor Local abre direto pela paleta", async ({ page }) => {
  await page.keyboard.press("Control+k");
  await paleta(page).getByRole("combobox").fill("desempenho");
  await paleta(page).getByRole("option", { name: /Servidor Local › Desempenho/ }).click();
  await expect(page.getByRole("tab", { name: "Desempenho" })).toHaveAttribute("aria-selected", "true");
});

test("Ctrl+número, Ctrl+vírgula e Ctrl+N levam às telas", async ({ page }) => {
  await page.keyboard.press("Control+2");
  await expect(nav(page).getByRole("button", { name: "Descobrir", exact: true })).toHaveAttribute("aria-current", "page");
  await page.keyboard.press("Control+Comma");
  await expect(nav(page).getByRole("button", { name: "Configurações", exact: true })).toHaveAttribute("aria-current", "page");
  await page.keyboard.press("Control+n");
  await expect(nav(page).getByRole("button", { name: "Chat", exact: true })).toHaveAttribute("aria-current", "page");
});

test("? mostra os atalhos, mas não enquanto se digita", async ({ page }) => {
  await page.getByRole("textbox").first().click();
  await page.keyboard.type("quanto é 2+2?");
  await expect(page.getByRole("dialog", { name: "Atalhos de teclado" })).toHaveCount(0);
  await page.locator("body").click({ position: { x: 700, y: 120 } });
  await page.keyboard.press("?");
  const folha = page.getByRole("dialog", { name: "Atalhos de teclado" });
  await expect(folha).toBeVisible();
  await expect(folha).toContainText("Ctrl+Shift+K");
});

test("num terminal, Ctrl+K fica com o shell e Ctrl+Shift+K abre a paleta", async ({ page }) => {
  await nav(page).getByRole("button", { name: "OwCLI", exact: true }).click();
  await page.getByRole("button", { name: "Novo terminal" }).first().click();
  const tela = page.locator(".xterm-rows");
  await expect(tela).toContainText("voce@navegador");
  await page.keyboard.press("Control+k");
  await expect(paleta(page)).toHaveCount(0);
  await page.keyboard.press("Control+Shift+k");
  await expect(paleta(page)).toBeVisible();
});

test("a paleta abre o agente OwCLI de qualquer tela", async ({ page }) => {
  await page.keyboard.press("Control+k");
  await paleta(page).getByRole("combobox").fill("agente");
  await paleta(page).getByRole("option", { name: /Abrir o agente OwCLI/ }).click();
  await expect(nav(page).getByRole("button", { name: "OwCLI", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.getByRole("dialog", { name: "Abrir o agente OwCLI" })).toBeVisible();
});

test("a paleta não tem violação séria de acessibilidade", async ({ page }) => {
  await page.keyboard.press("Control+k");
  await expect(paleta(page).getByRole("option").first()).toBeVisible();
  const r = await new AxeBuilder({ page })
    .include('[role="dialog"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const graves = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(graves.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
});

test("a ajuda está na barra lateral e na paleta", async ({ page }) => {
  await expect(nav(page).getByRole("button", { name: "Ajuda" })).toBeVisible();
  await page.keyboard.press("Control+k");
  await paleta(page).getByRole("combobox").fill("ajuda");
  await expect(paleta(page).getByRole("option").first()).toHaveText(/Abrir a ajuda/);
});
