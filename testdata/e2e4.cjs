/* 验证「并入整段」：段落边界识别、宽度撑开、撤销恢复、导出 */
const puppeteer = require('puppeteer-core');
const path = require('path');
const fs = require('fs');

const CHROME = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const BASE = 'C:/Users/yhpc2/Desktop/3D Design/pdf-editor/testdata';
const OUT = path.join(BASE, 'shots');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

if (!fs.existsSync(OUT)) fs.mkdirSync(OUT, { recursive: true });

async function boot() {
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-gpu'],
    defaultViewport: { width: 1280, height: 1000 },
    protocolTimeout: 180000,
  });
  const page = await browser.newPage();
  await page.evaluateOnNewDocument(() => {
    const orig = URL.createObjectURL.bind(URL);
    window.__blobs = [];
    URL.createObjectURL = function (b) { try { window.__blobs.push(b); } catch { } return orig(b); };
  });
  page.on('pageerror', (e) => console.log('[pageerror]', e.message));
  page.on('console', (m) => {
    if (m.type() === 'error' && !m.text().includes('favicon')) console.log('[err]', m.text().slice(0, 200));
  });
  await page.goto('http://127.0.0.1:17800', { waitUntil: 'networkidle0' });
  return { browser, page };
}

/// 第一个可见页上所有文字块的几何与文本
async function snap(page) {
  return page.evaluate(() => {
    const rec = [...document.querySelectorAll('.pw')].find((e) => e.style.display !== 'none');
    return [...rec.querySelectorAll('.txt')].map((d) => ({
      t: d.textContent.replace(/\s+/g, '').slice(0, 16),
      top: Math.round(parseFloat(d.style.top)),
      left: Math.round(parseFloat(d.style.left)),
      w: Math.round(parseFloat(d.style.width)),
      h: Math.round(parseFloat(d.style.height)),
      hid: d.style.display === 'none',
      mv: d.dataset.mergedCount || '0',
    }));
  });
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
  const { browser, page } = await boot();
  await (await page.$('#fileInput')).uploadFile(path.join(BASE, 'paras.pdf'));
  await page.waitForFunction(
    () => (document.querySelector('#stat')?.textContent || '').includes('已加载'),
    { timeout: 90000 }
  );
  await sleep(900);

  const rows = await snap(page);
  console.log('[初始] 文字块', rows.length, '个');
  rows.forEach((r, i) => console.log(`  ${i}: left=${r.left} w=${r.w} h=${r.h} "${r.t}"`));

  // 编辑器绑的是 mousedown（不是 click），要在页面里派发真实鼠标事件
  const clickIdx = async (i) => {
    await page.evaluate((k) => {
      const rec = [...document.querySelectorAll('.pw')].find((e) => e.style.display !== 'none');
      const t = [...rec.querySelectorAll('.txt')][k];
      const r = t.getBoundingClientRect();
      t.dispatchEvent(new MouseEvent('mousedown', {
        bubbles: true, cancelable: true, view: window, button: 0,
        clientX: r.left + 10, clientY: r.top + r.height / 2,
      }));
    }, i);
    await sleep(300);
  };

  /* ---------- 用例 1：首段首行 → 并入整段 ---------- */
  console.log('\n--- 用例1：首段首行 → 并入整段（应在段末停） ---');
  await clickIdx(1);
  console.log('点中:', await page.evaluate(() => document.querySelector('#stat').textContent));
  await page.evaluate(() => document.querySelector('#btnMergePara').click());
  await sleep(600);
  console.log('结果:', await page.evaluate(() => document.querySelector('#stat').textContent));

  let now = await snap(page);
  const m1 = now.find((r) => r.mv !== '0');
  const hid1 = now.filter((r) => r.hid);
  console.log(`  合并块: h=${m1?.h} w=${m1?.w} 吞了 ${m1?.mv} 行`);
  console.log(`  隐藏:`, hid1.map((r) => r.t.slice(0, 10)).join(' | '));
  console.log(`  合并块内容: "${m1?.t}…"`);
  const after1 = now[now.indexOf(m1) + 1];
  console.log(`  紧随其后: "${after1?.t}"  ← 应是第二段开头`);

  await page.screenshot({ path: path.join(OUT, 'para-merged.png') });

  /* ---------- 用例 2：撤销后完全还原 ---------- */
  console.log('\n--- 用例2：撤销文字改动 ---');
  await page.evaluate(() => document.querySelector('#btnResetText').click());
  await sleep(500);
  const back = await snap(page);
  let same = back.length === rows.length;
  if (same) {
    for (let i = 0; i < rows.length; i++) {
      if (back[i].top !== rows[i].top || back[i].w !== rows[i].w || back[i].h !== rows[i].h || back[i].hid) {
        same = false;
        console.log(`  ✗ 第${i}块: 原 top=${rows[i].top} w=${rows[i].w} h=${rows[i].h} → 现 top=${back[i].top} w=${back[i].w} h=${back[i].h} hid=${back[i].hid}`);
      }
    }
  }
  console.log('  还原:', same ? '与初始完全一致 ✓' : '✗ 有差异');
  console.log('  隐藏块数:', back.filter((r) => r.hid).length);

  /* ---------- 用例 3：从段中间的行并入，仍应止于段尾 ---------- */
  console.log('\n--- 用例3：第二段第 2 行 → 并入整段 ---');
  await clickIdx(6);
  console.log('点中:', await page.evaluate(() => document.querySelector('#stat').textContent));
  await page.evaluate(() => document.querySelector('#btnMergePara').click());
  await sleep(600);
  console.log('结果:', await page.evaluate(() => document.querySelector('#stat').textContent));

  now = await snap(page);
  const m3 = now.find((r) => r.mv !== '0');
  const hid3 = now.filter((r) => r.hid);
  console.log(`  合并块 h=${m3?.h} 吞了 ${m3?.mv} 行`);
  console.log(`  隐藏:`, hid3.map((r) => r.t.slice(0, 10)).join(' | '));
  console.log(`  紧随其后: "${now[now.indexOf(m3) + 1]?.t}"  ← 应是第三段开头`);

  /* ---------- 用例 4：整段编辑 + 导出 ---------- */
  console.log('\n--- 用例4：整段改写并导出 ---');
  await page.evaluate(() => {
    const rec = [...document.querySelectorAll('.pw')].find((e) => e.style.display !== 'none');
    const m = [...rec.querySelectorAll('.txt')].find((d) => d.dataset.mergedCount);
    const r = m.getBoundingClientRect();
    m.dispatchEvent(new MouseEvent('mousedown', {
      bubbles: true, cancelable: true, view: window, button: 0,
      clientX: r.left + 10, clientY: r.top + 10,
    }));
  });
  await sleep(500);
  console.log('  进入编辑:', await page.evaluate(() => !!document.querySelector('.txt.focus')));

  await page.evaluate(() => {
    const el = document.querySelector('.txt.focus');
    if (!el) return;
    el.innerText = '（本段已整体改写）第二条 设备管理。办公终端应当安装统一的防护软件并及时更新病毒库，禁止私自拆卸或改装设备，禁止安装来源不明的软件。';
  });
  await sleep(200);
  // 点到页面空白处结束编辑（走的也是 mousedown）
  await page.evaluate(() => {
    const pg = [...document.querySelectorAll('.page')].find((e) => e.getBoundingClientRect().height > 0);
    const r = pg.getBoundingClientRect();
    pg.dispatchEvent(new MouseEvent('mousedown', {
      bubbles: true, cancelable: true, view: window, button: 0,
      clientX: r.left + 20, clientY: r.bottom - 8,
    }));
  });
  await sleep(600);
  await page.screenshot({ path: path.join(OUT, 'para-edit.png') });
  console.log('  状态:', await page.evaluate(() => document.querySelector('#stat').textContent));
  console.log('  改动数:', await page.evaluate(() => document.querySelectorAll('.txt.dirty').length));

  await page.evaluate(() => document.querySelector('#btnSave').click());
  await grabPdf(page, 'para-out.pdf');

  await browser.close();
})();
