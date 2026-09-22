// Run: node scripts/test_frontend_security.mjs (CHROME_PATH overrides browser lookup).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { build } from "esbuild";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const chrome = [process.env.CHROME_PATH,
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "/usr/bin/chromium", "/usr/bin/google-chrome", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
].find((path) => path && existsSync(path));
assert.ok(chrome, "Install Chrome/Chromium or set CHROME_PATH; no browser test was run.");
const main = readFileSync(join(root, "src/main.ts"), "utf8");
const initOffset = main.lastIndexOf('\nif (document.readyState === "loading")');
assert.ok(initOffset > 0, "Locate app startup before running isolated fixtures.");

const probes = String.raw`
(async () => {
  const failures = [];
  let passed = 0;
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const probe = async (name, fn) => {
    try { await fn(); passed++; }
    catch (error) { failures.push(name + ': ' + error.message); }
  };
  const text = '中文 & <img data-injected src=x onerror="window.__xss=1"> "单引号\'';
  const attr = 'id" data-injected="yes';
  const goodUrl = 'https://example.test/中文?a=1&b="引号"&c=\'单引号\'';
  const badUrls = ['javascript:window.__xss=1', 'JaVaScRiPt:alert(1)',
    'java\nscript:alert(1)', 'data:text/html,<script>alert(1)</script>',
    'file:///C:/secret', 'ftp://example.test/x', '//example.test/x', '/relative',
    'https:example.test', 'https:///example.test', 'https://',
    'https://exa mple.test', 'https://example.test/\npath', 'https://example.test\\path'];
  const safe = (id) => {
    check(!$(id).querySelector('[data-injected],script,iframe,svg'), id + ' contains injected elements/attributes');
    check(!Array.from($(id).querySelectorAll('*')).some(el => Array.from(el.attributes).some(a => /^on/i.test(a.name))), id + ' contains an event handler');
  };
  const entry = { id: attr, name: text, genre_id: attr, creator: text, rating: text,
    review: text, tasting_date: null, tags: [text], images: [], links: [],
    created_at: '', updated_at: '' };

  await probe('HTML escaping in text and quoted attributes', () => {
    const holder = document.createElement('div');
    holder.innerHTML = '<span title="' + escapeHtml(text) + '">' + escapeHtml(text) + '</span>';
    check(holder.firstChild.textContent === text && holder.firstChild.title === text, 'text changed');
    check(holder.querySelectorAll('*').length === 1, 'HTML became markup');
  });
  await probe('genre filter and options', () => {
    genres = [{id: attr, name: text, is_default: false, created_at: ''}];
    renderGenres(); safe('genre-filter'); safe('entry-genre');
    check($('entry-genre').options[0].text === text, 'genre label changed');
    check($('entry-genre').value === attr, 'genre id changed');
    check($('genre-filter').querySelector('label').htmlFor === 'genre-' + attr, 'label association changed');
  });
  await probe('tag and year filters', () => {
    renderTagFilter([text]); safe('tag-filter');
    check($('tag-filter').querySelector('label').textContent === text, 'tag changed');
    renderYearFilter([2026]); check($('year-filter').options[1].value === '2026', 'year changed');
  });
  await probe('entry cards and append', () => {
    const summary = {...entry, genre_name: text, review_preview: text, primary_image: null};
    renderEntryCards([summary]); renderEntryCards([summary], true); safe('entry-list');
    check($('entry-list').querySelectorAll('.entry-card').length === 2, 'append changed');
    check($('entry-list').firstElementChild.dataset.id === attr, 'entry id changed');
    check($('entry-list').querySelector('.rating-badge').textContent === text, 'rating changed');
  });
  await probe('statistics labels', () => {
    fillStats({total: 1, rating_dist: [[text, 1]], genre_dist: [[text, 1]], year_dist: [['2026', 1]]});
    safe('stats-rating'); safe('stats-genre');
    check($('stats-genre').querySelector('.stats-bar-label').textContent === text, 'stats label changed');
  });
  await probe('detail HTTP links and inert legacy links', async () => {
    await renderDetailModal({...entry, links: [{url: goodUrl, label: text}, ...badUrls.map(url => ({url, label: url}))]});
    safe('detail-links'); safe('detail-tags');
    const anchors = $('detail-links').querySelectorAll('a');
    check(anchors.length === 1, 'unsafe protocol became clickable');
    check(anchors[0].getAttribute('href') === goodUrl && anchors[0].textContent === text, 'valid URL/label changed');
    check(anchors[0].target === '_blank' && anchors[0].relList.contains('noopener') && anchors[0].relList.contains('noreferrer'), 'missing new-window protections');
    for (const url of badUrls) check($('detail-links').textContent.includes(url), 'legacy link was silently dropped');
  });
  await probe('editing and local image attributes', async () => {
    const path = 'covers/file.png" data-injected="yes';
    await populateEntryForm({...entry, rating: 'S', links: [{label: text, url: goodUrl}], images: [{id: attr, path, is_primary: true}]});
    safe('links-container'); safe('images-container');
    check($('links-container').querySelector('.link-label').value === text, 'edit label changed');
    check($('links-container').querySelector('.link-url').value === goodUrl, 'edit URL changed');
    check($('images-container').firstElementChild.dataset.path === path, 'image path changed');
    await renderDetailModal({...entry, images: [{id: attr, path, is_primary: true}]});
    safe('detail-main-image'); safe('detail-thumbnails');
    await addImageToContainer(path); safe('images-container');
    renderEntryCards([{...entry, genre_name: text, review_preview: text, primary_image: path}]);
    await new Promise(resolve => setTimeout(resolve, 0)); safe('entry-list');
  });
  await probe('cover sources and loading message', async () => {
    coverSources = [{id: attr, name: text, usage: text, source_type: text}];
    renderCoverSourceList(); safe('cover-source-list');
    check($('cover-source-list').firstElementChild.dataset.id === attr, 'source id changed');
    check($('cover-source-list').querySelector('.source-item-name').textContent === text, 'source text changed');
    const loading = fetchAndShowCandidates('fixture', text);
    safe('cover-candidates'); await loading;
  });
  await probe('dictionary keys remain ordinary user text', () => {
    for (const key of ['constructor', '__proto__', 'toString']) {
      check(getGenreIcon(key) === '📁' && getGenreColor(key) === '#868e96', 'inherited genre value');
      coverSources = [{id: key, name: key, usage: key, source_type: key}];
      renderCoverSourceList();
      check($('cover-source-list').querySelector('.source-item-usage').textContent === key, 'usage changed');
      check($('cover-source-list').querySelector('.source-item-desc').textContent === key, 'description changed');
      fillStats({total: 1, rating_dist: [[key, 1]], genre_dist: [[key, 1]], year_dist: []});
      check($('stats-rating').querySelector('.stats-bar-fill').style.background === 'rgb(166, 173, 200)', 'inherited stats color');
    }
  });
  await probe('cover candidate URL boundary and thumbnail fallback', async () => {
    window.__candidates = [
      ...badUrls.map(url => ({url, thumbnail_url: goodUrl, title: text})),
      ...badUrls.map(thumbnail_url => ({url: goodUrl, thumbnail_url, title: text})),
      {url: goodUrl, thumbnail_url: 'http://example.test/thumb?a=1&b=2', title: text},
    ];
    const candidates = await api.fetchCoverCandidates('fixture', null, 'fixture');
    check(candidates.length === badUrls.length + 1, 'unsafe full-size URL survived');
    check(candidates.slice(0, -1).every(c => c.thumbnail_url === null), 'unsafe thumbnail URL survived');
    check(candidates.at(-1).thumbnail_url === 'http://example.test/thumb?a=1&b=2', 'valid thumbnail changed');
    const seen = [];
    const NativeImage = window.Image;
    window.Image = class extends NativeImage { set src(value) { seen.push(value); } };
    const before = window.__calls.length;
    try { await renderCoverCandidates(candidates, text); } finally { window.Image = NativeImage; }
    safe('cover-candidates');
    const previewCalls = window.__calls.slice(before).filter(call => call.command === 'fetch_cover_preview');
    check(previewCalls.length === candidates.length && previewCalls.slice(0, -1).every(call => call.args.url === goodUrl), 'preview bypassed backend throttle');
    check(seen.length === candidates.length && seen.every(url => url.startsWith('data:image/')), 'browser made an unthrottled preview request');
    check($('cover-candidates').querySelector('.cover-title').textContent === text, 'cover title changed');
  });
  await probe('download rejects every non-HTTP URL before IPC', async () => {
    for (const url of badUrls) {
      const before = window.__calls.length;
      let rejected = false;
      try { await api.downloadCover(url, 'fixture', null); } catch { rejected = true; }
      check(rejected && window.__calls.length === before, 'download accepted ' + url);
    }
    await api.downloadCover(goodUrl, 'fixture', null);
    check(window.__calls.at(-1).args.url === goodUrl, 'download URL changed');
  });
  await probe('form rejects invalid links without saving or losing inputs', async () => {
    await populateEntryForm({...entry, id: '', rating: 'S', images: [], links: [{label: text, url: goodUrl}]});
    const input = $('links-container').querySelector('.link-url');
    // Native URL inputs remove CR/LF; the resulting https://example.test/path is valid.
    const formBadUrls = badUrls.filter(url => url !== 'https://example.test/\npath');
    for (const url of [...formBadUrls, '']) {
      input.value = url;
      const before = window.__calls.length;
      await handleEntrySubmit(new Event('submit', {cancelable: true}));
      check(window.__calls.length === before, 'invalid link reached save IPC: ' + url);
      check(input.isConnected && !input.disabled && !$('btn-save-entry').disabled, 'invalid form lost inputs or locked submit');
      check($('toast').classList.contains('error'), 'invalid URL had no error message');
    }
    for (const id of ['', 'existing-entry']) {
      $('entry-id').value = id;
      input.value = goodUrl;
      await handleEntrySubmit(new Event('submit', {cancelable: true}));
      const call = window.__calls.at(-1);
      check(call.command === (id ? 'update_entry' : 'create_entry'), 'valid form not submitted');
      check(call.args.req.links[0].url === goodUrl && call.args.req.links[0].label === text, 'saved link changed');
    }
  });
  check(!window.__xss, 'payload executed');
  const result = document.createElement('pre'); result.id = 'security-result';
  result.textContent = encodeURIComponent(JSON.stringify({passed, failures})); document.body.append(result);
})().catch(error => { document.body.innerHTML = '<pre id="security-result">' + encodeURIComponent(JSON.stringify({passed: 0, failures: [error.stack]})) + '</pre>'; });
`;
const bundle = await build({
  stdin: {contents: main.slice(0, initOffset) + probes, resolveDir: join(root, "src"), loader: "ts"},
  bundle: true, write: false, format: "iife", loader: {".css": "empty"},
  plugins: [{name: "isolated-tauri", setup(build) {
    build.onResolve({filter: /^@tauri-apps\//}, args => ({path: args.path, namespace: "fixture"}));
    build.onLoad({filter: /.*/, namespace: "fixture"}, () => ({contents: "export const invoke = (...args) => window.__invoke(...args); export const open = () => null; export const save = () => null;"}));
  }}],
});
const prelude = `window.__calls = []; window.__candidates = [];
window.__invoke = async (command, args) => {
  window.__calls.push({command, args});
  if (command === 'get_image_base64') return 'AA==';
  if (command === 'fetch_cover_preview') return 'data:image/png;base64,AA==';
  if (command === 'fetch_cover_candidates') return window.__candidates;
  if (command === 'create_entry' || command === 'update_entry') throw new Error('fixture: simulated save failure');
  return 'fixture';
};`;
const temp = mkdtempSync(join(tmpdir(), "prefdb-security-"));
try {
  const html = readFileSync(join(root, "index.html"), "utf8")
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, "")
    .replace(/<link\b[^>]*>/g, "")
    .replace('<head>', '<head><meta http-equiv="Content-Security-Policy" content="default-src \'none\'; script-src \'unsafe-inline\'; img-src data:; style-src \'unsafe-inline\'">')
    .replace('</body>', () => '<script>' + (prelude + bundle.outputFiles[0].text).replace(/<\/script/gi, '<\\/script') + '</script></body>');
  const htmlPath = join(temp, "probe.html"); writeFileSync(htmlPath, html);
  const output = execFileSync(chrome, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
    "--disable-extensions", "--disable-background-networking", "--user-data-dir=" + join(temp, "profile"),
    "--dump-dom", "--virtual-time-budget=3000", pathToFileURL(htmlPath).href],
    {encoding: "utf8", timeout: 30000, maxBuffer: 4 * 1024 * 1024, windowsHide: true});
  const match = output.match(/<pre id="security-result">(%7B[^<]+)<\/pre>/);
  assert.ok(match, "Browser did not report results; no passing test is claimed.");
  const result = JSON.parse(decodeURIComponent(match[1]));
  console.log(JSON.stringify(result, null, 2));
  assert.equal(result.failures.length, 0, "Frontend security probes failed");
  console.log("PASS: real Chrome DOM; synthetic IPC only, no database or external image requests.");
} finally {
  rmSync(temp, {recursive: true, force: true, maxRetries: 5, retryDelay: 100});
}
