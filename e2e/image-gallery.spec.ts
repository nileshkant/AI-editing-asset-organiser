import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { installDesktopFixture } from './desktop-fixture';

const axe = async (page: Page) => expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
const calls = (page: Page) => page.evaluate(() => (window as any).__fixtureCalls as { command: string; args: any }[]);
const openImages = async (page: Page) => {
  await page.getByRole('button', { name: 'Images', exact: true }).click();
  await expect(page.getByRole('grid', { name: 'Images' })).toBeVisible();
};

for (const [width, height, zoom] of [[1280, 820, 1], [720, 560, 1], [1280, 820, 2]] as const) {
  test(`gallery, inspector and comparison work by keyboard at ${width}×${height} ${zoom * 100}%`, async ({ page }, info) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installDesktopFixture(page, true); await page.goto('/');
    await page.locator('html').evaluate((el, factor) => { el.style.zoom = String(factor); }, zoom);
    await openImages(page);
    await expect(page.getByText('240 images')).toBeVisible();
    const grid = page.getByRole('grid', { name: 'Images' });
    const first = grid.getByRole('gridcell').first();
    await expect(first.locator('img')).toBeVisible();
    // Long Unicode names and badges never cause horizontal page overflow.
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await axe(page);
    await page.screenshot({ path: info.outputPath(`gallery-${width}-${zoom}.png`) });
    await first.focus();
    await page.keyboard.press('ArrowRight');
    await expect(grid.locator('[data-index="1"]')).toBeFocused();
    await page.keyboard.press('Space');
    await expect(grid.locator('[data-index="1"]')).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('ArrowRight'); await page.keyboard.press('Space');
    await expect(page.getByText('2 selected')).toBeVisible();
    await page.keyboard.press('Enter');
    const inspector = page.getByRole('complementary', { name: /Image details: image-000002\.png/ });
    await expect(inspector).toBeFocused();
    await expect(inspector).toContainText('Partial transparency · semi-transparent pixels');
    await expect(inspector).toContainText('stored 3000 × 4000, EXIF orientation 6');
    await inspector.getByRole('button', { name: '100%' }).click();
    await expect.poll(async () => (await calls(page)).some(c => c.command === 'image_preview' && c.args.tier === 'tile')).toBe(true);
    const tileCalls = (await calls(page)).filter(c => c.command === 'image_preview' && c.args.tier === 'tile').length;
    expect(tileCalls).toBeLessThanOrEqual(48);
    await inspector.getByRole('radio', { name: 'Dark' }).check();
    await expect(inspector.locator('.image-viewer')).toHaveClass(/bg-dark/);
    await axe(page);
    await page.screenshot({ path: info.outputPath(`inspector-${width}-${zoom}.png`) });
    await inspector.getByRole('region', { name: /preview/ }).focus();
    await page.keyboard.press('Escape');
    await expect(inspector).not.toBeVisible();
    await expect(grid.locator('[data-index="2"]')).toBeFocused();
    await page.getByRole('button', { name: 'Compare selected' }).click();
    const dialog = page.getByRole('dialog', { name: 'Compare 2 images' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('img')).toHaveCount(2);
    await axe(page);
    await page.screenshot({ path: info.outputPath(`compare-${width}-${zoom}.png`) });
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'Compare selected' })).toBeFocused();
  });
}

test('switching to images keeps the audio list, selection, details and playback', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await installDesktopFixture(page, true); await page.goto('/');
  const rows = page.locator('.sound-select');
  await rows.nth(2).click();
  await expect(page.locator('.inspector')).toBeVisible();
  await page.getByRole('button', { name: /^Play / }).first().click();
  const transport = page.locator('.transport');
  const before = await transport.innerText();
  await openImages(page);
  // Space on a grid cell is selection, not audio playback.
  await page.getByRole('grid', { name: 'Images' }).getByRole('gridcell').first().focus();
  await page.keyboard.press('Space');
  expect((await calls(page)).filter(c => c.command === 'playback_pause')).toHaveLength(0);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await expect(page.locator('.inspector')).toBeVisible();
  await expect(page.locator('.inspector h2')).toHaveText('Recording 3');
  expect(await transport.innerText()).toBe(before);
  // The audio search was not re-issued or reset by the image visit.
  const searches = (await calls(page)).filter(c => c.command === 'search_sounds');
  expect(searches.every(c => c.args.query.offset === 0 && c.args.query.favorites_only === false)).toBe(true);
});

test('a 100,000-image library stays windowed while scrolling', async ({ page }, info) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await installDesktopFixture(page, true, 100_000); await page.goto('/');
  await openImages(page);
  await expect(page.getByText('100,000 images')).toBeVisible();
  const scroller = page.locator('.image-grid-scroller');
  const frames = await scroller.evaluate(async el => {
    const deltas: number[] = [];
    let last = performance.now();
    for (let i = 1; i <= 40; ++i) {
      el.scrollTop = (el.scrollHeight - el.clientHeight) * (i / 40);
      await new Promise(r => requestAnimationFrame(r));
      const now = performance.now(); deltas.push(now - last); last = now;
    }
    return deltas;
  });
  await expect(page.getByRole('gridcell', { name: /image-099999\.png/ })).toBeVisible();
  const during = (await calls(page)).filter(c => c.command === 'image_preview').length;
  // Once scrolling settles, the visible thumbnails (and only those) load.
  await expect(page.getByRole('gridcell', { name: /image-099999\.png/ }).locator('img')).toBeVisible();
  const cells = await page.locator('.image-cell').count();
  const previews = (await calls(page)).filter(c => c.command === 'image_preview').length;
  const windows = (await calls(page)).filter(c => c.command === 'image_gallery_page').length;
  const heap = await page.evaluate(() => (performance as any).memory?.usedJSHeapSize ?? 0);
  frames.sort((a, b) => a - b);
  const p95 = frames[Math.floor(frames.length * 0.95)];
  info.annotations.push({ type: 'SS-045 scale', description: `cells ${cells}, preview requests ${previews}, windows ${windows}, heap ${(heap / 2 ** 20).toFixed(1)} MiB, frame p95 ${p95.toFixed(1)} ms` });
  console.log(`SS-045 scale: cells ${cells}, preview requests ${previews}, windows ${windows}, heap ${(heap / 2 ** 20).toFixed(1)} MiB, frame p95 ${p95.toFixed(1)} ms, max ${frames.at(-1)!.toFixed(1)} ms`);
  expect(during).toBe(0);
  expect(previews).toBeGreaterThan(0);
  expect(previews).toBeLessThanOrEqual(cells);
  expect(cells).toBeLessThanOrEqual(120);
  // Only rows that were actually shown requested windows/previews, never the library.
  expect(windows).toBeLessThan(200);
  expect(await page.locator('.image-cell img').count()).toBeLessThanOrEqual(120);
});
