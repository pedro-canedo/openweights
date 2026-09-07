import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests/e2e",
  use: { baseURL: "http://localhost:1420", channel: process.env.PLAYWRIGHT_CHANNEL ?? (process.platform === "win32" ? "msedge" : undefined), viewport: { width: 1280, height: 800 } },
  webServer: { command: "npm run dev", url: "http://localhost:1420", reuseExistingServer: !process.env.CI },
});
