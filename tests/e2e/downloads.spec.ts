import { expect, test } from "@playwright/test";

// Descartar um download apaga o que já foi baixado: a tela pergunta, diz
// quanto se perde e, se o cancelamento falhar, diz isso também.

async function abrirMeusModelos(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Meus Modelos", exact: true }).click();
}

test("descartar um download incompleto pergunta e diz quanto se perde", async ({ page }) => {
  await abrirMeusModelos(page);
  const cartao = page.getByText("Qwen3-8B-Q4_K_M.gguf").first();
  await expect(cartao).toBeVisible();

  await page.getByRole("button", { name: "Descartar", exact: true }).click();
  const dialogo = page.getByRole("dialog");
  await expect(dialogo).toContainText("Descartar o download de Qwen3-8B-Q4_K_M.gguf?");
  await expect(dialogo).toContainText("Os 2,1 GB já baixados serão apagados");

  // Desistir mantém o download onde estava.
  await dialogo.getByRole("button", { name: "Cancelar", exact: true }).click();
  await expect(dialogo).toBeHidden();
  await expect(cartao).toBeVisible();

  // Confirmar apaga e o cartão some.
  await page.getByRole("button", { name: "Descartar", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Descartar", exact: true }).click();
  await expect(page.getByText("Qwen3-8B-Q4_K_M.gguf")).toHaveCount(0);
});

test("se o cancelamento falha, o aviso diz o motivo e o download continua", async ({ page }) => {
  await page.addInitScript(() => {
    (globalThis as { __downloadCancelFalha?: string }).__downloadCancelFalha = "arquivo em uso";
  });
  await abrirMeusModelos(page);
  await page.getByRole("button", { name: "Descartar", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Descartar", exact: true }).click();
  await expect(page.getByText("Não deu para cancelar o download: arquivo em uso")).toBeVisible();
  await expect(page.getByText("Qwen3-8B-Q4_K_M.gguf").first()).toBeVisible();
});

test("excluir um modelo diz o tamanho que volta ao disco", async ({ page }) => {
  await abrirMeusModelos(page);
  await page.locator(".model-library-card").getByRole("button", { name: "Excluir", exact: true }).click();
  await expect(page.locator(".model-library-card")).toContainText(/\(5,1 GB\) do disco\?/);
});
