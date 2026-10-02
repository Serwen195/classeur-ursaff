import { defineConfig } from '@playwright/test';

// Les tests e2e pilotent l'interface contre le faux backend en mémoire (`src/lib/mock.ts`).
// Pour utiliser un Chromium déjà installé : PW_CHROMIUM=/chemin/vers/chromium npm run e2e
export default defineConfig({
  testDir: 'e2e',
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: true,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: 'http://127.0.0.1:1420',
    viewport: { width: 1180, height: 820 },
    locale: 'fr-FR',
    launchOptions: process.env.PW_CHROMIUM ? { executablePath: process.env.PW_CHROMIUM } : {},
  },
  webServer: {
    command: 'npm run dev',
    url: 'http://127.0.0.1:1420',
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
