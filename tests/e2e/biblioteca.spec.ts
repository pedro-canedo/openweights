import { expect, test, type Page } from "@playwright/test";

// Meus Modelos em tabela: ordenar, selecionar vários e excluir com o total.

async function abrirTabela(page: Page, antes?: () => Promise<void>) {
  await page.addInitScript(() => {
    (globalThis as { __modelosVarios?: boolean }).__modelosVarios = true;
  });
  await antes?.();
  await page.goto("/");
  await page.getByRole("button", { name: "Meus Modelos", exact: true }).click();
  await page.getByRole("group", { name: "Visão da biblioteca" }).getByRole("button", { name: "Tabela" }).click();
  await expect(page.getByRole("table")).toBeVisible();
}

const linhas = (page: Page) => page.getByRole("table").locator("tbody tr");

test("a tabela ordena por coluna e diz o sentido para o leitor de tela", async ({ page }) => {
  await abrirTabela(page);
  await expect(linhas(page)).toHaveCount(3);
  const nomes = () => linhas(page).locator("td:nth-child(2)").allInnerTexts();
  expect(await nomes()).toEqual(// Ordem natural: o 8 vem antes do 30.
  ["Llama-3.2-3B-Q8_0.gguf", "Qwen3-8B-UD-Q4_K_XL.gguf", "Qwen3-30B-A3B-Q4_K_M.gguf"]);

  const tamanho = page.getByRole("columnheader", { name: /Tamanho/ });
  await tamanho.getByRole("button").click();
  await expect(tamanho).toHaveAttribute("aria-sort", "descending");
  expect((await nomes())[0]).toBe("Qwen3-30B-A3B-Q4_K_M.gguf");
  await tamanho.getByRole("button").click();
  await expect(tamanho).toHaveAttribute("aria-sort", "ascending");
  expect((await nomes())[0]).toBe("Llama-3.2-3B-Q8_0.gguf");
});

test("a escolha de visão fica lembrada", async ({ page }) => {
  await abrirTabela(page);
  await page.reload();
  await page.getByRole("button", { name: "Meus Modelos", exact: true }).click();
  await expect(page.getByRole("table")).toBeVisible();
});

test("selecionar vários mostra o total e só exclui depois do sim", async ({ page }) => {
  await abrirTabela(page);
  await page.getByRole("checkbox", { name: "Selecionar Llama-3.2-3B-Q8_0.gguf" }).check();
  await page.getByRole("checkbox", { name: "Selecionar Qwen3-8B-UD-Q4_K_XL.gguf" }).check();
  await expect(page.getByRole("status").filter({ hasText: "2 selecionados" })).toContainText("8,3 GB");

  await page.getByRole("button", { name: "Excluir selecionados" }).click();
  const dialogo = page.getByRole("dialog", { name: "Excluir 2 modelos do disco?" });
  await expect(dialogo).toContainText("Isso libera 8,3 GB e não tem volta.");
  // Desistir não apaga nada.
  await dialogo.getByRole("button", { name: "Cancelar" }).click();
  await expect(linhas(page)).toHaveCount(3);

  await page.getByRole("button", { name: "Excluir selecionados" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Excluir", exact: true }).click();
  await expect(linhas(page)).toHaveCount(1);
  await expect(page.getByText("2 modelos excluídos (8,3 GB liberados).")).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "selecionados" })).toHaveCount(0);
});

test("excluir uma linha diz o tamanho, e o que não apaga é dito sem sumir da lista", async ({ page }) => {
  await abrirTabela(page, async () => {
    await page.addInitScript(() => {
      (globalThis as { __modeloNaoApaga?: string }).__modeloNaoApaga = "Qwen3-30B";
    });
  });
  await page.getByRole("button", { name: "Excluir Qwen3-30B-A3B-Q4_K_M.gguf" }).click();
  await expect(page.getByRole("dialog")).toContainText("Isso libera 19 GB");
  await page.getByRole("dialog").getByRole("button", { name: "Excluir", exact: true }).click();
  await expect(page.getByText(/Não deu para excluir 1: Qwen3-30B-A3B-Q4_K_M.gguf: arquivo em uso/)).toBeVisible();
  await expect(linhas(page)).toHaveCount(3);
});
