import { chromium } from "@playwright/test";
import { writeFile } from "node:fs/promises";
const browser = await chromium.launch({ channel: process.platform === "win32" ? "msedge" : undefined, headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
await page.goto("http://localhost:1420/tests/render.html");
await page.waitForFunction(() => !!window.startRenderBench);
await page.waitForTimeout(1000);
const runs = [];
for (let i = 0; i < 3; i++) {
  await page.evaluate((batch) => { window.benchDone = false; window.startRenderBench(batch); }, Number(process.argv[3] ?? 1));
  await page.waitForFunction(() => window.benchDone, null, { timeout: 60000 });
  runs.push(await page.evaluate(() => ({ cpuMs: window.renderSamples.reduce((a, b) => a + b, 0), commits: window.renderSamples.length, domNodes: document.querySelectorAll("*").length })));
}
await writeFile(process.argv[2] ?? "render-bench.json", JSON.stringify({ measuredAt: new Date().toISOString(), runs }, null, 2));
console.log(runs);
await browser.close();
