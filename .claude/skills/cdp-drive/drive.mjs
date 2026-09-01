// Drives the running dev build's WebView2 pages over CDP. See SKILL.md for the launch
// step; this script only attaches to an app that is already up on port 9333.
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

// This repo has no playwright of its own; the global resolver finds one (normal resolution,
// else the newest copy in the npx cache) and names the fix command if there is none.
const require = createRequire(import.meta.url);
const { getChromium } = require(join(homedir(), ".claude/skills/_shared/playwright-resolve.cjs"));

const CDP = "http://127.0.0.1:9333";
const DEV_ORIGIN = "localhost:3102";

// The strip's URL is the bare dev origin, so a plain .includes() also matches the flyout,
// settings and overlay pages. Anchoring the end is what keeps them apart.
const PAGE_PATTERNS = {
  strip: /localhost:3102\/?$/,
  settings: /settings\.html/,
  flyout: /flyout\.html/,
  overlay: /overlay\.html/,
};

function usage(msg) {
  if (msg) console.error(`ERROR: ${msg}\n`);
  console.error(
    [
      "usage: node .claude/skills/cdp-drive/drive.mjs <command> [args]",
      "",
      "  targets                 every CDP target, with a hung-build warning",
      "  strips                  tiles, instance ids and width per live strip window",
      "  settings                open Settings > Widgets and dump the lanes/palette state",
      "  shot <out-dir>          screenshot the settings page and the lanes block",
      "  click <selector>        real pointer click in Settings > Widgets (a DOM .click() is ignored)",
      "  select <selector> <v>   set a Settings > Widgets dropdown (e.g. the Placement select)",
      "  drag <widget> <lane>    drag a preview tile into lane N (0-based), then report both lanes",
      "  eval <page> <js>        run JS in a page (strip|settings|flyout|overlay|<url substring>)",
    ].join("\n"),
  );
  process.exit(msg ? 2 : 0);
}

async function fetchTargets() {
  const res = await fetch(`${CDP}/json`).catch(() => null);
  if (!res) {
    throw new Error(
      `nothing answering on ${CDP}. Either the dev build is not running, or it was launched ` +
        "without WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS - see SKILL.md step 2.",
    );
  }
  return res.json();
}

async function connect() {
  const chromium = getChromium();
  const browser = await chromium.connectOverCDP(CDP);
  const pages = browser.contexts().flatMap((c) => c.pages());
  return { browser, pages };
}

function pick(pages, which) {
  const pattern = PAGE_PATTERNS[which];
  const hits = pages.filter((p) => (pattern ? pattern.test(p.url()) : p.url().includes(which)));
  if (hits.length === 0) {
    throw new Error(
      `no page matched "${which}". Live pages:\n  ${pages.map((p) => p.url()).join("\n  ") || "(none)"}`,
    );
  }
  return hits;
}

// settings.html opens on a menu, not the editor: nothing matching .wsf-* exists until the
// Widgets nav row is clicked. The rows are plain divs, so match data-nav, not label text.
async function openWidgetsSection(page) {
  if (await page.$(".wsf-lanes")) return;
  await page.click('[data-nav="section-widgets"]');
  await page.waitForSelector(".wsf-lanes", { timeout: 10_000 });
  await page.waitForTimeout(900);
}

async function cmdTargets() {
  const targets = await fetchTargets();
  for (const t of targets) console.log(` ${t.type} | ${t.title} | ${t.url}`);
  // A window whose build hung never navigates off about:blank; that is the signature of a
  // WebviewWindowBuilder::build() dispatched into the event loop instead of queued (todo 46).
  const stuck = targets.filter((t) => t.type === "page" && t.url === "about:blank");
  if (stuck.length > 0) {
    console.log(`\nWARNING: ${stuck.length} page target(s) stuck on about:blank - a hung window build.`);
  }
}

async function cmdStrips() {
  const { browser, pages } = await connect();
  for (const p of pick(pages, "strip")) {
    const state = await p.evaluate(() => ({
      tiles: [...document.querySelectorAll("#strip .tile")].map((t) => ({
        widget: t.dataset.widget,
        instance: t.dataset.instance,
      })),
      width: Math.round(document.getElementById("strip").getBoundingClientRect().width),
    }));
    console.log(p.url(), JSON.stringify(state));
  }
  await browser.close();
}

async function cmdSettings() {
  const { browser, pages } = await connect();
  const [page] = pick(pages, "settings");
  await openWidgetsSection(page);
  const state = await page.evaluate(() => ({
    laneCount: document.querySelectorAll(".wsf-lane").length,
    heads: [...document.querySelectorAll(".wsf-lane-head")].map((h) => h.textContent.trim()),
    strips: [...document.querySelectorAll(".wsf-strip")].map((s) => ({
      monitor: s.dataset.monitor,
      tiles: [...s.children].map((t) => t.dataset.widget),
    })),
    palette: [...document.querySelectorAll(".wsf-chip")].map((c) => c.dataset.widget),
    configFields: [...document.querySelectorAll(".wsf-config .kit-row-label")].map((l) =>
      l.textContent.trim(),
    ),
  }));
  console.log(JSON.stringify(state, null, 2));
  await browser.close();
}

