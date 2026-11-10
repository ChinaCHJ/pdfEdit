const puppeteer = require('puppeteer-core');
const fs = require('fs');
const path = require('path');

const CHROME = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const BASE = 'C:/Users/yhpc2/Desktop/3D Design/pdf-editor/testdata';
const OUT = path.join(BASE, 'shots');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function boot() {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-gpu', '--force-device-scale-factor=1'],
    defaultViewport: { width: 1280, height: 1000 },
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
  return { browser, page };
}

async function loadFile(page, file) {
  await (await page.$('#fileInput')).uploadFile(file);
  await page.waitForFunction(
    () => (document.querySelector('#stat')?.textContent || '').includes('已加载') ||
          (document.querySelector('#stat')?.textContent || '').includes('失败'),
    { timeout: 120000 }
  );
  await sleep(500);
}

async function grabPdf(page, name) {
  await page.waitForFunction(
    () => { const t = document.querySelector('#stat')?.textContent || ''; return t.includes('已导出') || t.includes('失败'); },
    { timeout: 180000 }
  );
  const statText = await page.evaluate(() => document.querySelector('#stat').textContent);
  const b64 = await page.evaluate(async () => {
    const arr = window.__blobs || [];
    const blob = arr[arr.length - 1];
    if (!blob) return null;
    const buf = new Uint8Array(await blob.arrayBuffer());
    let bin = '';
    for (let i = 0; i < buf.length; i += 8192) bin += String.fromCharCode.apply(null, buf.subarray(i, i + 8192));
    return btoa(bin);
  });
  if (!b64) { console.log(`[${name}] 未捕获 blob | ${statText}`); return null; }
  const out = path.join(OUT, name);
  fs.writeFileSync(out, Buffer.from(b64, 'base64'));
  console.log(`[${name}] ${statText} | ${(fs.statSync(out).size / 1024).toFixed(0)} KB`);
  return out;
}

(async () => {
  /* ============ 用例 1：表单 + 批注 + 水印 ============ */
  {
    const { browser, page } = await boot();
    await loadFile(page, path.join(BASE, 'form.pdf'));

    const info = await page.evaluate(() => ({
      fields: document.querySelectorAll('.ff').length,
      pages: [...document.querySelectorAll('.pw')].filter((e) => e.style.display !== 'none').length,
    }));
    console.log('[form] 识别表单字段', info.fields, '个，页数', info.pages);

    // 填表单
    await page.click('.tab[data-mode="form"]');
    const inputs = await page.$$('.ff');
    const vals = ['张伟', '信息安全部', '安全工程师', '2026-11-10'];
    for (let i = 0; i < vals.length && i < inputs.length - 1; i++) {
      await inputs[i].click();
      await page.keyboard.type(vals[i]);
    }
    const cb = inputs[inputs.length - 1];
    await cb.click();

    // 批注：高亮
    await page.click('.tab[data-mode="annot"]');
    const box = await (await page.$('.page')).boundingBox();
    await page.mouse.move(box.x + 70, box.y + 105);
    await page.mouse.down();
    await page.mouse.move(box.x + 420, box.y + 130, { steps: 10 });
    await page.mouse.up();
    await sleep(200);

    // 再加一个矩形框
    await page.click('[data-tool="box"]');
    await page.mouse.move(box.x + 60, box.y + 175);
    await page.mouse.down();
    await page.mouse.move(box.x + 430, box.y + 250, { steps: 10 });
    await page.mouse.up();
    await sleep(200);

    await page.screenshot({ path: path.join(OUT, 'form-annots.png') });

    // 水印
    await page.click('.tab[data-mode="watermark"]');
    await page.click('#btnWmToggle');
    await sleep(300);
    await page.screenshot({ path: path.join(OUT, 'form-watermark.png') });

    const meta = await page.evaluate(() => document.querySelector('#meta').textContent);
    console.log('[form] 状态栏:', meta);

    await page.$eval('#btnSave', (b) => b.click());
    await grabPdf(page, 'form-out.pdf');
    await browser.close();
  }

  /* ============ 用例 2：页面操作 ============ */
  {
    const { browser, page } = await boot();
    await loadFile(page, path.join(BASE, 'multi.pdf'));

    const clickBar = (idx, btnIdx) => page.evaluate(({ idx, btnIdx }) => {
      const bars = document.querySelectorAll('.pbar');
      bars[idx].querySelectorAll('button')[btnIdx].click();
    }, { idx, btnIdx });

    let n = await page.evaluate(() => document.querySelectorAll('.pw').length);
    console.log('[multi] 初始页数', n);

    // 删除第 2 页（按钮顺序: 0=⟲ 1=⟳ 2=↑ 3=↓ 4=删除）
    await clickBar(1, 4);
    await sleep(300);

    // 把第 3 页上移到第一位
    await clickBar(2, 2);
    await sleep(400);

    // 旋转当前第一页
    await clickBar(0, 1);
    await page.waitForFunction(() => (document.querySelector('#stat')?.textContent || '').includes('已旋转'), { timeout: 60000 });
    await sleep(500);

    const after = await page.evaluate(() => ({
      visible: [...document.querySelectorAll('.pw')].filter((e) => e.style.display !== 'none').length,
      meta: document.querySelector('#meta').textContent,
      visibleLabels: [...document.querySelectorAll('.pw')]
        .filter((e) => e.style.display !== 'none')
        .map((e) => e.querySelector('.pbar .num').textContent),
      firstSize: (() => {
        const pg = [...document.querySelectorAll('.pw')].find((e) => e.style.display !== 'none').querySelector('.page');
        return pg.style.width + ' x ' + pg.style.height;
      })(),
    }));
    console.log('[multi] 操作后:', JSON.stringify(after));

    await page.screenshot({ path: path.join(OUT, 'multi-pages.png') });

    await page.$eval('#btnSave', (b) => b.click());
    await grabPdf(page, 'multi-out.pdf');
    await browser.close();
  }
})().catch((e) => { console.error('测试失败:', e.message); process.exit(1); });
