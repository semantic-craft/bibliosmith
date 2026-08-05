#!/usr/bin/env node
/* global document */
const fs = require('fs');
const path = require('path');
const { fileURLToPath, pathToFileURL } = require('url');

function loadPlaywright() {
  const candidates = [
    path.resolve(__dirname, '../../../../books/node_modules/playwright-core'),
    path.resolve(__dirname, '../../../../vendor/playwright-core'),
  ];
  const found = candidates.find((candidate) => fs.existsSync(candidate));
  if (!found) throw new Error('Bundled playwright-core is missing.');
  return require(found);
}

function argument(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) throw new Error(`Missing ${name}.`);
  return process.argv[index + 1];
}

async function main() {
  const root = path.resolve(argument('--root'));
  const executablePath = path.resolve(argument('--browser'));
  const spinePaths = JSON.parse(argument('--spine-json'));
  const { chromium } = loadPlaywright();
  const browser = await chromium.launch({
    executablePath,
    headless: true,
    args: ['--disable-background-networking', '--disable-component-update', '--no-first-run'],
  });
  const measurements = [];
  try {
    for (const width of [390, 430]) {
      const context = await browser.newContext({
        viewport: { width, height: 800 },
        javaScriptEnabled: false,
      });
      await context.route('**/*', async (route) => {
        let protocol;
        try {
          protocol = new URL(route.request().url()).protocol;
        } catch {
          await route.abort('blockedbyclient');
          return;
        }
        if (protocol === 'file:') {
          try {
            const requestedPath = path.resolve(fileURLToPath(route.request().url()));
            if (requestedPath === root || requestedPath.startsWith(`${root}${path.sep}`)) {
              await route.continue();
            } else {
              await route.abort('blockedbyclient');
            }
          } catch {
            await route.abort('blockedbyclient');
          }
        } else if (['data:', 'blob:'].includes(protocol)) {
          await route.continue();
        } else {
          await route.abort('blockedbyclient');
        }
      });
      const page = await context.newPage();
      for (const relativePath of spinePaths) {
        const target = path.resolve(root, relativePath);
        if (!target.startsWith(`${root}${path.sep}`)) throw new Error('Spine path escapes EPUB root.');
        await page.goto(pathToFileURL(target).href, { waitUntil: 'load', timeout: 15000 });
        const geometry = await page.evaluate(() => {
          const rootElement = document.documentElement;
          const body = document.body;
          const viewportWidth = rootElement.clientWidth;
          const viewportHeight = rootElement.clientHeight;
          const scrollWidth = Math.max(rootElement.scrollWidth, body ? body.scrollWidth : 0);
          const clippedHeadings = [...document.querySelectorAll('h1,h2,h3,h4,h5,h6')]
            .filter((heading) => {
              const box = heading.getBoundingClientRect();
              return box.left < -1 || box.right > viewportWidth + 1 || box.height < 1;
            }).length;
          const candidates = [...document.querySelectorAll(
            'main h1,main h2,main h3,main h4,main h5,main h6,main p,main table,main img',
          )];
          const first = candidates.find((element) => {
            const box = element.getBoundingClientRect();
            const meaningful = element.matches('img,table')
              || (element.textContent || '').trim().length > 0;
            return meaningful && box.height > 0;
          });
          const firstBox = first ? first.getBoundingClientRect() : null;
          return {
            viewportWidth,
            scrollWidth,
            overflow: scrollWidth > viewportWidth + 1,
            clippedHeadings,
            blankFirstScreen: !firstBox || firstBox.top >= viewportHeight || firstBox.bottom <= 0,
          };
        });
        measurements.push({ width, path: relativePath, ...geometry });
      }
      await context.close();
    }
  } finally {
    await browser.close();
  }
  process.stdout.write(`${JSON.stringify({ browserVersion: browser.version(), measurements })}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error.stack || error.message}\n`);
  process.exitCode = 1;
});
