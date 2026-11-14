// Render captured terminal output to PNG and validate/render Mermaid blocks. Dependencies stay outside the repo.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';

const [mode, source, destination] = process.argv.slice(2);
const modules = process.env.CICADA_DOCS_MODULES;
const chrome = process.env.CICADA_DOCS_CHROME;
if (!modules || !chrome || !['terminal', 'mermaid'].includes(mode) || !source || !destination) {
  throw new Error('See docs/assets/README.md for dependencies, browser path and invocation.');
}
const require = createRequire(path.join(path.resolve(modules), 'package.json'));
const { default: puppeteer } = await import(pathToFileURL(require.resolve('puppeteer')));
const browser = await puppeteer.launch({ executablePath: chrome, headless: true });
try {
  const page = await browser.newPage();
  await page.setViewport({ width: 1400, height: 1000, deviceScaleFactor: 2 });
  await page.setContent('<!doctype html><html><head><meta charset="utf-8"></head><body></body></html>');
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  if (mode === 'terminal') {
    const recording = JSON.parse(await fs.readFile(source, 'utf8'));
    await page.addStyleTag({ path: require.resolve('@xterm/xterm/css/xterm.css') });
    await page.addStyleTag({ content: 'body { margin: 0; background: #151b24; } #screen { display: inline-block; padding: 24px; }' });
    await page.addScriptTag({ path: require.resolve('@xterm/xterm') });
    await page.evaluate(async recording => {
      document.body.innerHTML = '<div id="screen"><div id="terminal"></div></div>';
      const term = new window.Terminal({
        cols: recording.cols, rows: recording.rows, fontFamily: 'Menlo, monospace', fontSize: 14,
        lineHeight: 1.3, cursorBlink: false, allowProposedApi: true,
        theme: {
          background: '#151b24', foreground: '#d9e2ef', black: '#151b24',
          red: '#e06c75', green: '#98c379', yellow: '#e5c07b', blue: '#61afef',
          magenta: '#c678dd', cyan: '#56b6c2', white: '#d9e2ef', brightBlack: '#8390a5',
          brightRed: '#f08a93', brightGreen: '#b1d494', brightYellow: '#f2d492',
          brightBlue: '#86c5ff', brightMagenta: '#d9a0ef', brightCyan: '#81d9e0', brightWhite: '#ffffff',
        },
      });
      term.open(document.getElementById('terminal'));
      await document.fonts.ready;
      await new Promise(resolve => term.write(recording.ansi, resolve));
      // Preserve the application's final captured frame, without a browser input cursor over a cell.
      term.blur();
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, recording);
    await fs.mkdir(path.dirname(destination), { recursive: true });
    const screen = await page.$('#screen');
    await screen.screenshot({ path: destination });
    console.log(`Terminal screenshot: ${destination}`);
  } else {
    await page.addScriptTag({ path: path.join(path.dirname(require.resolve('mermaid/package.json')), 'dist/mermaid.min.js') });
    const markdown = await fs.readFile(source, 'utf8');
    const blocks = [...markdown.matchAll(/^```mermaid\n([\s\S]*?)^```/gm)].map(match => match[1]);
    await fs.mkdir(destination, { recursive: true });
    for (const [index, definition] of blocks.entries()) {
      const result = await page.evaluate(async ({ definition, index }) => {
        window.mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: 'neutral' });
        await window.mermaid.parse(definition);
        return window.mermaid.render(`diagram-${index}`, definition);
      }, { definition, index });
      await fs.writeFile(path.join(destination, `${index + 1}.svg`), result.svg);
      await page.evaluate(svg => { document.body.innerHTML = '<div style="padding:16px;background:white">' + svg + '</div>'; }, result.svg);
      const svg = await page.$('svg');
      await svg.screenshot({ path: path.join(destination, `${index + 1}.png`) });
    }
    console.log(`${source}: ${blocks.length} Mermaid diagrams parsed and rendered`);
  }
  if (errors.length) throw new Error(errors.join('\n'));
} finally {
  await browser.close();
}
