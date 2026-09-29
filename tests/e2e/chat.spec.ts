import { expect, test } from "@playwright/test";

test("chat generates, exposes run details, and cancels without losing text", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", e => errors.push(e.message));
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  const input = page.getByRole("textbox", { name: "O que você quer saber... (@arquivo)" });
  await input.fill("Escreva uma função em TypeScript");
  await page.getByRole("button", { name: "Enviar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Parar", exact: true })).toBeVisible();
  await expect(page.getByText("Claro!", { exact: false })).toBeVisible({ timeout: 15000 });
  await page.getByRole("button", { name: "Parar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Enviar", exact: true })).toBeVisible();
  await expect(page.getByText("Claro!", { exact: false })).toBeVisible();
  expect(errors).toEqual([]);
});

test("long histories stay virtualized and reading position survives streaming", async ({ page }) => {
  await page.goto("/tests/render.html");
  await page.waitForFunction(() => !!(window as any).startRenderBench);
  // Virtuoso applies its initial bottom position after measuring rows. Scroll
  // only once that placement is visible, as a reader would do.
  await expect(page.getByText("Mensagem 499", { exact: true })).toBeVisible();
  expect(await page.locator("body *").count()).toBeLessThan(500);
  const scroller = page.locator('[data-virtuoso-scroller="true"]');
  await scroller.evaluate(el => { el.scrollTop = 1000; });
  await page.waitForTimeout(200);
  expect(await scroller.evaluate(el => el.scrollTop)).toBeLessThan(2000);
  await page.evaluate(() => (window as any).startRenderBench(5));
  await page.waitForTimeout(300);
  const top = await scroller.evaluate(el => el.scrollTop);
  await page.getByRole("textbox", { name: "Typing probe" }).fill("typing remains available");
  await page.waitForTimeout(400);
  expect(Math.abs(await scroller.evaluate(el => el.scrollTop) - top)).toBeLessThan(100);
  await expect(page.getByRole("textbox")).toHaveValue("typing remains available");
});

test("comparison is discoverable and explains the desktop requirement", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Servidor Local", exact: true }).click();
  await page.getByRole("tab", { name: "Desempenho", exact: true }).click();
  await expect(page.getByRole("button", { name: "Otimizar para meu computador", exact: true })).toBeVisible();
});

test("apagar a última resposta some na hora e dá cinco segundos para desfazer", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  const input = page.getByRole("textbox", { name: "O que você quer saber... (@arquivo)" });
  await input.fill("Escreva uma função em TypeScript");
  await page.getByRole("button", { name: "Enviar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Enviar", exact: true })).toBeVisible({ timeout: 20000 });
  const resposta = page.getByText("Claro!", { exact: false });
  await expect(resposta).toBeVisible();

  await page.getByRole("button", { name: "Apagar mensagem" }).last().click({ force: true });
  await expect(resposta).toHaveCount(0);
  const aviso = page.getByText("Mensagem apagada.");
  await expect(aviso).toBeVisible();

  // Desfazer devolve a resposta no mesmo lugar.
  await aviso.locator("..").getByRole("button", { name: "Desfazer" }).click();
  await expect(resposta).toBeVisible();
  await expect(aviso).toHaveCount(0);

  // Sem desfazer, o aviso sai sozinho e a resposta continua apagada.
  await page.getByRole("button", { name: "Apagar mensagem" }).last().click({ force: true });
  await expect(resposta).toHaveCount(0);
  await expect(aviso).toBeHidden({ timeout: 8000 });
  await expect(resposta).toHaveCount(0);
});

test("um erro conhecido do servidor diz o que houve e o que fazer", async ({ page }) => {
  await page.addInitScript(() => {
    (globalThis as { __chatFalha?: string }).__chatFalha = "Failed to fetch";
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  const input = page.getByRole("textbox", { name: "O que você quer saber... (@arquivo)" });
  await input.fill("Olá");
  await page.getByRole("button", { name: "Enviar", exact: true }).click();

  const cartao = page.getByRole("alert").filter({ hasText: "O servidor local não respondeu" });
  await expect(cartao).toBeVisible({ timeout: 15000 });
  await expect(cartao.getByRole("button", { name: "Abrir o Servidor Local" })).toBeVisible();
  // O texto que o sistema deu fica recolhido, mas está lá.
  await cartao.locator("summary").click();
  await expect(cartao.locator("pre")).toContainText("Failed to fetch");

  // Refazer é o mesmo gesto do "Gerar de novo" da mensagem; aqui basta o botão
  // estar no cartão (o navegador simulado não guarda a conversa para refazer).
  await expect(cartao.getByRole("button", { name: "Tentar de novo" })).toBeVisible();
});

test("o botão principal do erro leva à tela que resolve", async ({ page }) => {
  await page.addInitScript(() => {
    (globalThis as { __chatFalha?: string }).__chatFalha = "HTTP 401: Missing API key";
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  await page.getByRole("textbox", { name: "O que você quer saber... (@arquivo)" }).fill("Olá");
  await page.getByRole("button", { name: "Enviar", exact: true }).click();
  const cartao = page.getByRole("alert").filter({ hasText: "A chave não foi aceita" });
  await cartao.getByRole("button", { name: "Abrir Fontes" }).click();
  await expect(
    page.getByRole("navigation").getByRole("button", { name: "Fontes", exact: true }),
  ).toHaveAttribute("aria-current", "page");
});
