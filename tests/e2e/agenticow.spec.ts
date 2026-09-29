import { expect, test } from "@playwright/test";

// O AgenticOw não baixa nada sem a pessoa saber quanto: a tela mostra o tamanho
// e só o botão começa o download.

test("a primeira abertura diz quanto vai baixar e espera o clique", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "AgenticOw", exact: true }).click();

  await expect(page.getByText("Ainda não preparado")).toBeVisible();
  await expect(page.getByText("Vai baixar 78 MB, entre o runtime do AgenticOw e o Node.js portátil")).toBeVisible();

  // Passou tempo suficiente para uma subida automática ter começado: não começou.
  await page.waitForTimeout(1500);
  await expect(page.getByText("Ainda não preparado")).toBeVisible();

  const botao = page.getByRole("button", { name: "Baixar 78 MB e abrir" });
  await expect(botao).toBeEnabled();
  await botao.click();
  await expect(page.getByText("Ainda não preparado")).toBeHidden({ timeout: 10000 });
});
