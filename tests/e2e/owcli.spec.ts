import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// A tela do OwCLI no navegador, com o shell simulado de lib/terminals.ts.
// O que se prova aqui é o contrato da interface: abrir, digitar, o terminal
// sobreviver à troca de tela, fechar. O pseudoterminal de verdade tem os
// testes dele no crate lr_pty.

/** Terminal novo pelo menu "Nova sessão" (ou pelo botão do estado vazio). */
async function novoTerminal(page: Page) {
  const vazio = page.getByRole("button", { name: "Novo terminal" }).first();
  if (await page.getByRole("heading", { name: "Nenhum terminal aberto" }).isVisible()) {
    await vazio.click();
    return;
  }
  await page.getByRole("button", { name: "Nova sessão" }).click();
  await page.getByRole("menuitem", { name: "Novo terminal" }).click();
}

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
  const itens = page.getByRole("list", { name: "Terminais abertos" }).getByRole("listitem");
  await novoTerminal(page);
  await expect(itens).toHaveCount(1);
  await page.keyboard.type("echo primeira");
  await page.keyboard.press("Enter");
  await novoTerminal(page);
  await expect(itens).toHaveCount(2);
  const botoes = page.locator("[data-sessao]");
  await expect(botoes.nth(1)).toHaveAttribute("aria-current", "true");

  await botoes.nth(1).focus();
  await page.keyboard.press("ArrowUp");
  await expect(botoes.first()).toHaveAttribute("aria-current", "true");
  await expect(botoes.first()).toBeFocused();
  await expect(page.locator(".xterm-rows")).toContainText("primeira");

  const fechar = page.getByRole("button", { name: /^Fechar bash/ });
  while ((await fechar.count()) > 0) {
    await fechar.first().click();
  }
  await expect(page.getByRole("heading", { name: "Nenhum terminal aberto" })).toBeVisible();
});

test("menu de contexto limpa a tela e fecha a sessão", async ({ page }) => {
  await page.getByRole("button", { name: "Novo terminal" }).first().click();
  await page.keyboard.type("echo marca-unica");
  await page.keyboard.press("Enter");
  const tela = page.locator(".xterm-rows");
  await expect(tela).toContainText("marca-unica");

  await page.getByRole("region", { name: "bash" }).click({ button: "right", position: { x: 200, y: 120 } });
  const menu = page.getByRole("menu", { name: "Ações do terminal" });
  await expect(menu).toBeVisible();
  // Sem seleção, copiar não tem o que copiar.
  await expect(menu.getByRole("menuitem", { name: "Copiar" })).toBeDisabled();
  await menu.getByRole("menuitem", { name: "Limpar a tela" }).click();
  await expect(tela).not.toContainText("marca-unica");

  await page.getByRole("region", { name: "bash" }).click({ button: "right", position: { x: 200, y: 120 } });
  await page.getByRole("menuitem", { name: "Fechar a sessão" }).click();
  await expect(page.getByRole("heading", { name: "Nenhum terminal aberto" })).toBeVisible();
});

test("a sessão fora de vista que pede atenção aparece na lista", async ({ page }) => {
  await novoTerminal(page);
  await page.keyboard.type("avisar Aprovar o comando?");
  await page.keyboard.press("Enter");
  // Troca de sessão antes do aviso chegar: ele vem para a que não está à vista.
  await novoTerminal(page);
  const primeira = page.locator("[data-sessao]").first();
  await expect(primeira).toContainText("pede sua atenção");
  await primeira.click();
  await expect(primeira).not.toContainText("pede sua atenção");
});

test("grade de dois painéis: cada sessão no seu, sem duplicar", async ({ page }) => {
  await page.getByRole("button", { name: "Novo terminal" }).first().click();
  await page.keyboard.type("echo painel-um");
  await page.keyboard.press("Enter");

  const dois = page.getByRole("button", { name: "Dois lado a lado" });
  await dois.click();
  await expect(dois).toHaveAttribute("aria-pressed", "true");
  const p1 = page.locator('[data-painel="0"]');
  const p2 = page.locator('[data-painel="1"]');
  await expect(p2).toContainText("Escolha uma sessão na lista");

  await p2.getByRole("button", { name: "Novo terminal" }).click();
  await expect(page.locator(".xterm-rows")).toHaveCount(2);
  await page.keyboard.type("echo painel-dois");
  await page.keyboard.press("Enter");
  await expect(p2.locator(".xterm-rows")).toContainText("painel-dois");
  await expect(p1.locator(".xterm-rows")).toContainText("painel-um");
  await expect(p1.locator(".xterm-rows")).not.toContainText("painel-dois");

  // A sessão 1 já está à vista: clicar nela na lista leva o foco ao painel
  // dela, não a copia para o painel com foco.
  await page.locator("[data-sessao]").first().click();
  await expect(page.locator(".xterm-rows")).toHaveCount(2);
  await expect(p2.locator(".xterm-rows")).toContainText("painel-dois");
  await expect(page.locator("[data-sessao]").first()).toHaveAttribute("aria-current", "true");
});

