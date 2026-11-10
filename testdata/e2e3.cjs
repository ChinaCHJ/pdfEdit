const puppeteer = require('puppeteer-core');
const fs = require('fs');
const path = require('path');

const CHROME = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const BASE = 'C:/Users/yhpc2/Desktop/3D Design/pdf-editor/testdata';
const OUT = path.join(BASE, 'shots');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  fs.mkdirSync(OUT, { recursive: true });

  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-gpu', '--force-device-scale-factor=1'],
    defaultViewport: { width: 1280, height: 1000 },
    protocolTimeout: 120000,
  });
  const page = await browser.newPage();
  await page.evaluateOnNewDocument(() => {
    const orig = URL.createObjectURL.bind(URL);
    window.__blobs = [];
    URL.createObjectURL = function (b) { try { window.__blobs.push(b); } catch { } return orig(b); };
  });
  page.on('pageerror', (e) => console.log('[pageerror]', e.message));
  page.on('console', (m) => { if (m.type() === 'error' && !m.text().includes('favicon')) console.log('[err]', m.text()); });

  await page.goto('http://127.0.0.1:17800', { waitUntil: 'networkidle0' });
  await (await page.$('#fileInput')).uploadFile(path.join(BASE, 'multi.pdf'));
  await page.waitForFunction(
    () => (document.querySelector('#stat')?.textContent || '').includes('已加载'),
    { timeout: 120000 }
  );
  await sleep(600);

  const blocks = await page.$$('.txt');
  console.log('文字块总数:', blocks.length);

  /* ---------- 1. 自动缩小字号 ---------- */
  const target = blocks[1];
  const before = await page.evaluate((el) => ({
    fs: parseFloat(el.style.fontSize),
    w: parseFloat(el.style.width),
    text: el.textContent,
  }), target);
  console.log('[1] 原始: 字号', before.fs.toFixed(2), '宽', before.w.toFixed(1), '字数', before.text.length);

  await target.click();
  await page.waitForFunction(() => !!document.querySelector('.txt.focus'), { timeout: 8000 });
  await page.evaluate(() => {
    const el = document.querySelector('.txt.focus');
    el.focus();
    const r = document.createRange();
    r.selectNodeContents(el);
    const s = window.getSelection();
    s.removeAllRanges();
    s.addRange(r);
  });
  await page.keyboard.type('替换后的这一行文字特意写得比原来长很多很多，用来验证自动缩小字号的功能是否真的生效');
  await page.mouse.click(100, 720);   // 点页面外空白处退出编辑
  await sleep(500);

  const after = await page.evaluate((el) => ({
    fs: parseFloat(el.style.fontSize),
    h: parseFloat(el.style.height),
    dirty: el.classList.contains('dirty'),
    text: el.innerText.replace(/\n+$/, ''),
  }), target);
  const shrunk = after.fs < before.fs * 0.95;
  console.log('[1] 修改后: 字号', after.fs.toFixed(2), '高', after.h.toFixed(1),
    '| 缩小了:', shrunk ? '是' : '否', '| dirty:', after.dirty);
  console.log('[1] 文本:', after.text);

  /* ---------- 2. 换颜色 + 换字体 ---------- */
  await target.click();
  await page.waitForFunction(() => !!document.querySelector('.txt.focus'), { timeout: 8000 });
  await page.evaluate(() => document.querySelectorAll('#textColors .sq')[4].click());  // 红
  await page.select('#fontSel', "'KaiTi','楷体',serif");
  await sleep(300);

  const styled = await page.evaluate((el) => ({
    fg: el.dataset.fg,
    fam: el.style.fontFamily,
    fs: parseFloat(el.style.fontSize),
  }), target);
  console.log('[2] 颜色:', styled.fg, '| 字体:', styled.fam, '| 字号:', styled.fs.toFixed(2));

  /* ---------- 3. 并入下一行 ---------- */
  const hBefore = await page.evaluate((el) => parseFloat(el.style.height), target);
  await page.evaluate(() => document.querySelector('#btnMergeNext').click());
  await sleep(600);

  const merged = await page.evaluate((el) => ({
    h: parseFloat(el.style.height),
    count: el.dataset.mergedCount,
    text: el.innerText.replace(/\n+$/, ''),
    hidden: [...document.querySelectorAll('.txt')].filter((d) => d.style.display === 'none').length,
  }), target);
  console.log('[3] 合并前高', hBefore.toFixed(1), '→ 合并后高', merged.h.toFixed(1),
    '| 合并次数:', merged.count, '| 隐藏行数:', merged.hidden);
  console.log('[3] 合并后文本:', merged.text.slice(0, 80));

  await page.mouse.click(100, 720);
  await sleep(300);
  await page.screenshot({ path: path.join(OUT, 'polish-edit.png') });

  /* ---------- 4. 导出 ---------- */
  await page.$eval('#btnSave', (b) => b.click());
  await page.waitForFunction(() => {
    const t = document.querySelector('#stat')?.textContent || '';
    return t.includes('已导出') || t.includes('失败');
  }, { timeout: 180000 });
  console.log('[4]', await page.evaluate(() => document.querySelector('#stat').textContent));

  const b64 = await page.evaluate(async () => {
    const arr = window.__blobs || [];
    const blob = arr[arr.length - 1];
    if (!blob) return null;
    const buf = new Uint8Array(await blob.arrayBuffer());
    let bin = '';
    for (let i = 0; i < buf.length; i += 8192) bin += String.fromCharCode.apply(null, buf.subarray(i, i + 8192));
    return btoa(bin);
  });
  if (b64) {
    const out = path.join(OUT, 'polish-out.pdf');
    fs.writeFileSync(out, Buffer.from(b64, 'base64'));
    console.log('[4] 已保存', (fs.statSync(out).size / 1024).toFixed(0), 'KB');
  }

  await browser.close();
})().catch((e) => { console.error('测试失败:', e.message); process.exit(1); });