async function cmdShot(outDir) {
  if (!outDir) usage("shot needs an output directory");
  mkdirSync(outDir, { recursive: true });
  const { browser, pages } = await connect();
  const [page] = pick(pages, "settings");
  await openWidgetsSection(page);
  await page.screenshot({ path: `${outDir}/settings-full.png`, fullPage: true });
  const lanes = await page.$(".wsf-lanes");
  if (lanes) await lanes.screenshot({ path: `${outDir}/lanes.png` });
  console.log("shot ->", outDir);
  await browser.close();
}

function laneState(page) {
  return page.evaluate(() =>
    [...document.querySelectorAll(".wsf-strip")].map((s) => ({
      monitor: s.dataset.monitor,
      tiles: [...s.children].map((t) => t.dataset.widget),
    })),
  );
}

// A real pointer click, because the preview tiles listen to pointerdown for drag and ignore
// a DOM .click() entirely. Falls back to a raw mouse click at the element's centre: the kit's
// toggle track sits inside a `label` whose first labelable control is a disabled range input,
// so Playwright reads the whole label as disabled and refuses the actionability check.
async function cmdClick(selector) {
  if (!selector) usage("click needs a selector");
  const { browser, pages } = await connect();
  const [page] = pick(pages, "settings");
  await openWidgetsSection(page);
  try {
    await page.click(selector, { timeout: 5000 });
    console.log("clicked", selector);
  } catch {
    const el = await page.$(selector);
    if (!el) throw new Error(`no element matched ${selector}`);
    const box = await el.boundingBox();
    if (!box) throw new Error(`${selector} has no layout box to click`);
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    console.log("clicked", selector, "(raw mouse fallback)");
  }
  await page.waitForTimeout(400);
  await browser.close();
}

async function cmdSelect(selector, value) {
  if (!selector || value === undefined) usage("select needs a selector and a value");
  const { browser, pages } = await connect();
  const [page] = pick(pages, "settings");
  await openWidgetsSection(page);
  await page.selectOption(selector, value);
  await page.waitForTimeout(600);
  console.log(
    "selected",
    value,
    "->",
    JSON.stringify(
      await page.evaluate(() =>
        [...document.querySelectorAll(".wsf-config .kit-row-label")].map((l) => l.textContent.trim()),
      ),
    ),
  );
  await browser.close();
}

async function cmdDrag(widget, laneArg) {
  if (!widget) usage("drag needs a widget id");
  const laneIndex = Number(laneArg ?? 1);
  const { browser, pages } = await connect();
  const [page] = pick(pages, "settings");
  await openWidgetsSection(page);

  const source = await page.$(`.wsf-strip [data-widget="${widget}"]`);
  if (!source) throw new Error(`no preview tile for "${widget}" in any lane`);
  const strips = await page.$$(".wsf-strip");
  if (!strips[laneIndex]) throw new Error(`lane ${laneIndex} does not exist (${strips.length} lanes)`);
  const from = await source.boundingBox();
  const to = await strips[laneIndex].boundingBox();

  const startX = from.x + from.width / 2;
  const startY = from.y + from.height / 2;
  const endX = to.x + 40;
  const endY = to.y + to.height / 2;
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  // Stepped, not a single jump: the drag handler tracks pointermove, so one hop reads as no drag.
  for (let i = 1; i <= 12; i++) {
    await page.mouse.move(startX + ((endX - startX) * i) / 12, startY + ((endY - startY) * i) / 12);
    await page.waitForTimeout(30);
  }
  await page.mouse.up();
  await page.waitForTimeout(2500);

  console.log(JSON.stringify(await laneState(page), null, 2));
  await browser.close();
}

async function cmdEval(which, js) {
  if (!which || !js) usage("eval needs a page and an expression");
  const { browser, pages } = await connect();
  for (const p of pick(pages, which)) {
    const out = await p.evaluate(`(() => (${js}))()`);
    console.log(p.url(), JSON.stringify(out, null, 2));
  }
  await browser.close();
}

const [command, ...args] = process.argv.slice(2);
const commands = {
  targets: cmdTargets,
  strips: cmdStrips,
  settings: cmdSettings,
  shot: cmdShot,
  click: cmdClick,
  select: cmdSelect,
  drag: cmdDrag,
  eval: cmdEval,
};
if (!command || !commands[command]) usage(command ? `unknown command "${command}"` : null);

try {
  await commands[command](...args);
} catch (e) {
  console.error(`ERROR: ${e.message}`);
  console.error(`(dev origin is ${DEV_ORIGIN}; see .claude/skills/cdp-drive/SKILL.md)`);
  process.exit(1);
}
