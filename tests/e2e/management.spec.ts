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
