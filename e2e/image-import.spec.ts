import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';
for (const zoom of [1, 2]) {
  test(`media preview and recoverable results are accessible at ${zoom * 100}%`, async ({ page }, info) => {
    await page.setViewportSize({ width: 1280, height: 820 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installDesktopFixture(page, true); await page.goto('/');
    await page.locator('html').evaluate((el, factor) => { el.style.zoom = String(factor); }, zoom);
    await page.getByRole('button', { name: 'Imports', exact: true }).click();
    const choose = page.getByRole('button', { name: 'Choose media folders' }); await choose.click();
    const dialog = page.getByRole('dialog', { name: 'Review media import' }); await expect(dialog).toBeVisible();
    await expect(dialog).toContainText('Folder, including subfolders'); await expect(dialog).toContainText('2 images · 0 audio · 1 unsupported');
    const cancel = dialog.getByRole('button', { name: 'Cancel', exact: true }), confirm = dialog.getByRole('button', { name: 'Confirm import' });
    await expect(cancel).toBeFocused(); await page.keyboard.press('Shift+Tab'); await expect(confirm).toBeFocused(); await page.keyboard.press('Tab'); await expect(cancel).toBeFocused();
    const overflow = await dialog.evaluate(el => el.scrollWidth - el.clientWidth); expect(overflow).toBeLessThanOrEqual(1);
    expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
    await page.screenshot({ path: info.outputPath(`image-preview-${zoom}.png`) });
    await page.keyboard.press('Escape'); await expect(dialog).not.toBeVisible(); await expect(choose).toBeFocused();
    await choose.click(); await page.getByRole('button', { name: 'Confirm import' }).click();
    await page.getByRole('button', { name: 'Browse files' }).click(); await expect(page.getByRole('article').getByText('chosen.png', { exact: true })).toBeVisible(); await expect(page.getByRole('article').getByText('corrupt.png', { exact: true })).toBeVisible();
    expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
    await page.screenshot({ path: info.outputPath(`image-results-${zoom}.png`) });
    const calls = await page.evaluate(() => (window as any).__fixtureCalls as { command: string }[]);
    expect(calls.some(c => /upload|model_download|provider/.test(c.command))).toBe(false);
  });
}
