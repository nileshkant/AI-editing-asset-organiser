import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';

const axe = async (page: Page) => expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
const overflow = (page: Page) => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);

for (const [width, height, zoom] of [[1280, 820, 1], [720, 560, 1], [1280, 820, 2]] as const) {
  test(`single and bulk annotations, history and tag rename by keyboard at ${width}×${height} ${zoom * 100}%`, async ({ page }, info) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installDesktopFixture(page, true); await page.goto('/');
    await page.locator('html').evaluate((el, factor) => { el.style.zoom = String(factor); }, zoom);
    await page.getByRole('button', { name: 'Images', exact: true }).click();
    const grid = page.getByRole('grid', { name: 'Images' });
    await grid.locator('[data-index="1"]').focus();
    await page.keyboard.press('Enter');
    const inspector = page.getByRole('complementary', { name: /Image details: image-000001\.png/ });
    await expect(inspector).toBeFocused();

    // Single edits: tags, rating, favorite and plain-text notes.
    const tagInput = inspector.getByLabel('Add tags to image-000001.png');
    await tagInput.focus();
    await page.keyboard.type('Brand A, Summer campaign');
    await page.keyboard.press('Enter');
    await expect(inspector.getByRole('list').filter({ hasText: 'Summer campaign' })).toBeVisible();
    await inspector.getByRole('radio', { name: '4 of 5 stars' }).check();
    await expect(inspector.getByRole('radio', { name: '4 of 5 stars' })).toBeChecked();
    await inspector.getByRole('button', { name: 'Favorite' }).click();
    await expect(inspector.getByRole('button', { name: 'Favorite' })).toHaveAttribute('aria-pressed', 'true');
    const xss = '<img src=x onerror="window.__pwned=1"> approved for episode 12';
    await inspector.getByLabel('Notes').fill(xss);
    await inspector.getByRole('button', { name: 'Save notes' }).click();
    await expect(inspector.getByRole('button', { name: 'Save notes' })).toBeDisabled();
    await expect(inspector.getByLabel('Notes')).toHaveValue(xss);
    expect(await page.evaluate(() => (window as { __pwned?: number }).__pwned)).toBeUndefined();
    expect(await inspector.locator('img[src="x"]').count()).toBe(0);
    await inspector.getByRole('button', { name: 'Remove tag thumbnail' }).click();
    await expect(inspector.getByRole('button', { name: 'Remove tag thumbnail' })).toHaveCount(0);
    expect(await overflow(page)).toBeLessThanOrEqual(1);
    await axe(page);
    await page.screenshot({ path: info.outputPath(`annotations-${width}-${zoom}.png`) });
    await inspector.getByRole('button', { name: 'Close image details' }).click();
    // Focus returns to the image that was open before moving on.
    await expect(grid.locator('[data-index="1"]')).toBeFocused();

    // Bulk edit two selected images, then undo it from history.
    await page.keyboard.press('ArrowRight');
    await expect(grid.locator('[data-index="2"]')).toBeFocused();
    await page.keyboard.press('Space');
    await page.keyboard.press('ArrowRight'); await page.keyboard.press('Space');
    await expect(page.getByText('2 selected')).toBeVisible();
    await expect(grid.locator('[data-index="3"]')).toHaveAttribute('aria-selected', 'true');
    await page.getByRole('button', { name: 'Edit selected…' }).click();
    const bulk = page.getByRole('dialog', { name: 'Edit 2 selected images' });
    await expect(bulk.getByLabel('Add tags')).toBeFocused();
    await page.keyboard.type('Client approved');
    await bulk.getByLabel('Rating').selectOption({ label: '5 of 5' });
    await bulk.getByRole('button', { name: 'Preview changes' }).click();
    await expect(bulk.getByText('2 images will change.')).toBeVisible();
    await expect(bulk.getByText('Add “Client approved”: 0 already have it')).toBeVisible();
    await axe(page);
    await bulk.getByRole('button', { name: 'Apply to 2 images' }).click();
    await expect(bulk.getByText('Updated 2 images')).toBeVisible();
    await page.screenshot({ path: info.outputPath(`bulk-${width}-${zoom}.png`) });
    await page.keyboard.press('Escape');
    await expect(bulk).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'Edit selected…' })).toBeFocused();

    await page.getByRole('button', { name: 'History…' }).click();
    const history = page.getByRole('dialog', { name: 'Annotation history' });
    await expect(history.getByRole('listitem')).toHaveCount(6);
    await axe(page);
    await history.getByRole('button', { name: /^Undo Add “Client approved”, rate 5\/5 · 2 images/ }).click();
    await expect(history.getByText(/Restored 2 images/)).toBeVisible();
    await expect(history.getByText('Undone', { exact: true })).toHaveCount(1);
    await page.keyboard.press('Escape');

    // Rename with an alias: 239 carry the tag (one removed above); 5 offline originals keep it.
    await page.getByRole('button', { name: 'Tags…' }).click();
    const tags = page.getByRole('dialog', { name: 'Image tags' });
    await tags.getByRole('button', { name: /^thumbnail/ }).click();
    await expect(tags.getByLabel('New name', { exact: true })).toBeFocused();
    await tags.getByLabel('New name', { exact: true }).fill('Thumbnail candidate');
    await tags.getByRole('button', { name: 'Preview rename' }).click();
    await expect(tags.getByText(/^234 images will change\. 5 offline or missing images keep “thumbnail”\.$/)).toBeVisible();
    await axe(page);
    await tags.getByRole('button', { name: 'Rename', exact: true }).click();
    await expect(tags.getByText(/Renamed on 234 images/)).toBeVisible();
    await expect(tags.getByText('“thumbnail” → “Thumbnail candidate”')).toBeVisible();
    expect(await overflow(page)).toBeLessThanOrEqual(1);
    await page.screenshot({ path: info.outputPath(`tags-${width}-${zoom}.png`) });
    await page.keyboard.press('Escape');
    await expect(page.getByRole('button', { name: 'Tags…' })).toBeFocused();
  });
}
