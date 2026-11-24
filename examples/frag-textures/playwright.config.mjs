import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  timeout: 120_000,
  use: { viewport: { width: 1280, height: 800 } },
  webServer: { command: "npx vite --port 5197 --strictPort", port: 5197, reuseExistingServer: false },
});