test("quatro painéis e de volta a um mantém as sessões vivas", async ({ page }) => {
  await page.getByRole("button", { name: "Novo terminal" }).first().click();
  await page.keyboard.type("echo sobrevive");
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: "Quatro em grade" }).click();
  await expect(page.locator("[data-painel]")).toHaveCount(4);
  await expect(page.getByRole("separator")).toHaveCount(3);
  await page.getByRole("button", { name: "Um painel" }).click();
  await expect(page.locator("[data-painel]")).toHaveCount(1);
  await expect(page.locator(".xterm-rows")).toContainText("sobrevive");
});

test("o agente abre pelo diálogo com aprovação e sandbox escolhidos", async ({ page }) => {
  await page.getByRole("button", { name: "Abrir o agente OwCLI" }).click();
  const dialogo = page.getByRole("dialog", { name: "Abrir o agente OwCLI" });
  await expect(dialogo).toBeVisible();
  await expect(dialogo).toContainText("Pasta pessoal");
  // O primeiro do catálogo (os locais vêm antes) já vem escolhido.
  await expect(dialogo.getByLabel("Modelo")).toHaveValue("local:Qwen3-Coder-30B");
  await dialogo.getByLabel("Modelo").selectOption("openrouter:qwen/qwen3-coder");
  await dialogo.getByLabel("Quando pedir sua aprovação").selectOption("untrusted");
  await dialogo.getByLabel("O que ele pode fazer").selectOption("read-only");
  await dialogo.getByRole("button", { name: "Abrir", exact: true }).click();
  await expect(dialogo).toBeHidden();
  const tela = page.locator(".xterm-rows");
  await expect(tela).toContainText("OwCLI");
  await expect(tela).toContainText("modelo: openrouter:qwen/qwen3-coder");
  await expect(tela).toContainText("aprovação: untrusted · sandbox: read-only");
  await expect(page.locator("[data-sessao]").first()).toContainText("OwCLI");

  // A escolha fica lembrada para a próxima sessão.
  await page.getByRole("button", { name: "Nova sessão" }).click();
  await page.getByRole("menuitem", { name: "Agente OwCLI…" }).click();
  await expect(
    page.getByRole("dialog", { name: "Abrir o agente OwCLI" }).getByLabel("Modelo"),
  ).toHaveValue("openrouter:qwen/qwen3-coder");
});

test("sem modelo nenhum, o diálogo mostra de onde tirar um", async ({ page }) => {
  await page.addInitScript(() => {
    (window as { __owcliSemModelos?: boolean }).__owcliSemModelos = true;
  });
  await page.reload();
  await page.getByRole("button", { name: "OwCLI", exact: true }).click();
  await page.getByRole("button", { name: "Abrir o agente OwCLI" }).click();
  const dialogo = page.getByRole("dialog", { name: "Escolha o cérebro do OwCLI" });
  await expect(dialogo).toBeVisible();
  await expect(dialogo.getByRole("button", { name: "Abrir", exact: true })).toHaveCount(0);
  await expect(dialogo).toContainText("2 modelos na biblioteca, servidor parado.");

  // Continuar uma conversa gravada também cai aqui, em vez de abrir sem modelo.
  await dialogo.getByRole("button", { name: "Fechar" }).click();
  await page
    .getByRole("button", { name: "Continuar a conversa Migração do banco" })
    .click();
  await expect(page.getByRole("dialog", { name: "Escolha o cérebro do OwCLI" })).toBeVisible();

  // As ações levam à tela certa e fecham o diálogo.
  await page.getByRole("button", { name: "Configurar em Fontes" }).first().click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Fontes", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("o histórico continua uma conversa e renomeia outra", async ({ page }) => {
  const historico = page.getByRole("region", { name: "Histórico" });
  await expect(historico.getByRole("listitem")).toHaveCount(2);
  await expect(historico).toContainText("projetos/api");

  await historico
    .getByRole("button", { name: "Continuar a conversa Conserte o teste que falha no parser" })
    .click();
  const tela = page.locator(".xterm-rows");
  await expect(tela).toContainText("retomando 01a0e747-cde3-79e1-bd44-168d74d9976c");
  // O modelo da conversa, que ainda está no catálogo.
  await expect(tela).toContainText("modelo: local:Qwen3-Coder-30B");
  await expect(tela).toContainText("aprovação: on-request · sandbox: workspace-write");

  await historico.getByRole("button", { name: "Renomear Migração do banco" }).click();
  const dialogo = page.getByRole("dialog", { name: "Renomear a conversa" });
  const campo = dialogo.getByLabel("Nome da conversa");
  await expect(campo).toBeFocused();
  await expect(campo).toHaveValue("Migração do banco");
  await campo.fill("Migração para o Postgres");
  await campo.press("Enter");
  await expect(dialogo).toBeHidden();
  await expect(historico).toContainText("Migração para o Postgres");
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
