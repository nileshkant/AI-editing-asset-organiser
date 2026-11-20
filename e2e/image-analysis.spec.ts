import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';

const axe = async (page: Page) => expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
const overflow = (page: Page) => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);

for (const [width, height, zoom] of [[1280, 820, 1], [720, 560, 1], [1280, 820, 2]] as const) {
  test(`image analysis needs an explicit allow, an estimate and ceilings, by keyboard at ${width}×${height} ${zoom * 100}%`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.addInitScript(() => { (window as any).__readyProvider = true; });
    await installDesktopFixture(page, true); await page.goto('/');
    await page.locator('html').evaluate((el, factor) => { el.style.zoom = String(factor); }, zoom);
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('tab', { name: 'AI providers' }).click();
    const section = page.getByRole('region', { name: 'Image analysis' });
    await expect(section.getByText('file names or folder paths')).toBeVisible();
    await expect(section.getByRole('button', { name: 'Allow analysis' })).toBeDisabled();
    await expect(section.getByRole('button', { name: /Estimate analysis/ })).toHaveCount(0);
    expect(await overflow(page)).toBeLessThanOrEqual(1);
    await axe(page);

    await section.getByLabel(/I understand copies of these images will be sent/).focus();
    await page.keyboard.press('Space');
    await section.getByRole('button', { name: 'Allow analysis' }).focus();
    await page.keyboard.press('Enter');
    await expect(section.getByText(/Allowed for .*: provider Local vision server/)).toBeVisible();
    await section.getByRole('button', { name: /Estimate analysis of/ }).focus();
    await page.keyboard.press('Enter');
    await expect(section.getByText(/3 images would be sent/)).toBeVisible();
    await expect(section.getByLabel('Most requests to send')).toHaveValue('3');
    expect(await overflow(page)).toBeLessThanOrEqual(1);
    await axe(page);
    await section.getByRole('button', { name: 'Start analysis' }).focus();
    await page.keyboard.press('Enter');
    const calls = await page.evaluate(() => (window as any).__fixtureCalls.map((c: any) => c.command));
    expect(calls.indexOf('image_enrichment_grant')).toBeLessThan(calls.indexOf('image_enrichment_estimate'));
    expect(calls.indexOf('image_enrichment_estimate')).toBeLessThan(calls.indexOf('image_enrichment_start'));
  });
}
