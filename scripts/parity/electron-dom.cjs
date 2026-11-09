// Usage: electron electron-dom.cjs <url> <out.json> [--size 1200x800] [--wait 3000]
// Loads <url> in a frameless window of the given content size, waits, and
// writes the boxes of dom-rects.js. Requests outside the page's origin are
// blocked so a harness cannot reach the network. Run under xvfb-run on CI.
const { app, BrowserWindow, session } = require('electron');
const fs = require('fs');
const path = require('path');
const args = process.argv.slice(process.argv.indexOf(__filename) + 1);
const [url, out] = args;
const option = (name, fallback) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : fallback;
};
const [width, height] = option('--size', '1200x800').split('x').map(Number);
const wait = Number(option('--wait', '3000'));
if (!url || !out) {
  console.error('usage: electron electron-dom.cjs <url> <out.json> [--size WxH] [--wait ms]');
  process.exit(64);
}
app.whenReady().then(async () => {
  const origin = new URL(url).origin;
  session.defaultSession.webRequest.onBeforeRequest((details, done) => {
    const allowed = details.url.startsWith(origin) || details.url.startsWith('data:')
      || details.url.startsWith(origin.replace(/^http/, 'ws'));
    done({ cancel: !allowed });
  });
  const timer = setTimeout(() => app.exit(2), wait + 30000);
  try {
    const window = new BrowserWindow({ width, height, useContentSize: true, frame: false, show: true });
    await window.loadURL(url);
    // No hover state: park the pointer in the corner.
    window.webContents.sendInputEvent({ type: 'mouseMove', x: width - 1, y: height - 1 });
    await new Promise((resolve) => setTimeout(resolve, wait));
    const script = fs.readFileSync(path.join(__dirname, 'dom-rects.js'), 'utf8');
    fs.writeFileSync(out, await window.webContents.executeJavaScript(script));
  } catch (error) {
    console.error(String(error));
    process.exitCode = 1;
  }
  clearTimeout(timer);
  app.quit();
});
