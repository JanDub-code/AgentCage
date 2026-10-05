#!/usr/bin/env node
// Actual Chromium DOM, keyboard, clipboard and reflow regression checks.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import path from 'node:path';

const require = createRequire(import.meta.url);
const { chromium } = require(require.resolve('playwright', { paths: [process.env.AC_WEB_NODE_MODULES || '.', ...require.resolve.paths('playwright')] }));
const root = path.resolve(process.argv[2] || path.join(path.dirname(fileURLToPath(import.meta.url)), '..'));
const source = await readFile(path.join(root, 'docs/index.html'));
const server = createServer((request, response) => {
    const headers = { 'content-type': 'text/html; charset=utf-8' };
    if (request.url === '/clipboard-denied') headers['permissions-policy'] = 'clipboard-write=()';
    response.writeHead(200, headers);
    response.end(source);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, ...(process.env.AC_BROWSER_EXECUTABLE ? { executablePath: process.env.AC_BROWSER_EXECUTABLE } : {}) });
const results = [];

async function checkBrowserZoom() {
    // A new profile and a test-only extension change Chromium's actual tab zoom.
    // CSS zoom and CDP page-scale emulation do not verify this browser behavior.
    const temporary = await mkdtemp(path.join(tmpdir(), 'agentcage-browser-zoom-'));
    const extension = path.join(temporary, 'extension');
    let context;
    try {
        await mkdir(extension);
        await writeFile(path.join(extension, 'manifest.json'), JSON.stringify({ manifest_version: 3,
            name: 'AgentCage isolated zoom regression', version: '1.0', permissions: ['tabs'],
            background: { service_worker: 'background.js' } }));
        await writeFile(path.join(extension, 'background.js'), 'chrome.runtime.onInstalled.addListener(() => {});');
        context = await chromium.launchPersistentContext(path.join(temporary, 'profile'), {
            channel: 'chromium', headless: true, viewport: null,
            ...(process.env.AC_BROWSER_EXECUTABLE ? { executablePath: process.env.AC_BROWSER_EXECUTABLE } : {}),
            args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`, '--window-size=1280,1000']
        });
        const worker = context.serviceWorkers()[0] || await context.waitForEvent('serviceworker', { timeout: 10000 });
        const page = await context.newPage();
        const errors = [];
        page.on('pageerror', error => errors.push(String(error)));
        await page.goto(origin);
        const baseline = await page.evaluate(() => ({ width: innerWidth, scale: devicePixelRatio }));
        const measurements = [];
        for (const zoom of [1, 2, 4]) {
            const applied = await worker.evaluate(async ({ origin, zoom }) => {
                const tab = (await chrome.tabs.query({})).find(tab => tab.url.startsWith(origin + '/'));
                if (!tab) throw new Error('Isolated zoom test tab not found');
                await chrome.tabs.setZoom(tab.id, zoom);
                return chrome.tabs.getZoom(tab.id);
            }, { origin, zoom });
            assert.ok(Math.abs(applied - zoom) < 0.001);
            await page.waitForFunction(({ baseline, zoom }) => Math.abs(innerWidth - baseline.width / zoom) <= 2 &&
                Math.abs(devicePixelRatio - baseline.scale * zoom) < 0.05, { baseline, zoom });
            const tabs = page.getByRole('tab');
            for (let index = 0; index < await tabs.count(); index += 1) {
                await tabs.nth(index).click();
                const bounds = await page.evaluate(() => ({ width: innerWidth, scale: devicePixelRatio,
                    client: document.documentElement.clientWidth, scroll: document.documentElement.scrollWidth,
                    commands: [...document.querySelectorAll('.terminal-cmd')].filter(element => element.getClientRects().length)
                        .map(element => ({ width: element.clientWidth, scroll: element.scrollWidth })),
                    controls: [...document.querySelectorAll('button')].filter(element => element.getClientRects().length)
                        .map(element => ({ left: element.getBoundingClientRect().left, right: element.getBoundingClientRect().right })) }));
                assert.ok(bounds.scroll <= bounds.client + 2, JSON.stringify({ zoom, bounds }));
                assert.ok(bounds.commands.every(command => command.scroll <= command.width + 2), JSON.stringify({ zoom, bounds }));
                assert.ok(bounds.controls.every(control => control.left >= 0 && control.right <= bounds.width + 2), JSON.stringify({ zoom, bounds }));
                if (index === 0) measurements.push({ zoom: applied, width: bounds.width, devicePixelRatio: bounds.scale });
            }
        }
        assert.deepEqual(errors, []);
        results.push({ browserZoom: measurements, method: 'isolated chrome.tabs.setZoom; all install panels' });
    } finally {
        if (context) await context.close();
        // Only the newly created direct child of the OS temp directory is removed.
        assert.equal(path.dirname(path.resolve(temporary)), path.resolve(tmpdir()));
        assert.ok(path.basename(temporary).startsWith('agentcage-browser-zoom-'));
        await rm(temporary, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    }
}

try {
    const context = await browser.newContext({ permissions: ['clipboard-read', 'clipboard-write'] });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    await page.goto(origin);
    assert.equal(await page.locator('h1:visible').count(), 1);
    const tabs = page.getByRole('tab');
    const tabCount = await tabs.count();
    assert.ok(tabCount >= 2);
    assert.equal(await page.locator('#install-status').textContent(), '');
    const panelNames = { 'install-curl': 'curl installation commands',
        'install-cargo': 'Cargo Git installation commands', 'install-git': 'Source installation commands' };
    await tabs.first().focus();
    for (const [key, index] of [['End', tabCount - 1], ['Home', 0], ['ArrowRight', 1], ['ArrowLeft', 0]]) {
        await page.keyboard.press(key);
        assert.equal(await tabs.nth(index).getAttribute('aria-selected'), 'true');
        assert.equal(await tabs.nth(index).evaluate(element => element === document.activeElement), true);
        const panel = await tabs.nth(index).getAttribute('aria-controls');
        assert.equal(await page.locator(`#${panel}`).isVisible(), true);
        assert.equal(await page.locator('[role="tabpanel"]:visible').count(), 1);
        assert.equal(await page.getByRole('tabpanel', { name: panelNames[panel], exact: true }).isVisible(), true);
        assert.equal(await page.locator('#install-status').textContent(),
            `${(await tabs.nth(index).textContent()).trim()} selected. ${panelNames[panel]}.`);
    }
    results.push('heading, named installation panels, selection feedback and native keyboard tab state');
    const labels = await page.locator('.copy-btn').evaluateAll(buttons => buttons.map(button => button.getAttribute('aria-label')));
    assert.equal(new Set(labels).size, labels.length);
    assert.ok(labels.every(label => label && label.startsWith('Copy ')));
    const copy = page.locator('.copy-btn:visible').first();
    await copy.focus();
    const visibleFocus = await copy.evaluate(button => {
        const style = getComputedStyle(button);
        return { opacity: +style.opacity, outline: style.outlineStyle, width: button.getBoundingClientRect().width, height: button.getBoundingClientRect().height };
    });
    assert.equal(visibleFocus.opacity, 1);
    assert.notEqual(visibleFocus.outline, 'none');
    assert.ok(visibleFocus.width >= 44 && visibleFocus.height >= 44);
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.querySelector('.copy-status').textContent.startsWith('Copied'));
    const id = await copy.getAttribute('data-copy');
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), await page.locator(`#${id}`).textContent());
    assert.equal(await copy.evaluate(element => element === document.activeElement), true);
    results.push('actual clipboard grant and retained keyboard focus');
    for (const viewport of [{ width: 320, height: 900 }, { width: 360, height: 900 }, { width: 390, height: 900 }, { width: 1280, height: 900 }]) {
        await page.setViewportSize(viewport);
        for (const zoom of viewport.width === 1280 ? [1, 2, 4] : [1]) {
            await page.evaluate(value => document.documentElement.style.zoom = String(value), zoom);
            for (let index = 0; index < tabCount; index += 1) {
                await tabs.nth(index).click();
                const overflow = await page.evaluate(() => ({ width: document.documentElement.clientWidth, scroll: document.documentElement.scrollWidth,
                    commands: [...document.querySelectorAll('.terminal-cmd')].filter(element => element.getClientRects().length).map(element => ({ width: element.clientWidth, scroll: element.scrollWidth })) }));
                assert.ok(overflow.scroll <= overflow.width + 2, JSON.stringify({ viewport, zoom, overflow }));
                assert.ok(overflow.commands.every(command => command.scroll <= command.width + 2), JSON.stringify({ viewport, zoom, overflow }));
            }
        }
    }
    results.push('320/360/390 px and 200/400% CSS reflow in all install panels');
    assert.deepEqual(errors, []);
    await context.close();
    for (const mode of ['denied', 'unavailable']) {
        const failureContext = await browser.newContext({ permissions: ['clipboard-read', 'clipboard-write'] });
        await failureContext.addInitScript(mode => {
            // Keep an observer of the real clipboard to verify a native Ctrl+C.
            // The denied case uses a real browser Permissions Policy rejection.
            window.__acReadRealClipboard = navigator.clipboard.readText.bind(navigator.clipboard);
            window.__acRealClipboard = navigator.clipboard;
            if (mode === 'unavailable') Object.defineProperty(navigator, 'clipboard', { configurable: true, value: undefined });
        }, mode);
        const failurePage = await failureContext.newPage();
        const failureErrors = [];
        failurePage.on('pageerror', error => failureErrors.push(String(error)));
        await failurePage.goto(mode === 'denied' ? origin + '/clipboard-denied' : origin);
        assert.equal(await failurePage.evaluate(() => typeof navigator.clipboard?.writeText), mode === 'denied' ? 'function' : 'undefined');
        for (const commandId of ['cmd-curl', 'cmd-cargo', 'cmd-start']) {
            const button = failurePage.locator(`.copy-btn[data-copy="${commandId}"]`);
            if (commandId === 'cmd-cargo') {
                await failurePage.getByRole('tab', { name: 'curl', exact: true }).focus();
                await failurePage.keyboard.press('ArrowRight');
                await failurePage.keyboard.press('Tab');
                assert.equal(await failurePage.locator('#install-cargo').evaluate(element => element === document.activeElement), true);
                await failurePage.keyboard.press('ArrowDown');
                await failurePage.keyboard.press('Tab');
                assert.equal(await button.evaluate(element => element === document.activeElement), true);
            } else {
                await button.focus();
            }
            await failurePage.keyboard.press('Enter');
            await failurePage.waitForFunction(id => document.getElementById(id).closest('.terminal-line')
                .querySelector('.copy-status').textContent.includes('Ctrl+C'), commandId);
            const text = await failurePage.locator(`#${commandId}`).textContent();
            assert.equal(await failurePage.evaluate(() => window.getSelection().toString()), text);
            await failurePage.keyboard.press(process.platform === 'darwin' ? 'Meta+c' : 'Control+c');
            const copied = await failurePage.evaluate(() => window.__acReadRealClipboard());
            // Native Windows copy uses CRLF; preserve all other command bytes.
            assert.equal(copied.replaceAll('\r\n', '\n'), text);
            assert.equal(await button.evaluate(element => element === document.activeElement), true);
        }
        if (mode === 'unavailable') {
            await failurePage.evaluate(() => Object.defineProperty(navigator, 'clipboard', { configurable: true, value: window.__acRealClipboard }));
            const button = failurePage.locator('.copy-btn[data-copy="cmd-start"]');
            await button.focus();
            await failurePage.keyboard.press('Enter');
            await failurePage.waitForFunction(() => document.querySelector('#cmd-start').closest('.terminal-line')
                .querySelector('.copy-status').textContent.startsWith('Copied'));
            assert.equal(await failurePage.locator('#cmd-start').locator('..').locator('..').locator('.copy-status').textContent(),
                'Copied project initialization and Codex commands.');
            assert.equal((await failurePage.evaluate(() => window.__acReadRealClipboard())).replaceAll('\r\n', '\n'),
                await failurePage.locator('#cmd-start').textContent());
            assert.equal(await button.evaluate(element => element === document.activeElement), true);
        }
        assert.deepEqual(failureErrors, []);
        await failureContext.close();
        results.push(`browser executed ${mode === 'denied' ? 'Permissions Policy denial' : 'unavailable API fixture and restored clipboard success'} recovery, Cargo panel keyboard flow and native manual copy of one-line/multiline commands`);
    }
    const noScriptContext = await browser.newContext({ javaScriptEnabled: false, permissions: ['clipboard-read', 'clipboard-write'] });
    try {
        const noScriptPage = await noScriptContext.newPage();
        await noScriptPage.goto(origin);
        assert.equal(await noScriptPage.locator('noscript p').isVisible(), true);
        assert.equal(await noScriptPage.locator('noscript code').textContent(),
            'cargo install --git https://github.com/JanDub-code/AgentCage.git --locked');
        const command = noScriptPage.locator('#cmd-curl');
        await command.click({ clickCount: 3 });
        const text = await command.textContent();
        assert.equal((await noScriptPage.evaluate(() => window.getSelection().toString())).trim(), text);
        await noScriptPage.keyboard.press(process.platform === 'darwin' ? 'Meta+c' : 'Control+c');
        assert.equal((await noScriptPage.evaluate(() => navigator.clipboard.readText())).replaceAll('\r\n', '\n').trim(), text);
        results.push('JavaScript-disabled installation alternatives and native text selection/manual clipboard copy');
    } finally {
        await noScriptContext.close();
    }
    await checkBrowserZoom();
    console.log(JSON.stringify({ browser: await browser.version(), passed: results }, null, 2));
} finally {
    await browser.close();
    await new Promise(resolve => server.close(resolve));
}
