import { realpathSync } from "node:fs";
import path from "node:path";
import { defineConfig, devices } from "@playwright/test";

const isolatedRun = process.env.T71_RUN;
if (isolatedRun && (path.dirname(isolatedRun) !== "/data/knoted_dev/t71" ||
    !path.basename(isolatedRun).startsWith("instance.") || realpathSync(isolatedRun) !== isolatedRun)) {
  throw new Error("Run T71 through tests/fixtures/t71/instance.sh");
}

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: "list",
  use: { baseURL: "http://127.0.0.1:4177" },
  ...(isolatedRun ? { outputDir: `${isolatedRun}/playwright-results` } : {}),
  webServer: isolatedRun ? [] : [
    {
      command: "pnpm --filter @knoted/web exec vite --config ../../tests/e2e/fixtures/vite.config.mts",
      url: "http://127.0.0.1:4177/tests/e2e/fixtures/index.html",
      reuseExistingServer: false,
    },
    {
      command: "pnpm --filter @knoted/web dev --host 127.0.0.1 --port 4178 --strictPort",
      url: "http://127.0.0.1:4178/",
      reuseExistingServer: false,
    },
  ],
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
