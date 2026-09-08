import { test, expect } from "@playwright/test";

// A barra é o único lugar do app onde sete medidores dividem uma linha só.
// O que quebrou antes foi layout, não lógica: rótulos por extenso empurravam
// o número para uma segunda linha. Um teste de altura pega isso; um de texto,
// não.
test("a barra cabe numa linha em qualquer largura", async ({ page }) => {
  await page.goto("/tests/statusbar.html");
  for (const largura of [1920, 1440, 1100, 900]) {
    const barra = page.locator(`[data-width="${largura}"] footer`);
    await expect(barra).toBeVisible();
    const caixa = await barra.boundingBox();
    // `h-9` são 36px. Passar disso significa que algo quebrou em duas linhas.
    expect(
      caixa!.height,
      `a barra cresceu em ${largura}px`,
    ).toBeLessThanOrEqual(37);
  }
});

test("o nome de cada medidor vive no pictograma, não no texto da barra", async ({
  page,
}) => {
  await page.goto("/tests/statusbar.html");
  const barra = page.locator('[data-width="1920"] footer');

  // `innerText` é o que a tela de fato mostra; `textContent` e os seletores
  // de texto do Playwright ainda alcançam o `<title>` de um SVG, que não é
  // renderizado. A diferença entre os dois É a mudança: o rótulo saiu da
  // linha e virou o nome acessível do pictograma.
  const visivel = await barra.innerText();
  for (const rotulo of ["ENERGIA", "DISCO", "VRAM"]) {
    expect(visivel, `"${rotulo}" ainda ocupa espaço na barra`).not.toContain(
      rotulo,
    );
    await expect(
      barra.getByTitle(rotulo, { exact: true }).first(),
    ).toBeAttached();
  }

  // Os números continuam onde sempre estiveram: visíveis, sem passar o mouse.
  await expect(barra.getByText("46 W / 370 W")).toBeVisible();
  await expect(barra.getByText("26 GB / 64 GB")).toBeVisible();
});
