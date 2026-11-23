import { defineConfig, devices } from '@playwright/test';

// Each spec starts its own server (see e2e/server.ts) on a fresh data dir with the fixture card DB.
// Run `npm run build` first.
export default defineConfig({
  testDir: 'e2e',
  timeout: 120_000,
  expect: { timeout: 10_000 },
  workers: 1,
  fullyParallel: false,
  retries: 0,
  reporter: [['list']],
  use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 }, actionTimeout: 15_000, trace: 'retain-on-failure' },
});
