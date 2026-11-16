import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: "list",
  use: { baseURL: "http://127.0.0.1:4177" },
  webServer: {
    command: "pnpm --filter @knoted/web exec vite --config ../../tests/e2e/fixtures/vite.config.mts",
    url: "http://127.0.0.1:4177/tests/e2e/fixtures/index.html",
    reuseExistingServer: false,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
