import { test, expect, type Page } from "@playwright/test";

// O Descobrir com o download de verdade por trás de cada botão: o que já está
// na biblioteca oferece conversar, o que baixa mostra a porcentagem, o que
// falha diz por quê, e o que termina avisa de qualquer tela.

const nav = (page: Page) => page.getByRole("navigation").first();

async function abrirDescobrir(page: Page) {
  await page.goto("/");
  await nav(page).getByRole("button", { name: "Descobrir", exact: true }).click();
  await expect(page.getByRole("group", { name: "UD-Q4_K_XL" })).toBeVisible();
}

/** As versões que não são a recomendada ficam recolhidas. */
async function outrasVersoes(page: Page) {
  await page.getByText("Outras versões (5)").click();
  await expect(page.getByRole("group", { name: "UD-Q2_K_XL" })).toBeVisible();
}

test("a recomendada vem no topo com o tamanho no botão, e as outras recolhidas", async ({ page }) => {
  await abrirDescobrir(page);
  const recomendada = page.getByRole("group", { name: "UD-Q4_K_XL" });
  await expect(recomendada.getByRole("button", { name: /^Baixar recomendado \(18 GB\)$/ })).toBeVisible();
  await expect(page.getByRole("group", { name: "UD-Q2_K_XL" })).toBeHidden();
  await outrasVersoes(page);
  await expect(page.getByRole("group", { name: "BF16" })).toBeVisible();
});

test("sem espaço em disco, o motivo vem em palavras", async ({ page }) => {
  await page.addInitScript(() => {
    (globalThis as { __downloadFalha?: string }).__downloadFalha = "disk-space:20000000000:5000000000";
  });
  await abrirDescobrir(page);
  await page
    .getByRole("group", { name: "UD-Q4_K_XL" })
    .getByRole("button", { name: /^Baixar recomendado/ })
    .click();
  await expect(page.getByText(/^Falta espaço em disco: este download precisa de 19 GB livres/)).toBeVisible();
  await expect(page.getByText(/e há 4,7 GB\.$/)).toBeVisible();
});

test("o que já está na biblioteca oferece conversar", async ({ page }) => {
  await abrirDescobrir(page);
  await outrasVersoes(page);
  const q3 = page.getByRole("group", { name: "Q3_K_M" });
  await expect(q3).toContainText("Na biblioteca");
  await q3.getByRole("button", { name: "Conversar" }).click();
  await expect(nav(page).getByRole("button", { name: "Chat", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("baixar mostra o progresso de verdade, e o fim avisa com o caminho do Chat", async ({ page }) => {
  await page.addInitScript(() => {
    (globalThis as { __downloadsTerminam?: boolean }).__downloadsTerminam = true;
  });
  await abrirDescobrir(page);
  await outrasVersoes(page);
  const q2 = page.getByRole("group", { name: "UD-Q2_K_XL" });
  await q2.getByRole("button", { name: "Baixar", exact: true }).click();
  await expect(q2.getByRole("button", { name: "Baixando… 24%" })).toBeDisabled();

  // Terminou: o botão vira "Conversar" e o aviso leva ao Chat.
  await expect(q2.getByRole("button", { name: "Conversar" })).toBeVisible();
  await expect(q2).toContainText("Na biblioteca");
  const texto = page.getByText("model-UD-Q2_K_XL.gguf está pronto para conversar.");
  await expect(texto).toBeVisible();
  await texto.locator("..").getByRole("button", { name: "Conversar" }).click();
  await expect(nav(page).getByRole("button", { name: "Chat", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("um download que não começa diz por quê e o botão volta a baixar", async ({ page }) => {
  await page.addInitScript(() => {
    (globalThis as { __downloadFalha?: string }).__downloadFalha = "espaço insuficiente em disco";
  });
  await abrirDescobrir(page);
  await outrasVersoes(page);
  const q2 = page.getByRole("group", { name: "UD-Q2_K_XL" });
  await q2.getByRole("button", { name: "Baixar", exact: true }).click();
  await expect(page.getByText("O download não começou: espaço insuficiente em disco")).toBeVisible();
  await expect(q2.getByRole("button", { name: "Baixar", exact: true })).toBeEnabled();
});

test("o botão não fica preso em Baixando quando o download segue", async ({ page }) => {
  await abrirDescobrir(page);
  await outrasVersoes(page);
  const q2 = page.getByRole("group", { name: "UD-Q2_K_XL" });
  await q2.getByRole("button", { name: "Baixar", exact: true }).click();
  await expect(q2.getByRole("button", { name: /^Baixando… \d+%$/ })).toBeVisible();
  // Trocar de modelo e voltar: o estado vem do download, não de um clique lembrado.
  await nav(page).getByRole("button", { name: "Chat", exact: true }).click();
  await nav(page).getByRole("button", { name: "Descobrir", exact: true }).click();
  await outrasVersoes(page);
  await expect(
    page.getByRole("group", { name: "UD-Q2_K_XL" }).getByRole("button", { name: /^Baixando… \d+%$/ }),
  ).toBeVisible();
});
