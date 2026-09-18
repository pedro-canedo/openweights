import { expect, test } from "@playwright/test";

test("library search keeps model actions and offers a recoverable empty result", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Meus Modelos", exact: true }).click();
  const search = page.getByRole("textbox", { name: "Buscar nos seus modelos…" });
  await expect(page.locator(".model-library-card")).toHaveCount(1);
  await search.fill("no-matching-model");
  await expect(page.getByRole("status").filter({ hasText: "Nenhum modelo corresponde" })).toBeVisible();
  await search.fill("qwen");
  await expect(page.locator(".model-library-card")).toHaveCount(1);
  await page.locator(".model-library-card").getByRole("button", { name: "Excluir", exact: true }).click();
  await page.locator(".model-library-card").getByRole("button", { name: "Cancelar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Conversar", exact: true })).toBeVisible();
});

test("source cards open their controls and settings stay usable in a narrow light window", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", e => errors.push(e.message));
  await page.setViewportSize({ width: 900, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Fontes", exact: true }).click();
  await page.locator(".source-card").filter({ hasText: "9router" }).click();
  await expect(page.getByRole("tab", { name: "9router", exact: true })).toHaveAttribute("aria-selected", "true");
  await page.locator(".source-card").filter({ hasText: "Máquina local" }).click();
  await expect(page.getByRole("tab", { name: "Máquina local (llama.cpp)", exact: true })).toHaveAttribute("aria-selected", "true");
  await page.getByRole("button", { name: "Abrir servidor local", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Servidor Local", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Configurações", exact: true }).click();
  await page.getByRole("combobox", { name: "Tema", exact: true }).selectOption("light");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByText("Versão do motor e detalhes da instalação", { exact: true }).click();
  await expect(page.getByText("Esperado por esta versão", { exact: true })).toBeVisible();
  await page.getByRole("combobox", { name: "Idioma", exact: true }).selectOption("en");
  await expect(page.getByRole("heading", { name: "Settings", exact: true })).toBeVisible();
  const overflow = await page.locator(".workspace-page").evaluate(el => el.scrollWidth > el.clientWidth);
  expect(overflow).toBe(false);
  expect(errors).toEqual([]);
});

for (const running of [false, true]) {
  test(`9router update preserves settings and ${running ? "running" : "stopped"} state`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", e => errors.push(e.message));
    await page.setViewportSize({ width: 900, height: 900 });
    await page.goto("/");
    await page.getByRole("button", { name: "Fontes", exact: true }).click();
    await page.locator(".source-card").filter({ hasText: "9router" }).click();
    await page.getByRole("button", { name: "Instalar", exact: true }).click();
    await expect(page.getByText("Nova versão disponível: 0.5.75", { exact: true })).toBeVisible();
    if (running) await page.getByRole("button", { name: "Iniciar", exact: true }).click();
    await page.getByRole("button", { name: "Atualizar", exact: true }).click();
    await expect(page.getByRole("button", { name: "Atualizando…", exact: true })).toBeDisabled();
    await expect(page.getByText("9router atualizado com sucesso.", { exact: true })).toBeVisible();
    await expect(page.getByText("versão 0.5.75", { exact: true })).toBeVisible();
    await expect(page.getByText("demo-password", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: running ? "Parar" : "Iniciar", exact: true })).toBeEnabled();
    await expect(page.getByRole("button", { name: "Atualizar", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "Verificar atualização", exact: true }).click();
    await expect(page.getByText("Você já está na versão mais recente disponível para esta instalação.", { exact: true })).toBeVisible();
    expect(await page.locator(".workspace-page").evaluate(el => el.scrollWidth > el.clientWidth)).toBe(false);
    expect(errors).toEqual([]);
  });
}
