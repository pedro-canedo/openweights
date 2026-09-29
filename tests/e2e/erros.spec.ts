import { expect, test } from "@playwright/test";

// O erro que aparece numa tela diz o que houve e leva a quem resolve.

test("o servidor que não sobe explica o motivo e leva a instalar o motor", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Servidor Local", exact: true }).click();
  await page.evaluate(() => {
    (globalThis as { __servidorFalha?: string }).__servidorFalha =
      "o runtime do llama.cpp ainda não está instalado";
  });
  await page.getByRole("button", { name: "Iniciar", exact: true }).click();

  const cartao = page.getByRole("alert").filter({ hasText: "O motor de inferência ainda não está instalado" });
  await expect(cartao).toBeVisible();
  // Já se está no Servidor Local: o botão "abrir o Servidor Local" não entra.
  await expect(cartao.getByRole("button", { name: "Abrir o Servidor Local" })).toHaveCount(0);

  await cartao.getByRole("button", { name: "Instalar o motor" }).click();
  await expect(
    page.getByRole("navigation").getByRole("button", { name: "Configurações", exact: true }),
  ).toHaveAttribute("aria-current", "page");
});

test("um erro do servidor que dá para refazer oferece tentar de novo", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Servidor Local", exact: true }).click();
  await page.evaluate(() => {
    (globalThis as { __servidorFalha?: string }).__servidorFalha =
      "o modelo ficou 120s sem emitir nada (antes do primeiro token)";
  });
  await page.getByRole("button", { name: "Iniciar", exact: true }).click();
  const cartao = page.getByRole("alert").filter({ hasText: "O modelo parou de responder" });
  await expect(cartao).toBeVisible();

  // Refazer é ligar o servidor de novo — e a segunda vez dá certo.
  await cartao.getByRole("button", { name: "Tentar de novo" }).click();
  await expect(page.getByRole("button", { name: "Parar", exact: true })).toBeVisible({ timeout: 10000 });
  await expect(cartao).toHaveCount(0);
});

test("a prévia do comando diz se o que roda é o que está configurado, e copia como shell", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.addInitScript(() => {
    (globalThis as { __configPendente?: boolean }).__configPendente = true;
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Servidor Local", exact: true }).click();
  await page.getByRole("tab", { name: "Desempenho" }).click();

  const previa = page.getByRole("region", { name: "Prévia do comando" });
  // Parado: é o que vai rodar.
  await expect(previa).toContainText("O que vai rodar ao iniciar");
  await page.getByRole("button", { name: "Iniciar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Parar", exact: true })).toBeVisible();
  await expect(previa).toContainText("Mudanças pendentes");
  await expect(previa).toContainText("Reinicie o servidor para aplicar");

  await previa.getByRole("button", { name: "Copiar como comando" }).click();
  await expect(page.getByText("Comando copiado.")).toBeVisible();
  const colado = await page.evaluate(() => navigator.clipboard.readText());
  expect(colado).toContain("GGML_OP_OFFLOAD_MIN_BATCH=32 \\\nllama-server --models-dir /dados/models");
});
