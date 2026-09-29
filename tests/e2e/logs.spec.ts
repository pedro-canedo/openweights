import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("navigation").first().getByRole("button", { name: "Chat", exact: true })).toBeVisible();
});

// O painel de logs: tudo que roda por baixo do app num lugar só, que se abre
// por atalho, pela barra de status ou pela paleta.

test("Ctrl+Shift+L abre o painel com o que já passou, e filtra por origem e texto", async ({ page }) => {
  await page.keyboard.press("Control+Shift+L");
  const painel = page.getByRole("region", { name: "Logs" });
  await expect(painel).toBeVisible();
  await expect(painel).toContainText("[servidor] llama-server: carregando o modelo Qwen3-8B");
  await expect(painel).toContainText("[decisor] decisor pronto na porta 11713");
  await expect(painel.getByRole("status")).toHaveText("4 linhas");

  // Desligar uma origem tira as linhas dela.
  await painel.getByRole("button", { name: "Decisor" }).click();
  await expect(painel).not.toContainText("decisor pronto");
  await expect(painel.getByRole("status")).toHaveText("3 linhas");
  await painel.getByRole("button", { name: "Decisor" }).click();

  // A busca ignora a caixa.
  await painel.getByRole("textbox", { name: "Filtrar o texto" }).fill("ERRO DE EXEMPLO");
  await expect(painel.getByRole("status")).toHaveText("1 linhas");
  await expect(painel).toContainText("alocar o KV cache");
});

test("uma linha nova aparece ao vivo, e pausar congela o que se lê", async ({ page }) => {
  await page.getByRole("button", { name: "Logs", exact: true }).click();
  const painel = page.getByRole("region", { name: "Logs" });
  await page.evaluate(() => (window as unknown as { __owLog: (o: string, t: string) => void }).__owLog("gateway", "chegou uma requisição"));
  await expect(painel).toContainText("[gateway] chegou uma requisição");

  await painel.getByRole("button", { name: "Pausar" }).click();
  await page.evaluate(() => (window as unknown as { __owLog: (o: string, t: string) => void }).__owLog("app", "durante a pausa"));
  await page.waitForTimeout(300);
  await expect(painel).not.toContainText("durante a pausa");
  await painel.getByRole("button", { name: "Retomar" }).click();
  await expect(painel).toContainText("durante a pausa");
});

test("salvar diz onde ficou e limpar esvazia o painel", async ({ page }) => {
  await page.getByRole("button", { name: "Logs", exact: true }).click();
  const painel = page.getByRole("region", { name: "Logs" });
  await painel.getByRole("button", { name: "Salvar" }).click();
  await expect(page.getByText(/Logs salvos em .*openweights-logs-.*\.txt/)).toBeVisible();

  await painel.getByRole("button", { name: "Limpar" }).click();
  await expect(painel).toContainText("Nada foi registrado ainda");
  await painel.getByRole("button", { name: "Fechar os logs" }).click();
  await expect(painel).toBeHidden();
});

test("a paleta abre os logs", async ({ page }) => {
  await page.keyboard.press("Control+k");
  await page.getByRole("dialog", { name: "Comandos" }).getByRole("combobox").fill("logs");
  await page.getByRole("option", { name: /Abrir ou fechar os logs/ }).click();
  await expect(page.getByRole("region", { name: "Logs" })).toBeVisible();
});
