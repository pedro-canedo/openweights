import { test, expect } from "@playwright/test";

// A troca rápida de modelo: o modelo na barra de status abre os do Router,
// com carregar e descarregar ali mesmo; o seletor do Chat marca os carregados
// e lembra a última escolha.

test("o modelo da barra de status abre os modelos do Servidor Local", async ({ page }) => {
  await page.goto("/");
  const chip = page.getByRole("button", { name: /^Modelos do Servidor Local \(agora:/ });
  await chip.click();
  const painel = page.getByRole("dialog", { name: "Modelos do Servidor Local" });
  await expect(painel).toBeVisible();
  await expect(painel.getByRole("listitem")).toHaveCount(3);
  await expect(painel.getByRole("button", { name: "Descarregar Qwen3-8B-UD-Q4_K_XL" })).toBeVisible();
  await expect(painel.getByRole("button", { name: "Carregar gemma-3-4b-it-Q4_K_M" })).toBeVisible();
  await expect(painel).toContainText("Fora da memória");

  // "Configurar" leva ao Servidor Local, na aba do motor.
  await painel.getByRole("button", { name: "Configurar no Servidor Local" }).click();
  await expect(painel).toBeHidden();
  await expect(page.getByRole("tab", { name: "Desempenho" })).toHaveAttribute("aria-selected", "true");
});

test("o seletor do Chat marca o carregado e lembra a escolha", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  const seletor = page.locator("button[aria-haspopup]").filter({ hasText: /Qwen|gguf/i }).first();
  await seletor.click();
  await expect(page.getByRole("img", { name: "Carregado: responde na hora" }).first()).toBeVisible();

  // A escolha fica para a próxima conversa nova.
  const escolhido = await page.evaluate(() => localStorage.getItem("ow.chat.model"));
  expect(escolhido).toBeNull();
  const opcoes = page.locator("[data-overlay] button").filter({ hasText: /gguf/ });
  const nome = (await opcoes.first().locator("span.block").last().textContent())!.trim();
  await opcoes.first().click();
  expect(await page.evaluate(() => localStorage.getItem("ow.chat.model"))).toBe(nome);
});

// O rodapé cortava o que saía dele (overflow hidden): o monitor e este
// painel abriam "visíveis" para o toBeVisible, mas escondidos na tela. O
// elementFromPoint é o que diz o que a pessoa vê.
test("os painéis da barra de status abrem acima dela, à vista", async ({ page }) => {
  await page.goto("/");
  await page.locator("footer button").last().click();
  const monitor = page.getByRole("dialog", { name: "Monitor de hardware" });
  await expect(monitor).toBeVisible();
  const caixa = (await monitor.boundingBox())!;
  const naFrente = await page.evaluate(
    ([x, y]) => !!document.elementFromPoint(x, y)?.closest('[aria-label="Monitor de hardware"]'),
    [caixa.x + caixa.width / 2, caixa.y + 20],
  );
  expect(naFrente).toBe(true);
});
