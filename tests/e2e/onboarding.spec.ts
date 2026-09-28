import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// O primeiro uso, como numa instalação nova (sem motor e sem modelo): o
// computador, o motor e o primeiro modelo, com "Baixar e conversar".

async function instalacaoNova(page: Page) {
  await page.addInitScript(() => {
    (globalThis as { __owInstalacaoNova?: boolean }).__owInstalacaoNova = true;
  });
  await page.goto("/");
  return page.getByRole("dialog", { name: "Bem-vindo ao OpenWeights" });
}

test("do computador ao primeiro modelo, em três passos", async ({ page }) => {
  const dialogo = await instalacaoNova(page);
  await expect(dialogo).toBeVisible();
  const passos = dialogo.getByRole("listitem");
  // O passo feito troca o número pelo ✓.
  await expect(passos).toHaveText(["Seu computador", "2O motor", "3Primeiro modelo"]);
  // Com o hardware lido, o passo de agora é o motor.
  await expect(dialogo.locator('[aria-current="step"]')).toHaveText(/O motor/);

  await dialogo.getByRole("button", { name: "Instalar o motor" }).click();
  await expect(dialogo.getByRole("heading", { name: "Escolha o seu primeiro modelo" })).toBeVisible();
  await expect(dialogo.locator('[aria-current="step"]')).toHaveText(/Primeiro modelo/);
  await expect(dialogo).toContainText("Motor de IA pronto!");

  // Três sugestões, cada uma com a versão recomendada e o veredito.
  const baixar = dialogo.getByRole("button", { name: /^Baixar .* e conversar$/ });
  await expect(baixar).toHaveCount(3);
  await expect(dialogo).toContainText("Qwen3.6 35B-A3B");
  await expect(dialogo).toContainText("UD-Q4_K_XL");

  await dialogo.getByRole("button", { name: "Baixar Qwen3.6 35B-A3B e conversar" }).click();
  await expect(dialogo).toContainText("Baixando Qwen3.6 35B-A3B…");
  await expect(dialogo.getByRole("progressbar")).toBeVisible();

  // Fechar não cancela: o download segue no painel de downloads.
  await dialogo.getByRole("button", { name: "Continuar em segundo plano" }).click();
  await expect(dialogo).toBeHidden();
});

test("escolher em Descobrir fecha o primeiro uso e leva até lá", async ({ page }) => {
  const dialogo = await instalacaoNova(page);
  await dialogo.getByRole("button", { name: "Instalar o motor" }).click();
  await dialogo.getByRole("button", { name: "Escolher em Descobrir" }).click();
  await expect(dialogo).toBeHidden();
  await expect(page.getByRole("button", { name: "Descobrir", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("o primeiro uso não tem violação séria de acessibilidade", async ({ page }) => {
  const dialogo = await instalacaoNova(page);
  await dialogo.getByRole("button", { name: "Instalar o motor" }).click();
  await expect(dialogo.getByRole("button", { name: /^Baixar .* e conversar$/ })).toHaveCount(3);
  const r = await new AxeBuilder({ page })
    .include('[role="dialog"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const graves = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(graves.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
});
