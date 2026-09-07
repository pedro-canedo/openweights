import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
const runtime = process.env.OW_TEST_RUNTIME;
const model = process.env.OW_TEST_MODEL;
const repetitions = Number(process.argv[3] ?? 3);
if (!Number.isInteger(repetitions) || repetitions < 3 || repetitions % 3 !== 0) throw new Error('Repetitions must be a positive multiple of 3');
if (!runtime || !model) throw new Error('Set OW_TEST_RUNTIME (llama-server executable) and OW_TEST_MODEL');
const child = spawn(runtime, ['-m', model, '--alias', 'test', '--host', '127.0.0.1', '--port', '14820', '-c', '8192', '-ngl', '99', '-fa', 'on'], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
let log = '';
child.stderr.on('data', b => { log = (log + b).slice(-6000); });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 120; i++) {
    if (child.exitCode != null) throw new Error(log);
    if (await fetch('http://127.0.0.1:14820/health').then(r => r.ok).catch(() => false)) { ready = true; break; }
    await new Promise(r => setTimeout(r, 1000));
  }
  if (!ready) throw new Error(log);
  browser = await chromium.launch({channel: process.platform === 'win32' ? 'msedge' : undefined, headless:true});
  const page = await browser.newPage();
  await page.goto('http://localhost:1420/tests/inference.html');
  await page.waitForFunction(() => !!window.measureInference);
  await page.evaluate(() => window.measureInference('previous'));
  await page.evaluate(() => window.measureInference('current'));
  const runs = [];
  const order = Array.from({length: repetitions / 3}, () => ['previous','current','current','previous','previous','current']).flat();
  for (const which of order) {
    const measured = await page.evaluate(which => window.measureInference(which), which);
    runs.push({which,...measured}); console.log(runs.at(-1));
  }
  await writeFile(process.argv[2] ?? 'inference-bench.json', JSON.stringify({runtime,model,runs,measuredAt:new Date().toISOString()},null,2));
} finally { await browser?.close(); child.kill(); }
