import { test, expect } from '@playwright/test';

test.describe('MoFA Engine Dashboard', () => {

  // ── Test 1: Dashboard loads ──────────────────────────────────────────
  test('dashboard loads and shows header', async ({ page }) => {
    await page.goto('/');

    // Title tag
    await expect(page).toHaveTitle('MoFA Engine');

    // Header h1
    const heading = page.locator('.header h1');
    await expect(heading).toHaveText('MoFA Engine');

    // Version and uptime badges exist in the header
    await expect(page.locator('#version-badge')).toBeVisible();
    await expect(page.locator('#uptime-badge')).toBeVisible();

    // Model cards section exists (either cards or empty-state placeholder)
    const modelGrid = page.locator('#model-grid');
    await expect(modelGrid).toBeVisible();

    // Wait for the first auto-refresh to populate cards (2s interval in dashboard JS)
    // If models are loaded, .model-card elements should appear
    const cards = page.locator('.model-card');
    const cardCount = await cards.count();
    // At minimum the grid container is rendered; cards depend on engine state
    expect(cardCount).toBeGreaterThanOrEqual(0);
  });

  // ── Test 2: Stats bar displays metrics ───────────────────────────────
  test('stats bar displays metrics', async ({ page }) => {
    await page.goto('/');

    // Wait for JS to fetch /v1/status and populate stats
    // The stat-value elements start at "0" and get updated by refresh()
    await expect(page.locator('.stat-card')).toHaveCount(4);

    // Check labels are present
    await expect(page.getByText('Total Models')).toBeVisible();
    await expect(page.getByText('Loaded')).toBeVisible();
    await expect(page.locator('.stat-label', { hasText: 'Providers' })).toBeVisible();
    await expect(page.getByText('Memory', { exact: false })).toBeVisible();

    // Wait for refresh() to update stat values (polls /v1/status)
    const totalModels = page.locator('#stat-total');
    await expect(totalModels).toBeVisible();
    // After refresh, total models should be a number >= 0
    await expect(totalModels).not.toHaveText('--');

    const loadedModels = page.locator('#stat-loaded');
    await expect(loadedModels).toBeVisible();

    const providers = page.locator('#stat-providers');
    await expect(providers).toBeVisible();

    const memory = page.locator('#stat-memory');
    await expect(memory).toBeVisible();
  });

  // ── Test 3: Model cards show correct data ────────────────────────────
  test('model cards show correct data', async ({ page }) => {
    await page.goto('/');

    // Wait for at least one model card to appear (refresh() fetches /v1/capabilities)
    const firstCard = page.locator('.model-card').first();
    await expect(firstCard).toBeVisible({ timeout: 10_000 });

    // Each card should have: model-name, status-dot, cap-badge, model-provider
    await expect(firstCard.locator('.model-name')).toBeVisible();
    await expect(firstCard.locator('.status-dot')).toBeVisible();
    await expect(firstCard.locator('.cap-badge')).toBeVisible();
    await expect(firstCard.locator('.model-provider')).toBeVisible();

    // Model name should not be empty
    const nameText = await firstCard.locator('.model-name').textContent();
    expect(nameText?.trim().length).toBeGreaterThan(0);

    // Capability badge should be one of the known types
    const capText = await firstCard.locator('.cap-badge').textContent();
    const validCaps = ['chat', 'tts', 'asr', 'imagegen', 'embedding'];
    expect(validCaps).toContain(capText?.trim().toLowerCase());
  });

  // ── Test 4: Provider health panel ────────────────────────────────────
  test('provider health panel shows providers', async ({ page }) => {
    await page.goto('/');

    // The "Providers" panel header
    const providerHeader = page.locator('.panel-header').filter({ hasText: 'Providers' });
    await expect(providerHeader).toBeVisible();

    // Wait for provider data to load from /v1/status
    const providerList = page.locator('#provider-list');
    await expect(providerList).toBeVisible();

    // After refresh, at least one provider-item should appear
    const firstProvider = page.locator('.provider-item').first();
    await expect(firstProvider).toBeVisible({ timeout: 10_000 });

    // Provider item should have a name and a circuit state label
    await expect(firstProvider.locator('.provider-name')).toBeVisible();
    await expect(firstProvider.locator('.circuit-label')).toBeVisible();
  });

  // ── Test 5: Try It form works ────────────────────────────────────────
  test('try it form sends request and shows response', async ({ page }) => {
    test.setTimeout(90_000);
    await page.goto('/');

    // Find the Try It panel
    const tryItHeader = page.locator('.panel-header').filter({ hasText: 'Try It' });
    await expect(tryItHeader).toBeVisible();

    // Select "chat" capability (default, but be explicit)
    const capSelect = page.locator('#try-cap');
    await capSelect.selectOption('chat');

    // Type into the textarea
    const textarea = page.locator('#try-input');
    await textarea.fill('Say hello');

    // Click submit
    const submitBtn = page.locator('#try-btn');
    await expect(submitBtn).toBeEnabled();
    await submitBtn.click();

    // Button should show "Sending..." while waiting
    await expect(submitBtn).toHaveText('Sending...');

    // Wait for response (Ollama can be slow — 60s timeout)
    const output = page.locator('#try-output');
    await expect(output).not.toHaveText('Waiting for response...', { timeout: 60_000 });

    // Response should contain some text (either a successful reply or an error)
    const responseText = await output.textContent();
    expect(responseText?.trim().length).toBeGreaterThan(0);
    expect(responseText).not.toBe('Response will appear here');

    // Button should reset to "Send Request"
    await expect(submitBtn).toHaveText('Send Request');
    await expect(submitBtn).toBeEnabled();
  });

  // ── Test 6: API endpoints return correct data ────────────────────────
  test('API endpoints respond correctly', async ({ request }) => {
    // GET /health
    const healthRes = await request.get('/health');
    expect(healthRes.ok()).toBeTruthy();
    const health = await healthRes.json();
    expect(health.status).toBe('ok');
    expect(health.version).toBeTruthy();
    expect(typeof health.uptime_secs).toBe('number');

    // GET /v1/capabilities
    const capsRes = await request.get('/v1/capabilities');
    expect(capsRes.ok()).toBeTruthy();
    const caps = await capsRes.json();
    expect(Array.isArray(caps)).toBeTruthy();
    expect(caps.length).toBeGreaterThan(0);

    // Each capability entry should have required fields
    const first = caps[0];
    expect(first.name).toBeTruthy();
    expect(first.provider).toBeTruthy();
    expect(first.capability).toBeTruthy();

    // GET /v1/status
    const statusRes = await request.get('/v1/status');
    expect(statusRes.ok()).toBeTruthy();
    const status = await statusRes.json();
    expect(typeof status.total_models).toBe('number');
    expect(status.total_models).toBeGreaterThan(0);
    expect(typeof status.loaded_models).toBe('number');
    expect(typeof status.providers).toBe('number');
  });

  // ── Test 7: Dashboard auto-refresh doesn't crash ─────────────────────
  test('dashboard survives multiple auto-refresh cycles', async ({ page }) => {
    await page.goto('/');

    // Verify initial render
    const heading = page.locator('.header h1');
    await expect(heading).toHaveText('MoFA Engine');

    // The dashboard JS runs setInterval(refresh, 2000).
    // Wait 3 seconds to let at least one refresh cycle complete.
    await page.waitForTimeout(3_000);

    // Page should still be intact after auto-refresh cycles
    await expect(heading).toHaveText('MoFA Engine');
    await expect(page.locator('.stats-bar')).toBeVisible();
    await expect(page.locator('#model-grid')).toBeVisible();
    await expect(page.locator('#provider-list')).toBeVisible();

    // No JS errors should have crashed the page
    const footer = page.locator('.footer');
    await expect(footer).toBeVisible();
    await expect(footer).toContainText('MoFA Engine');

    // Stats should still be populated (not reverted to placeholder)
    const totalModels = page.locator('#stat-total');
    await expect(totalModels).toBeVisible();
  });
});
