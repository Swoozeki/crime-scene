import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'e2e',
  timeout: 30_000,
  use: { baseURL: 'http://127.0.0.1:7788' },
  webServer: {
    command: 'bash e2e/serve.sh',
    url: 'http://127.0.0.1:7788/api/overview',
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
  },
});
