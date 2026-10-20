import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';

const axe = async (page: Page) => expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
const overflow = (page: Page) => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);

for (const [width, height, zoom] of [[1280, 820, 1], [720, 560, 1], [1280, 820, 2]] as const) {
  test(`SS-062 exact-duplicate review by keyboard at ${width}×${height} ${zoom * 100}%`, async ({ page }, info) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installDesktopFixture(page, true); await page.goto('/');
    await page.locator('html').evaluate((el, factor) => { el.style.zoom = String(factor); }, zoom);
    await page.getByRole('button', { name: 'Images', exact: true }).click();
    const opener = page.getByRole('button', { name: 'Duplicates…' });
    await opener.focus();
    await page.keyboard.press('Enter');
    const dialog = page.getByRole('dialog', { name: 'Duplicate images' });
    await expect(dialog.getByText('1 stack · 2 extra copies')).toBeVisible();
    await expect(dialog.getByRole('heading', { level: 3 })).toContainText('Exact copies');
    await expect(dialog.getByText('Offline')).toBeVisible();
    expect(await overflow(page)).toBeLessThanOrEqual(1);
    await axe(page);
    await page.screenshot({ path: info.outputPath(`duplicates-${width}-${zoom}.png`) });

    // Decisions by keyboard; each is sent from the revision last read.
    await dialog.getByRole('button', { name: /^Prefer image-000002\.png/ }).focus();
    await page.keyboard.press('Enter');
    await expect(dialog.getByText('Preferred', { exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: /^Keep image-000001\.png .* separate$/ }).click();
    await expect(dialog.getByText('Kept separate', { exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: 'Dismiss' }).click();
    await expect(dialog.getByRole('button', { name: 'Show again' })).toBeVisible();
    await dialog.getByRole('checkbox', { name: /Show dismissed and resolved/ }).check();
    await dialog.getByRole('button', { name: 'Show again' }).click();
    await expect(dialog.getByRole('button', { name: 'Dismiss' })).toBeVisible();
    const sent = await page.evaluate(() => (window as any).__fixtureCalls.filter((c: any) => c.command === 'update_image_duplicate_group').map((c: any) => [c.args.expectedRevision, c.args.action.action]));
    expect(sent).toEqual([[0, 'prefer'], [1, 'separate'], [2, 'dismiss'], [3, 'restore']]);
    // Nothing destructive is ever requested.
    const commands = await page.evaluate(() => (window as any).__fixtureCalls.map((c: any) => c.command));
    expect(commands.some((c: string) => /delete|remove_source|merge/.test(c))).toBe(false);

    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();
    await expect(opener).toBeFocused();
  });
}
