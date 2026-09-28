import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// O kit de componentes existe para que foco, Esc e leitor de tela funcionem
// igual em todo painel do app. Nada disso aparece num print; aqui se prova
// pelo que o navegador faz com o teclado e pelo que o axe encontra.

const focado = (page: Page) =>
  page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    return el?.id || el?.getAttribute("aria-label") || el?.textContent?.trim() || "";
  });

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ui-kit.html");
});

test("o diálogo prende o Tab, fecha no Esc e devolve o foco a quem abriu", async ({ page }) => {
  await page.locator("#abrir-dialogo").click();
  const dialogo = page.getByRole("dialog", { name: "Diálogo de teste" });
  await expect(dialogo).toBeVisible();
  await expect(dialogo).toHaveAttribute("aria-modal", "true");

  // O foco nasce dentro e não sai, nem para a frente nem para trás.
  for (let i = 0; i < 6; i++) {
    await page.keyboard.press("Tab");
    expect(await dialogo.evaluate((d) => d.contains(document.activeElement))).toBe(true);
  }
  for (let i = 0; i < 6; i++) {
    await page.keyboard.press("Shift+Tab");
    expect(await dialogo.evaluate((d) => d.contains(document.activeElement))).toBe(true);
  }

  await page.keyboard.press("Escape");
  await expect(dialogo).toBeHidden();
  expect(await focado(page)).toBe("abrir-dialogo");
});

test("confirmar algo destrutivo nasce no Cancelar e responde pela promessa", async ({ page }) => {
  const resposta = page.getByTestId("resposta");

  await page.locator("#abrir-confirmacao").click();
  const dialogo = page.getByRole("dialog", { name: "Apagar o modelo?" });
  await expect(dialogo).toContainText("Os 14 GB saem do disco.");
  expect(await focado(page)).toBe("Cancelar");
  await page.keyboard.press("Enter");
  await expect(resposta).toHaveText("não");

  await page.locator("#abrir-confirmacao").click();
  await dialogo.getByRole("button", { name: "Apagar" }).click();
  await expect(resposta).toHaveText("sim");

  await page.locator("#abrir-confirmacao").click();
  await page.keyboard.press("Escape");
  await expect(resposta).toHaveText("não");
  expect(await focado(page)).toBe("abrir-confirmacao");
});

test("o popover diz que está aberto e fecha no Esc e no clique fora", async ({ page }) => {
  const gatilho = page.locator("#abrir-popover");
  await expect(gatilho).toHaveAttribute("aria-expanded", "false");
  await gatilho.click();
  await expect(gatilho).toHaveAttribute("aria-expanded", "true");
  const painel = page.getByRole("dialog", { name: "Detalhes" });
  await expect(painel).toBeVisible();
  // A marca que faz a tela do AgenticOw esconder a webview nativa.
  await expect(painel).toHaveAttribute("data-overlay", "");

  await page.keyboard.press("Escape");
  await expect(painel).toBeHidden();
  expect(await focado(page)).toBe("abrir-popover");

  await gatilho.click();
  await expect(painel).toBeVisible();
  await page.getByRole("heading", { name: "Kit de componentes" }).click();
  await expect(painel).toBeHidden();
});

test("o menu anda pelas setas e escolher fecha e devolve o foco", async ({ page }) => {
  await page.locator("#abrir-menu").click();
  const menu = page.getByRole("menu", { name: "Mais ações" });
  await expect(menu).toBeVisible();
  expect(await focado(page)).toBe("Renomear");
  await page.keyboard.press("ArrowDown");
  expect(await focado(page)).toBe("Exportar");
  await page.keyboard.press("End");
  expect(await focado(page)).toBe("Apagar");
  await page.keyboard.press("ArrowDown");
  expect(await focado(page)).toBe("Renomear");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("escolha")).toHaveText("apagar");
  await expect(menu).toBeHidden();
  expect(await focado(page)).toBe("abrir-menu");
});

test("a dica abre no foco do teclado e fica ligada ao botão", async ({ page }) => {
  await page.locator("#com-dica").focus();
  const dica = page.getByRole("tooltip");
  await expect(dica).toHaveText("Copia o endereço do servidor.");
  const id = await dica.getAttribute("id");
  await expect(page.locator("#com-dica")).toHaveAttribute("aria-describedby", id!);
  await page.keyboard.press("Escape");
  await expect(dica).toBeHidden();
});

test("o aviso é anunciado e a ação dele funciona", async ({ page }) => {
  await page.locator("#avisar").click();
  const regiao = page.locator("[aria-live='polite']");
  await expect(regiao).toContainText("Modelo pronto.");
  await regiao.getByRole("button", { name: "Conversar" }).click();
  await expect(page.getByTestId("escolha")).toHaveText("conversar");
  await expect(regiao).not.toContainText("Modelo pronto.");
});

test("o divisor responde ao teclado dentro dos limites", async ({ page }) => {
  const divisor = page.getByRole("separator", { name: "Largura da lista" });
  await expect(divisor).toHaveAttribute("aria-valuenow", "240");
  await divisor.focus();
  await page.keyboard.press("ArrowRight");
  await expect(divisor).toHaveAttribute("aria-valuenow", "256");
  await page.keyboard.press("Shift+ArrowLeft");
  await expect(divisor).toHaveAttribute("aria-valuenow", "192");
  await page.keyboard.press("Home");
  await expect(divisor).toHaveAttribute("aria-valuenow", "120");
});

test("o ? de ajuda abre a explicação", async ({ page }) => {
  await page.getByRole("button", { name: "O que é quantização?" }).click();
  await expect(page.getByRole("dialog", { name: "O que é quantização?" })).toContainText(
    "Uma versão comprimida do modelo",
  );
});

for (const tema of ["escuro", "claro"]) {
  test(`sem violação séria de acessibilidade no tema ${tema}`, async ({ page }) => {
    await page.goto(`/tests/ui-kit.html${tema === "claro" ? "?tema=claro" : ""}`);
    await expect(page.getByRole("heading", { name: "Kit de componentes" })).toBeVisible();
    // Com os painéis abertos também: é aí que costumam faltar nome e papel.
    await page.locator("#abrir-popover").click();
    const resultado = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
      .analyze();
    const graves = resultado.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(
      graves.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`),
    ).toEqual([]);
  });
}
