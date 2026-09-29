import { expect, test, type Page } from "@playwright/test";

// Exportar e importar a configuração, em Configurações.

const nav = (page: Page) => page.getByRole("navigation").first();

test.beforeEach(async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  await expect(nav(page).getByRole("button", { name: "Chat", exact: true })).toBeVisible();
  await nav(page).getByRole("button", { name: "Configurações", exact: true }).click();
});

test("exportar copia o JSON, e as chaves só vão se marcadas", async ({ page }) => {
  await page.getByRole("button", { name: "Copiar", exact: true }).click();
  await expect(page.getByText("Configuração copiada.")).toBeVisible();
  const sem = JSON.parse(await page.evaluate(() => navigator.clipboard.readText()));
  expect(sem.app).toBe("OpenWeights");
  expect(sem.secrets).toBeUndefined();

  await page.getByRole("checkbox", { name: /Incluir chaves e senhas/ }).check();
  await expect(page.getByRole("note")).toContainText("não o compartilhe");
  await page.getByRole("button", { name: "Copiar", exact: true }).click();
  const com = JSON.parse(await page.evaluate(() => navigator.clipboard.readText()));
  expect(com.secrets).toBeDefined();
});

test("importar confere o arquivo, mostra o que traz e só aplica depois do sim", async ({ page }) => {
  const campo = page.getByRole("textbox", { name: "Cole aqui o JSON exportado" });
  const aplicar = page.getByRole("button", { name: "Conferir e aplicar" });
  await expect(aplicar).toBeDisabled();

  // Lixo: o motivo aparece e nada é aplicado.
  await campo.fill("isto não é json");
  await aplicar.click();
  await expect(page.getByRole("alert").filter({ hasText: "O texto não é um JSON válido." })).toBeVisible();
  await campo.fill('{"app":"Outro","format":1}');
  await aplicar.click();
  await expect(page.getByText("Este texto não é uma exportação do OpenWeights.")).toBeVisible();
  await campo.fill('{"app":"OpenWeights","format":9}');
  await aplicar.click();
  await expect(page.getByText("O arquivo é do formato 9")).toBeVisible();

  // Arquivo bom: pede confirmação com o que ele traz.
  await campo.fill(
    JSON.stringify({ app: "OpenWeights", format: 1, server: { server_port: "11711" }, profiles: [{ model: "a", profile: {} }, { model: "b", profile: {} }], presets: [] }),
  );
  await aplicar.click();
  const dialogo = page.getByRole("dialog", { name: "Aplicar esta configuração?" });
  await expect(dialogo).toContainText("2 perfis de modelo, 0 presets e 1 ajustes do servidor");
  await dialogo.getByRole("button", { name: "Cancelar" }).click();
  await expect(campo).not.toHaveValue("");

  await aplicar.click();
  await page.getByRole("dialog").getByRole("button", { name: "Aplicar", exact: true }).click();
  await expect(page.getByText("Configuração aplicada: 2 perfis, 0 presets e 1 ajustes.")).toBeVisible();
  await expect(campo).toHaveValue("");
});
