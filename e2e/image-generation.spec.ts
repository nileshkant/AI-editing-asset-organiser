import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';

const axe = async (page: Page) => expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
const overflow = (page: Page) => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);

for (const [width, height, zoom] of [[1280, 820, 1], [720, 560, 1], [1280, 820, 2]] as const) {
  test(`generation provenance is attributed, editable by keyboard and private by default at ${width}×${height} ${zoom * 100}%`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installDesktopFixture(page, true); await page.goto('/');
    await page.locator('html').evaluate((el, factor) => { el.style.zoom = String(factor); }, zoom);
    await page.getByRole('button', { name: 'Images', exact: true }).click();
    await page.getByRole('grid', { name: 'Images' }).locator('[data-index="1"]').focus();
    await page.keyboard.press('Enter');
    const inspector = page.getByRole('complementary', { name: /Image details: image-000001\.png/ });
    const section = inspector.getByRole('region', { name: 'Generation provenance' });
    await expect(section.getByText('Generation metadata found in this file.')).toBeVisible();
    // Embedded text is shown as text, never as markup; the huge seed is kept exactly.
    await expect(section.getByText(/<b>not markup<\/b>/)).toBeVisible();
    await expect(section.locator('b')).toHaveCount(0);
    await expect(section.getByText('18446744073709551617')).toBeVisible();
    await expect(section.getByText(/Claimed by A1111-style parameters/).first()).toBeVisible();
    await expect(section.getByRole('link')).toHaveCount(0);
    expect(await overflow(page)).toBeLessThanOrEqual(1);

    await section.getByRole('button', { name: 'Edit provenance' }).focus();
    await page.keyboard.press('Enter');
    await section.getByLabel('Generator').focus();
    await page.keyboard.type('My pipeline');
    await section.getByLabel('Seed').fill('42');
    await section.getByRole('button', { name: 'Save provenance' }).focus();
    await page.keyboard.press('Enter');
    await expect(section.getByText('Saved provenance.')).toBeVisible();
    await expect(section.getByText('My pipeline')).toBeVisible();
    await expect(section.getByText('Your entry')).toHaveCount(2);
    await section.getByRole('button', { name: 'Mark imported values reviewed' }).click();
    await expect(section.getByText('You reviewed these imported values.')).toBeVisible();
    await axe(page);

    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('tab', { name: 'Catalog data' }).click();
    const options = page.getByRole('group', { name: 'Image generation metadata' });
    await expect(options.getByLabel('Include prompts in this JSON export')).not.toBeChecked();
    await expect(options.getByLabel('Include private references in this JSON export')).not.toBeChecked();
    await options.getByLabel('Search generation metadata').selectOption('prompts');
    await expect(page.getByText('Generation metadata settings saved.')).toBeVisible();
    expect(await overflow(page)).toBeLessThanOrEqual(1);
    await axe(page);
  });
}
