import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';
const axe = (page: import('@playwright/test').Page, scope: string) =>
  new AxeBuilder({ page }).include(scope).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
for (const zoom of [1, 2]) {
  test(`SS-047 source status, opt-in watching, relink and moves at ${zoom * 100}%`, async ({ page }, info) => {
    await page.setViewportSize({ width: 1024, height: 720 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installDesktopFixture(page, true); await page.goto('/');
    await page.locator('html').evaluate((el, factor) => { el.style.zoom = String(factor); }, zoom);
    await page.getByRole('button', { name: 'Imports', exact: true }).click();
    const panel = page.getByRole('region', { name: 'Image sources' });
    const card = panel.getByRole('article', { name: /^Image source / });
    await expect(card).toContainText('Offline · reconnect the drive');
    await expect(card).toContainText('3 images · 1 missing · 0 access denied · 1 could not be read');
    await expect(card.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(await panel.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    expect((await axe(page, '[aria-label="Image sources"]')).violations).toEqual([]);
    await page.screenshot({ path: info.outputPath(`image-sources-offline-${zoom}.png`) });

    await card.getByRole('button', { name: 'Check availability' }).click();
    await expect(card).toContainText('Available');
    const watch = card.getByRole('checkbox', { name: 'Watch for changes' });
    await expect(watch).not.toBeChecked();
    await watch.focus(); await page.keyboard.press('Space');
    await expect(watch).toBeChecked();
    await expect(card).toContainText('Watching. Changes are checked by size and date');
    await card.getByRole('button', { name: 'Verify file bytes' }).click();
    await expect(page.getByRole('article', { name: /import refresh-job/ })).toContainText('Byte verification · images and audio');

    // Explicit reconciliation, never automatic.
    await card.getByText('Possible moved files (1)').click();
    const reattach = card.getByRole('button', { name: /^Reattach shoot\/before-move\.png/ });
    await expect(reattach).toBeVisible();
    expect(await card.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    await reattach.click();
    await expect(card).not.toContainText('Possible moved files');

    // Relink: native picker only, disclosed counts, consent, focus restored.
    const relink = card.getByRole('button', { name: 'Relink…' });
    await relink.click();
    const dialog = page.getByRole('dialog', { name: /^Relink / });
    await expect(dialog.getByRole('textbox')).toHaveCount(0);
    await expect(dialog.getByRole('button', { name: 'Choose folder' })).toBeFocused();
    await dialog.getByRole('button', { name: 'Choose folder' }).click();
    await expect(dialog).toContainText('2 of 3 cataloged files found · 1 not found');
    await expect(dialog).toContainText('Sample check: 1 of 2 identical · 1 changed · 0 unreadable');
    const confirm = dialog.getByRole('button', { name: 'Relink and verify' });
    await expect(confirm).toBeDisabled();
    expect(await dialog.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    expect((await axe(page, 'dialog')).violations).toEqual([]);
    await page.screenshot({ path: info.outputPath(`image-relink-${zoom}.png`) });
    await dialog.getByRole('checkbox').check();
    await confirm.click();
    await expect(dialog).not.toBeVisible();
    await expect(relink).toBeFocused();
    await expect(page.getByRole('article', { name: /import relink-job/ })).toContainText('Relink verification · images and audio');

    const calls = await page.evaluate(() => (window as any).__fixtureCalls.filter((c: any) => /image_source|image_relink|image_move/.test(c.command)).map((c: any) => [c.command, c.args]));
    expect(calls).toEqual(expect.arrayContaining([
      ['set_image_source_watch', { id: 'source', enabled: true }],
      ['refresh_image_source', { id: 'source', scan: 'listed', verify: true }],
      ['reconcile_image_move', { missingId: 'image-old', presentId: 'image-new', missingRevision: 3, presentRevision: 1 }],
      ['choose_image_relink', { id: 'source' }],
      ['confirm_image_relink', { previewId: 'relink-plan', acknowledged: true }],
    ]));
    expect(calls.some(([command]: [string]) => command === 'relink_source')).toBe(false);
  });
}
