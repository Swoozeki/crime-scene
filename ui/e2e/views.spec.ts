import { test, expect, type Page } from '@playwright/test';

// Every view renders real data without errors, in light and dark mode.
const views = ['overview', 'hotspots', 'map', 'architecture', 'coupling', 'knowledge', 'diff'];

async function open(page: Page, hash: string, errors: string[]) {
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(`/#/${hash}`);
  await page.waitForLoadState('networkidle');
  await expect(page.locator('main h1').first()).toBeVisible();
  await expect(page.locator('main .error')).toHaveCount(0);
}

for (const theme of ['Light', 'Dark']) {
  for (const view of views) {
    test(`${view} (${theme.toLowerCase()})`, async ({ page }) => {
      const errors: string[] = [];
      await open(page, view, errors);
      await page.getByRole('button', { name: theme, exact: true }).click();
      await page.waitForLoadState('networkidle');
      await expect(page.locator('main h1').first()).toBeVisible();
      await expect(page.locator('main .error')).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }
}

test('hotspots link through to an entity page', async ({ page }) => {
  const errors: string[] = [];
  await open(page, 'hotspots', errors);
  await expect(page.locator('main')).toContainText('cart.component');
  await page.locator('main').getByText('cart.component').first().click();
  await expect(page).toHaveURL(/#\/entity\//);
  await expect(page.locator('main h1').first()).toContainText('cart');
  await expect(page.locator('main .error')).toHaveCount(0);
  expect(errors).toEqual([]);
});
