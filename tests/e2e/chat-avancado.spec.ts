import { expect, test } from "@playwright/test";

// Amostragem avançada e "Copiar requisição", no painel de parâmetros do Chat.

test.beforeEach(async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  await page.getByRole("button", { name: "Parâmetros" }).click();
});

test("a requisição copiada é a que o chat mandaria, com os parâmetros avançados e sem a chave", async ({ page }) => {
  await page.getByText("Amostragem avançada").click();
  await page.getByRole("spinbutton", { name: "min_p" }).fill("0.05");
  await page.getByRole("spinbutton", { name: "Semente (seed)" }).fill("7");
  await page.getByRole("textbox", { name: "Parar em" }).fill("FIM");

  await page.getByRole("button", { name: "Python", exact: true }).click();
  await expect(page.getByText("Requisição copiada (Python).")).toBeVisible();
  const py = await page.evaluate(() => navigator.clipboard.readText());
  expect(py).toContain("seed=7,");
  expect(py).toContain('"min_p": 0.05,');
  expect(py).toContain('"FIM"');
  expect(py).toContain('api_key="local"');

  await page.getByRole("button", { name: "cURL", exact: true }).click();
  const curl = await page.evaluate(() => navigator.clipboard.readText());
  expect(curl).toContain("curl http://127.0.0.1:11711/v1/chat/completions");
  expect(curl).toContain('"min_p": 0.05');
  expect(curl).not.toContain('"stream": true');
});

test("os padrões do servidor tiram temperatura, top_p e top_k da requisição", async ({ page }) => {
  await page.getByText("Amostragem avançada").click();
  await page.getByRole("checkbox", { name: /Usar os padrões do servidor/ }).check();
  await page.getByRole("button", { name: "cURL", exact: true }).click();
  const curl = await page.evaluate(() => navigator.clipboard.readText());
  expect(curl).not.toContain("temperature");
  expect(curl).not.toContain("top_k");
  expect(curl).toContain('"model"');
});

test("um esquema JSON inválido avisa e não vai na requisição", async ({ page }) => {
  await page.getByText("Amostragem avançada").click();
  await page.getByRole("textbox", { name: /Esquema JSON da resposta/ }).fill("{ isto não é json");
  await expect(page.getByText("Isto não é um JSON válido")).toBeVisible();
  await page.getByRole("button", { name: "cURL", exact: true }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).not.toContain("response_format");
});

test("dá para digitar mais de uma sequência de parada, uma por linha", async ({ page }) => {
  await page.getByText("Amostragem avançada").click();
  const campo = page.getByRole("textbox", { name: "Parar em" });
  await campo.click();
  await page.keyboard.type("FIM");
  await page.keyboard.press("Enter");
  await page.keyboard.type("###");
  await expect(campo).toHaveValue("FIM\n###");
  await page.getByRole("button", { name: "cURL", exact: true }).click();
  const curl = await page.evaluate(() => navigator.clipboard.readText());
  expect(curl).toMatch(/"stop": \[\s*"FIM",\s*"###"\s*\]/);
});
