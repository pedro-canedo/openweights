import { test, expect } from "@playwright/test";

test("one-click optimization presents ready rates and supports restoring", async ({ page }) => {
  await page.goto("/tests/optimization.html");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(page.getByText("Configuração pronta", { exact: true })).toBeVisible();
  await expect(page.getByText("28.0", { exact: true })).toBeVisible();
  await expect(page.getByRole("table")).not.toBeVisible();
  await page.getByRole("button", { name: "Usar configuração" }).click();
  await expect(page.getByRole("button", { name: /Restaurar/ })).toBeVisible();
  await page.getByRole("button", { name: /Restaurar/ }).click();
  await expect(page.getByRole("button", { name: "Usar configuração" })).toBeVisible();
});

test("cancellation releases the interface without presenting an incomplete result", async ({ page }) => {
  await page.goto("/tests/optimization.html?scenario=cancel");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await page.getByRole("button", { name: /Cancelar/ }).click();
  await expect(page.getByRole("button", { name: "Otimizar para meu computador" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Usar configuração" })).toHaveCount(0);
});

test("package failures are explained without hiding successful measurements", async ({ page }) => {
  await page.goto("/tests/optimization.html?scenario=unavailable");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(page.getByText(/Algumas opções não puderam ser verificadas/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Usar configuração" })).toBeEnabled();
});

test("sem vencedor, a tela nao oferece desfazer o que nao fez", async ({ page }) => {
  await page.goto("/tests/optimization.html?scenario=inconclusive");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(
    page.getByText("Nenhuma opção reduziu o tempo total de forma consistente."),
  ).toBeVisible();
  // "Restaurar" prometeria desfazer uma mudança que não houve — e o clique
  // reiniciava o motor à toa.
  await expect(page.getByRole("button", { name: /Restaurar/ })).toHaveCount(0);
  // "Usar configuração" fica visível, porém desabilitado: não há o que usar.
  await expect(page.getByRole("button", { name: "Usar configuração" })).toBeDisabled();
  await expect(page.getByRole("radio", { name: "Referência" })).toBeChecked();
});

test("o veredito anterior some enquanto uma nova medicao roda", async ({ page }) => {
  await page.goto("/tests/optimization.html");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(page.getByText("Configuração pronta", { exact: true })).toBeVisible();
  // Segunda rodada: o veredito da primeira não pode ficar na tela junto do
  // progresso, dando a impressão de que a bateria em curso já concluiu.
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(page.getByText("Configuração pronta", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Configuração pronta", { exact: true })).toBeVisible();
});


test("a faster generation option can be applied without a total-time recommendation", async ({ page }) => {
  await page.goto("/tests/optimization.html?scenario=fast-generation");
  await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
  await expect(page.getByText("143.0", { exact: true })).toBeVisible();
  await page.getByRole("radio", { name: "Opção 1", exact: true }).check();
  await page.getByRole("button", { name: "Usar configuração", exact: true }).click();
  await expect(page.getByText("Configuração em uso", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__applied.at(-1))).toMatchObject({ armIndex: 1, restore: false });
  await page.getByRole("button", { name: "Restaurar configuração anterior" }).click();
  await expect(page.getByText("Configuração em uso", { exact: true })).toHaveCount(0);
});

test("history applies an exact saved profile with the server running and refreshes current", async ({ page }) => {
  await page.goto("/tests/optimization.html?scenario=history");
  const row = page.getByRole("article").filter({ hasText: "33.1" });
  await row.getByRole("button", { name: "Usar configuração" }).click();
  await expect(page.getByRole("status")).toHaveText("Configuração aplicada e modelo carregado.");
  await expect(row.getByText("atual", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__applied.at(-1).profile)).toEqual({ctx:8192, threads:16, engine:"official"});
  await expect(page.getByText("Perfil não salvo", { exact:true })).toBeVisible();
});

for (const scenario of ["history-busy", "history-failure"]) test(`${scenario} never reports an unapplied profile as active`, async ({ page }) => {
  await page.goto(`/tests/optimization.html?scenario=${scenario}`);
  await page.getByRole("article").filter({ hasText: "33.1" }).getByRole("button").click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(page.getByRole("article").filter({ hasText: "27.6" }).getByText("atual", {exact:true})).toBeVisible();
});

test("optimization and history fit narrow screens", async ({ page }) => {
  await page.setViewportSize({width:390,height:844});
  for (const scenario of ["fast-generation", "history"]) {
    await page.goto(`/tests/optimization.html?scenario=${scenario}`);
    if (scenario === "fast-generation") {
      await page.getByRole("button", { name: "Otimizar para meu computador" }).click();
      await expect(page.getByText("143.0", {exact:true})).toBeVisible();
    } else await expect(page.getByText("33.1", {exact:false}).first()).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  }
});
