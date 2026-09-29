import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// O owcli no terminal do sistema: opt-in, em Configurações, no modo Avançado.

async function abrirConfiguracoes(page: Page, modo = "avancado") {
  await page.addInitScript((m) => {
    if (!localStorage.getItem("ow.mode")) localStorage.setItem("ow.mode", m);
  }, modo);
  await page.goto("/");
  await page
    .getByRole("navigation")
    .first()
    .getByRole("button", { name: "Configurações", exact: true })
    .click();
}

const ativar = (page: Page) => page.getByRole("button", { name: "Ativar o comando owcli" });
const flag = (page: Page, nome: string) =>
  page.addInitScript((n) => {
    (globalThis as Record<string, unknown>)[n] = true;
  }, nome);

async function semViolacoes(page: Page) {
  const r = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const graves = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(
    graves.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`),
  ).toEqual([]);
}

test("ativa e desativa o comando, mostrando onde ele está só depois de ativar", async ({ page }) => {
  await abrirConfiguracoes(page);
  await expect(page.getByText("Sem o clique, nada muda no seu terminal.")).toBeVisible();
  // Antes de ativar, o caminho não é "o comando".
  await expect(page.getByRole("button", { name: "/home/voce/.local/bin/owcli" })).toHaveCount(0);
  await expect(page.getByText("o app precisa estar aberto quando você usar o comando", { exact: false })).toBeVisible();

  await ativar(page).click();
  await expect(page.getByRole("status").filter({ hasText: "Ativo: o owcli abre a versão instalada." })).toBeVisible();
  await expect(page.getByRole("button", { name: "/home/voce/.local/bin/owcli" })).toBeVisible();

  await page.getByRole("button", { name: "Desativar", exact: true }).click();
  await expect(ativar(page)).toBeVisible();
  await expect(page.getByText("Ativo: o owcli abre a versão instalada.")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "/home/voce/.local/bin/owcli" })).toHaveCount(0);
});

test("sem o agente instalado, o cartão manda instalar primeiro", async ({ page }) => {
  await flag(page, "__owcliSemTerminal");
  await abrirConfiguracoes(page);
  await expect(page.getByText("Instale o agente OwCLI primeiro")).toBeVisible();
  await expect(ativar(page)).toHaveCount(0);
  await page.getByRole("button", { name: "Abrir a tela OwCLI" }).click();
  await expect(page.getByRole("heading", { name: "Nenhuma sessão aberta" })).toBeVisible();
});

test("um arquivo alheio no lugar do link é dito, e tentar de novo diz o motivo sem tocar nele", async ({ page }) => {
  await flag(page, "__owcliTerminalConflito");
  await abrirConfiguracoes(page);
  const aviso = /Já existe um arquivo em \/home\/voce\/\.local\/bin\/owcli que não é do OpenWeights/;
  await expect(page.getByRole("status").filter({ hasText: aviso })).toBeVisible();
  await semViolacoes(page);
  // O botão segue à mão: a pessoa resolve o arquivo e tenta de novo.
  await expect(ativar(page)).toBeEnabled();
  await ativar(page).click();
  await expect(page.getByRole("alert")).toContainText("Renomeie-o ou apague-o");
  await expect(ativar(page)).toBeVisible();
});

test("com a pasta do link fora do PATH, a linha para o perfil do shell aparece", async ({ page }) => {
  await flag(page, "__owcliTerminalForaDoPath");
  await abrirConfiguracoes(page);
  await ativar(page).click();
  await expect(page.getByText(/A pasta do link não está no PATH lido por este app/)).toBeVisible();
  await expect(page.getByText(/fish_add_path ~\/\.local\/bin/)).toBeVisible();
  await expect(page.getByRole("button", { name: 'export PATH="$HOME/.local/bin:$PATH"' })).toBeVisible();
  await semViolacoes(page);
});

test("no Windows o comando chega pelo Path e pede um terminal novo", async ({ page }) => {
  await flag(page, "__owcliTerminalWindows");
  await abrirConfiguracoes(page);
  await ativar(page).click();
  await expect(page.getByText("A pasta do comando, no Path do seu usuário:")).toBeVisible();
  await expect(page.getByText("Abra um terminal novo")).toBeVisible();
  await expect(page.getByText(/fish_add_path/)).toHaveCount(0);
});

test("um link que aponta para versão antiga é dito, e a correção é desativar e ativar", async ({ page }) => {
  await flag(page, "__owcliTerminalDesatualizado");
  await abrirConfiguracoes(page);
  await ativar(page).click();
  await expect(page.getByText(/aponta para uma versão antiga: o app corrige isso ao abrir/)).toBeVisible();
});

test("no modo Simples o cartão não aparece, a menos que o comando já esteja ligado", async ({ page }) => {
  await abrirConfiguracoes(page, "simples");
  await expect(page.getByRole("heading", { name: "Preferências" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "OwCLI no terminal do sistema" })).toHaveCount(0);

  // Ligado no Avançado e passado ao Simples, o cartão fica para dar como desligar.
  await page.getByLabel("Modo", { exact: true }).selectOption("avancado");
  await ativar(page).click();
  await expect(page.getByText("Ativo: o owcli abre a versão instalada.")).toBeVisible();
  await page.getByLabel("Modo", { exact: true }).selectOption("simples");
  await expect(page.getByRole("button", { name: "Desativar", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Desativar", exact: true }).click();
  await expect(page.getByRole("heading", { name: "OwCLI no terminal do sistema" })).toHaveCount(0);
});

test("o cartão não tem violação séria de acessibilidade", async ({ page }) => {
  await abrirConfiguracoes(page);
  await ativar(page).click();
  await expect(page.getByText("Ativo: o owcli abre a versão instalada.")).toBeVisible();
  await semViolacoes(page);
});
