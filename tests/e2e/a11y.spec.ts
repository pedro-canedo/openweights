import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Portão de acessibilidade das telas, com os dados simulados do navegador.
// Só violações sérias e críticas — as que impedem alguém de usar a tela:
// controle sem nome, contraste abaixo do AA, diálogo sem papel.

const TELAS = ["Descobrir", "Meus Modelos", "Chat", "OwCLI", "Servidor Local", "Fontes", "Configurações"];

// Botões cheios com o roxo da marca que ainda vivem em arquivos de outra
// frente de trabalho (MyModels.tsx, ServerHeader.tsx): migram para
// `bg-accent-fill` quando ela terminar. Só o contraste fica de fora, e só aqui.
const CONTRASTE_PENDENTE = new Set(["dark:Meus Modelos", "dark:Servidor Local"]);

for (const tema of ["dark", "light"] as const) {
  test(`nenhuma tela tem violação séria de acessibilidade (tema ${tema})`, async ({ page }) => {
    await page.addInitScript((t) => localStorage.setItem("theme", t), tema);
    await page.goto("/");
    const achados: string[] = [];
    for (const tela of TELAS) {
      await page.getByRole("button", { name: tela, exact: true }).click();
      await page.waitForTimeout(300);
      let axe = new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"]);
      if (CONTRASTE_PENDENTE.has(`${tema}:${tela}`)) axe = axe.disableRules(["color-contrast"]);
      const r = await axe.analyze();
      for (const v of r.violations) {
        if (v.impact !== "serious" && v.impact !== "critical") continue;
        achados.push(`${tela}: ${v.id} em ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
      }
    }
    expect(achados).toEqual([]);
  });
}

test("as ações de uma conversa são alcançáveis pelo teclado", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  await page.getByRole("textbox", { name: "O que você quer saber... (@arquivo)" }).fill("Olá");
  await page.getByRole("button", { name: "Enviar", exact: true }).click();

  // A conversa nova aparece na lista; as ações dela antes só surgiam no hover.
  const conversa = page.locator("nav").getByRole("button", { name: /Olá/ }).first();
  await expect(conversa).toBeVisible({ timeout: 15000 });
  await conversa.focus();
  await page.keyboard.press("Tab");
  const acao = page.getByRole("button", { name: "Copiar como Markdown" });
  await expect(acao).toBeVisible();
  await expect(acao).toBeFocused();
});

test("abas andam pelas setas e só a ativa entra no Tab", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Servidor Local", exact: true }).click();
  const abas = page.getByRole("tab");
  await expect(abas.first()).toHaveAttribute("aria-selected", "true");
  await abas.first().focus();
  await page.keyboard.press("ArrowRight");
  await expect(abas.nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(abas.nth(1)).toBeFocused();
  await expect(abas.first()).toHaveAttribute("tabindex", "-1");
});
