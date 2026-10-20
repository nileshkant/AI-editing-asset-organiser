import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';

async function expectAccessible(page: import('@playwright/test').Page) {
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  expect(results.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(node => node.target) }))).toEqual([]);
}

test.beforeEach(async ({ page }) => {
  await installDesktopFixture(page);
  await page.goto('/');
  await expect(page.getByRole('button', { name: /Cinematic whoosh.*duration/ })).toBeVisible();
});

for (const viewport of [{ width: 1280, height: 820 }, { width: 720, height: 560 }]) {
  test(`library, inspector and settings fit ${viewport.width}×${viewport.height}`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport);
    await expectAccessible(page);
    await page.screenshot({ path: testInfo.outputPath(`library-${viewport.width}.png`) });
    await page.getByRole('button', { name: /Cinematic whoosh.*duration/ }).click();
    await expect(page.getByRole('heading', { level: 2, name: /Cinematic whoosh/ })).toBeVisible();
    await page.getByRole('button', { name: 'Save changes' }).scrollIntoViewIfNeeded();
    await expect(page.getByRole('button', { name: 'Save changes' })).toBeInViewport();
    await expectAccessible(page);
    await page.locator('.inspector').evaluate(element => { element.scrollTop = 0; });
    await page.screenshot({ path: testInfo.outputPath(`inspector-${viewport.width}.png`) });
    await page.getByRole('button', { name: 'Close details' }).click();
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    for (const category of ['Sources', 'Catalog data', 'Audio & recovery', 'Agent access', 'AI providers', 'Application', 'MCP guide']) {
      await page.getByRole('tab', { name: category, exact: true }).click();
      await expect(page.getByRole('tabpanel')).toBeVisible();
      await expectAccessible(page);
      const overflow = await page.getByRole('tabpanel').evaluate(element => element.scrollWidth - element.clientWidth);
      expect(overflow).toBeLessThanOrEqual(1);
    }
    await page.screenshot({ path: testInfo.outputPath(`settings-${viewport.width}.png`) });
    expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  });
}

test('keyboard completes navigation, metadata, filters, transport and relink cancellation', async ({ page }) => {
  const title = page.getByRole('button', { name: /Cinematic whoosh.*duration/ });
  await title.focus();
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('button', { name: 'Recording 2, duration 0:10.00' })).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.getByRole('button', { name: 'Recording 75, duration 0:10.00' })).toBeFocused();
  await expect(page.getByRole('button', { name: 'Recording 75, duration 0:10.00' })).toBeInViewport();
  await page.keyboard.press('Home');
  await page.keyboard.press('Enter');
  await page.getByRole('textbox', { name: 'Comment', exact: true }).fill('Keyboard edited comment');
  await page.getByRole('button', { name: 'Save changes' }).focus();
  await page.keyboard.press('Space');
  await expect(page.getByRole('button', { name: 'Saved', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Close details' }).focus();
  await page.keyboard.press('Enter');
  await expect(title).toBeFocused();
  await page.getByRole('button', { name: 'Filters', exact: true }).focus();
  await page.keyboard.press('Space');
  await expect(page.getByRole('region', { name: 'Search filters' })).toBeVisible();
  await page.getByRole('button', { name: 'Play', exact: true }).focus();
  await page.keyboard.press('Space');
  await expect(page.getByRole('button', { name: 'Pause', exact: true })).toBeVisible();
  await page.keyboard.press('Space');
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).focus();
  await page.keyboard.press('Enter');
  await page.getByRole('tab', { name: 'Sources', exact: true }).focus();
  await page.keyboard.press('End');
  await expect(page.getByRole('tab', { name: 'MCP guide', exact: true })).toBeFocused();
  await page.keyboard.press('Home');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('tabpanel')).toBeFocused();
  const relink = page.getByRole('button', { name: 'Relink folder', exact: true });
  await relink.focus(); await page.keyboard.press('Enter');
  await expect(page.getByRole('textbox', { name: 'Folder path' })).toBeFocused();
  await expectAccessible(page);
  await page.keyboard.press('Shift+Tab');
  await expect(page.getByRole('button', { name: 'Verify & relink' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('textbox', { name: 'Folder path' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(relink).toBeFocused();
  expect(await page.evaluate(() => (window as any).__fixtureCalls.filter((call: any) => call.command === 'relink_source'))).toEqual([]);
});


test('decimal clip boundaries survive real keyboard input', async ({ page }) => {
  await page.getByRole('button', { name: /Cinematic whoosh.*duration/ }).click();
  await page.getByRole('button', { name: 'Select All', exact: true }).click();
  const start = page.getByRole('spinbutton', { name: 'Selection start in seconds' });
  await start.fill('');
  await start.pressSequentially('0.5');
  await start.press('Tab');
  await expect(start).toHaveValue('0.500');
  const duration = page.getByRole('spinbutton', { name: 'Selection duration in seconds' });
  await duration.fill('');
  await duration.pressSequentially('1.25');
  await duration.press('Tab');
  await expect(duration).toHaveValue('1.250');
  await expect(page.getByText('Frames: 24000 – 84000 (exclusive)', { exact: true })).toBeVisible();
});


test('Tab reaches details and native source confirmation keeps background inert', async ({ page }) => {
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to workspace' })).toBeFocused();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Add files', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('textbox', { name: 'Search audio' })).toBeFocused();
  await page.keyboard.press('Tab');
  await page.keyboard.press('Tab');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary', { name: /Cinematic whoosh/ })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Close details' })).toBeFocused();
  await page.keyboard.press('Enter');
  await page.keyboard.press('/');
  await expect(page.getByRole('textbox', { name: 'Search audio' })).toBeFocused();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const remove = page.getByRole('button', { name: 'Remove source', exact: true });
  await remove.focus(); await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  expect(await page.getByRole('dialog').evaluate(dialog => dialog.matches(':modal'))).toBe(true);
  await page.getByRole('button', { name: 'Settings', exact: true }).evaluate(button => button.focus());
  await expect(page.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  await expectAccessible(page);
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Confirm', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(remove).toBeFocused();
});
