'use strict';

// ============================================================
// 基本工具
// ============================================================
const invoke = (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) ||
  (() => Promise.reject('請從桌面程式開啟（目前不是在 Tauri 視窗內）'));

const $ = (sel, el = document) => el.querySelector(sel);
const $$ = (sel, el = document) => Array.from(el.querySelectorAll(sel));

function esc(v) {
  return String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

async function call(cmd, args) {
  try {
    return await invoke(cmd, args || {});
  } catch (e) {
    toast(String(e), 'error');
    throw e;
  }
}

function toast(msg, type) {
  const el = document.createElement('div');
  el.className = 'toast' + (type === 'error' ? ' error' : '');
  el.textContent = msg;
  $('#toast').appendChild(el);
  setTimeout(() => el.remove(), type === 'error' ? 6000 : 3000);
}

function confirmBox(msg, okText = '確定') {
  return new Promise((resolve) => {
    const back = $('#confirm-back');
    $('#confirm-msg').textContent = msg;
    $('#confirm-ok').textContent = okText;
    back.classList.add('open');
    const done = (v) => {
      back.classList.remove('open');
      $('#confirm-ok').onclick = null;
      $('#confirm-cancel').onclick = null;
      resolve(v);
    };
    $('#confirm-ok').onclick = () => done(true);
    $('#confirm-cancel').onclick = () => done(false);
    $('#confirm-ok').focus();
  });
}

const pad2 = (n) => String(n).padStart(2, '0');
function today() {
  const d = new Date();
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}
function thisMonth() { return today().slice(0, 7); }
function monthRange(ym) {
  const [y, m] = ym.split('-').map(Number);
  const last = new Date(y, m, 0).getDate();
  return [`${ym}-01`, `${ym}-${pad2(last)}`];
}
function slashDate(d) { return (d || '').replace(/-/g, '/'); }

function fmtInt(n) {
  const v = Math.round(Number(n) || 0);
  return (v < 0 ? '-' : '') + Math.abs(v).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}
function fmtNum(x) {
  if (x === null || x === undefined || x === '') return '';
  const v = Number(x);
  if (Number.isInteger(v)) return fmtInt(v);
  const [i, f] = v.toFixed(4).replace(/0+$/, '').split('.');
  return fmtInt(Number(i)) + (f ? '.' + f : '');
}
function parseNum(s) {
  const t = String(s ?? '').replace(/[０-９]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xFEE0)).replace(/[,，\s]/g, '').replace('．', '.');
  if (t === '') return null;
  const v = Number(t);
  return Number.isFinite(v) ? v : NaN;
}

// 與 Rust 端相同的斤兩解析
function parseQty(text) {
  let s = String(text ?? '').replace(/[０-９]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xFEE0)).replace('．', '.').replace(/\s|　/g, '');
  if (!s) return null;
  const num = (t) => { const v = Number(t); if (t === '' || !Number.isFinite(v)) throw new Error('bad'); return v; };
  try {
    let v;
    if (s.includes('斤')) {
      const [j, restRaw] = s.split('斤');
      const rest = restRaw.replace(/兩$/, '');
      const jin = j === '' ? 0 : num(j);
      const liang = rest === '' ? 0 : num(rest);
      if (liang >= 16) return NaN;
      v = jin + liang / 16;
    } else if (s.endsWith('兩')) {
      v = num(s.slice(0, -1)) / 16;
    } else {
      v = num(s);
    }
    return v < 0 ? NaN : v;
  } catch { return NaN; }
}

function computeTotals(amounts, taxMode, sign) {
  const sum = amounts.reduce((a, b) => a + b, 0);
  let sub, tax, total;
  if (taxMode === 'inclusive') { sub = Math.round(sum / 1.05); tax = sum - sub; total = sum; }
  else if (taxMode === 'none') { sub = sum; tax = 0; total = sum; }
  else { sub = sum; tax = Math.round(sum * 0.05); total = sum + tax; }
  return { sub: sub * sign, tax: tax * sign, total: total * sign };
}

const TAX_LABEL = { exclusive: '外加 5%', inclusive: '內含 5%', none: '免稅' };
const closingLabel = (d) => (d > 0 ? `每月 ${d} 日` : '月底');

// ============================================================
// 共用資料
// ============================================================
const S = { info: null, settings: null, customers: [], products: [], balances: {} };

function customerLabel(c) { return c.code ? `${c.code} ${c.name}` : c.name; }
function productLabel(p) { return p.code ? `${p.code} ${p.name}` : p.name; }

function findCustomer(text) {
  const t = String(text || '').trim();
  if (!t) return null;
  return S.customers.find((c) => customerLabel(c) === t) ||
    S.customers.find((c) => c.name === t) ||
    S.customers.find((c) => c.code && c.code.toLowerCase() === t.toLowerCase()) || null;
}
function findProduct(text) {
  const t = String(text || '').trim();
  if (!t) return null;
  return S.products.find((p) => productLabel(p) === t) ||
    S.products.find((p) => p.name === t) ||
    S.products.find((p) => p.code && p.code.toLowerCase() === t.toLowerCase()) || null;
}
const customerById = (id) => S.customers.find((c) => c.id === id) || null;

async function loadCustomers() {
  S.customers = await call('list_customers', { includeInactive: true });
  setNavCount('cust', S.customers.filter((c) => c.needs_review).length);
  const bal = await call('customer_balances');
  S.balances = Object.fromEntries(bal.map((b) => [b.customer_id, b.balance]));
}
async function loadProducts() {
  S.products = await call('list_products', { includeInactive: true });
}

// ============================================================
// 客戶／品項選擇器：點進欄位就列出全部，打字才篩選
// （取代瀏覽器內建的提示清單；內建清單在欄位有字時只會列出符合那些字的項目）
// 用法：<input data-combo="customers"> 或 <input data-combo="products">
// 選定後會對欄位送出 input 與 change 事件，原本的處理流程照常運作。
// ============================================================
const Combo = { input: null, items: [], active: -1, typed: false };

function comboSource(kind) {
  if (kind === 'products') {
    return S.products.filter((p) => p.active).map((p) => ({
      label: productLabel(p),
      title: p.name,
      sub: [p.code, p.unit, p.price === null ? '時價' : `$${fmtNum(p.price)}`].filter(Boolean).join('・'),
      keys: [p.name, p.code].join(' ').toLowerCase(),
    }));
  }
  return S.customers.filter((c) => c.active).map((c) => ({
    label: customerLabel(c),
    title: c.name,
    sub: [c.code, c.phone].filter(Boolean).join('・'),
    keys: [c.name, c.code, c.phone].join(' ').toLowerCase(),
  }));
}

function comboRender() {
  const pop = $('#combo-pop');
  const input = Combo.input;
  if (!input) return;
  const kind = input.dataset.combo;
  const all = comboSource(kind);
  const q = input.value.trim().toLowerCase();
  // 剛點進來（還沒打字）一律列出全部；打字後才依名稱、代號、電話篩選
  Combo.items = Combo.typed && q ? all.filter((it) => q.split(/\s+/).every((w) => it.keys.includes(w) || it.label.toLowerCase().includes(w))) : all;
  // 出貨單明細打了清單裡沒有的品名：提供「新增品項」，存檔時自動加入品項清單
  const typedText = input.value.trim();
  if (kind === 'products' && input.dataset.f === 'name' && Combo.typed && typedText &&
      !all.some((it) => it.title === typedText || it.label === typedText)) {
    Combo.items = Combo.items.concat([{ isNew: true, label: typedText, title: `＋ 新增品項「${typedText}」`, sub: '存檔時自動加入品項清單' }]);
  }
  if (Combo.active >= Combo.items.length) Combo.active = Combo.items.length - 1;
  if (!Combo.items.length) {
    const what = kind === 'products' ? '品項' : '客戶';
    pop.innerHTML = `<div class="combo-empty">${all.length ? `找不到符合的${what}` : `還沒有${what}`}${kind === 'products' ? '，可以直接輸入品名' : '，請先到「客戶」新增'}</div>`;
  } else {
    const current = input.value.trim();
    pop.innerHTML = Combo.items.map((it, i) => `<div class="combo-item${i === Combo.active ? ' active' : ''}${!it.isNew && it.label === current ? ' current' : ''}${it.isNew ? ' new' : ''}" role="option" id="combo-opt-${i}" data-i="${i}" aria-selected="${i === Combo.active}">
        <span class="combo-title">${esc(it.title)}</span><span class="combo-sub">${esc(it.sub)}</span></div>`).join('');
  }
  input.setAttribute('aria-activedescendant', Combo.active >= 0 ? `combo-opt-${Combo.active}` : '');
  comboPlace();
  const act = pop.querySelector('.combo-item.active');
  if (act) act.scrollIntoView({ block: 'nearest' });
}

function comboPlace() {
  const pop = $('#combo-pop');
  const input = Combo.input;
  if (!input) return;
  const r = input.getBoundingClientRect();
  const below = window.innerHeight - r.bottom - 12;
  const height = Math.min(320, Math.max(below, r.top - 12));
  pop.style.minWidth = `${Math.max(r.width, 260)}px`;
  pop.style.left = `${Math.min(r.left, window.innerWidth - Math.max(r.width, 260) - 8)}px`;
  pop.style.maxHeight = `${height}px`;
  if (below >= 160 || below >= r.top) { pop.style.top = `${r.bottom + 4}px`; pop.style.bottom = ''; }
  else { pop.style.top = ''; pop.style.bottom = `${window.innerHeight - r.top + 4}px`; }
}

function comboOpen(input) {
  Combo.input = input;
  Combo.typed = false;
  const all = comboSource(input.dataset.combo);
  Combo.active = all.findIndex((it) => it.label === input.value.trim());
  $('#combo-pop').hidden = false;
  input.setAttribute('aria-expanded', 'true');
  comboRender();
}

function comboClose() {
  if (Combo.input) Combo.input.setAttribute('aria-expanded', 'false');
  Combo.input = null;
  $('#combo-pop').hidden = true;
}

function comboPick(i) {
  const input = Combo.input;
  const it = Combo.items[i];
  if (!input || !it) return;
  comboClose();
  input.value = it.label;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
  // 出貨單明細選好品項後，游標直接跳到數量
  if (input.dataset.f === 'name') input.closest('tr')?.querySelector('[data-f="qty_text"]')?.focus();
}

document.addEventListener('focusin', (e) => {
  const t = e.target;
  if (t.matches && t.matches('input[data-combo]') && !t.disabled) {
    comboOpen(t);
    t.select();
  }
});
document.addEventListener('focusout', (e) => {
  if (e.target === Combo.input) setTimeout(() => { if (document.activeElement !== Combo.input) comboClose(); }, 0);
});
document.addEventListener('input', (e) => {
  if (e.target !== Combo.input || !e.isTrusted) return;
  Combo.typed = true;
  Combo.active = e.target.value.trim() ? 0 : -1;
  comboRender();
});
// 用 capture：選擇器開著時，↑↓、Enter、Esc 先由選擇器處理，不會觸發明細表的「Enter 跳下一格」
document.addEventListener('keydown', (e) => {
  const input = e.target;
  if (!input.matches || !input.matches('input[data-combo]')) return;
  if (Combo.input !== input) {
    if (e.key === 'ArrowDown') { e.preventDefault(); comboOpen(input); }
    return;
  }
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    const n = Combo.items.length;
    if (!n) return;
    Combo.active = e.key === 'ArrowDown' ? (Combo.active + 1) % n : (Combo.active - 1 + n) % n;
    comboRender();
  } else if (e.key === 'Enter') {
    if (Combo.active >= 0 && Combo.items[Combo.active] && (Combo.typed || Combo.items[Combo.active].label !== input.value.trim())) {
      e.preventDefault();
      e.stopPropagation();
      comboPick(Combo.active);
    } else {
      comboClose();
    }
  } else if (e.key === 'Escape') {
    e.preventDefault();
    comboClose();
  } else if (e.key === 'Tab') {
    comboClose();
  }
}, true);
$('#combo-pop').addEventListener('mousedown', (e) => {
  e.preventDefault(); // 保持游標在欄位裡
  const item = e.target.closest('.combo-item');
  if (item) comboPick(Number(item.dataset.i));
});
window.addEventListener('resize', () => { if (Combo.input) comboPlace(); });
document.addEventListener('scroll', () => { if (Combo.input) comboPlace(); }, true);

// ============================================================
// 手持機同步：設定、配對、同步提醒、新客戶確認與合併
// ============================================================
function setNavCount(view, n) {
  const b = $(`#nav button[data-view="${view}"]`);
  if (!b) return;
  let el = b.querySelector('.nav-count');
  if (!n) { if (el) el.remove(); return; }
  if (!el) { el = document.createElement('span'); el.className = 'nav-count'; b.appendChild(el); }
  el.textContent = n;
}

function fmtTime(t) { return t ? t.replace(/^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}).*/, '$2/$3 $4:$5') : '從未同步'; }

async function renderSyncCard() {
  let st;
  try { st = await invoke('sync_status'); } catch (e) { $('#sync-status').textContent = String(e); return; }
  const box = $('#sync-status');
  if (!st.enabled) {
    box.innerHTML = '<p class="hint">同步服務已關閉，手持機無法連線。</p>';
  } else if (st.error) {
    box.innerHTML = `<div class="banner warn">${esc(st.error)}<br>可能是這個連接埠被其他程式使用，請換一個連接埠後儲存設定。</div>`;
  } else {
    const addrs = st.addresses.map((a) => `<div><span class="addr">${esc(a.ip)}:${st.port}</span>　<span class="hint">${esc(a.kind)}</span></div>`).join('');
    box.innerHTML = `<p class="hint" style="color:var(--accent-dark);font-weight:600">● 運作中</p>
      <p class="hint" style="margin-top:6px">手持機要連到這台電腦的位址：</p>${addrs || '<p class="hint">找不到網路位址，請確認電腦有連上網路。</p>'}
      ${st.addresses.some((a) => a.kind === 'Tailscale') ? '' : '<p class="hint warn-text" style="margin-top:6px">沒有偵測到 Tailscale，手持機外出時會連不到。請先安裝並登入 Tailscale。</p>'}`;
  }
  const devices = await call('list_devices');
  $('#device-list').innerHTML = devices.length
    ? devices.map((d) => `<div class="list-item">
        <div><div style="font-weight:600">${esc(d.name)} <span class="badge hh">${esc(d.prefix)}</span>${d.revoked ? ' <span class="badge">已停用</span>' : ''}</div>
          <div class="hint">上次同步：${esc(fmtTime(d.last_sync_at))}</div></div>
        ${d.revoked ? '' : `<button type="button" class="btn small danger" data-revoke="${d.id}" data-name="${esc(d.name)}">停用</button>`}
      </div>`).join('')
    : '<p class="hint">還沒有配對任何手持機。</p>';
}

$('#device-list').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-revoke]');
  if (!b) return;
  if (!(await confirmBox(`停用「${b.dataset.name}」？\n停用後這台手持機就不能再同步。手持機遺失或換機時使用。\n已同步上來的單據不受影響。`, '停用'))) return;
  await call('revoke_device', { id: Number(b.dataset.revoke) });
  toast(`已停用 ${b.dataset.name}`);
  renderSyncCard();
});

let pairTimer = null;
$('#btn-pair').addEventListener('click', async () => {
  await saveSettings(true);
  const st = await call('sync_status');
  if (!st.enabled || !st.running) { toast('同步服務沒有在運作，請先啟用並儲存設定', 'error'); return; }
  const pc = await call('create_pairing_code');
  const before = (await call('list_devices')).length;
  const addrs = st.addresses.map((a) => `<div><span class="addr">${esc(a.ip)}:${st.port}</span>　<span class="hint">${esc(a.kind)}</span></div>`).join('');
  $('#pair-body').innerHTML = `
    <p style="margin:0 0 10px">在手持機的 App 裡選「連線到電腦」，輸入電腦位址和這組配對碼：</p>
    <div class="pair-code">${esc(pc.code)}</div>
    <p class="hint" style="text-align:center;margin-top:8px">這台手持機的單號字頭會是 <b>${esc(pc.prefix)}</b>（例如 ${esc(pc.prefix)}-202610-0001）。配對碼 10 分鐘內有效、只能用一次。</p>
    <div style="margin-top:14px"><p class="hint" style="margin-bottom:4px">電腦位址（外出時用 Tailscale 那一個）：</p>${addrs}</div>
    <p id="pair-wait" class="hint" style="margin-top:14px">等待手持機配對中…</p>`;
  $('#pair-back').classList.add('open');
  clearInterval(pairTimer);
  pairTimer = setInterval(async () => {
    const list = await invoke('list_devices').catch(() => []);
    if (list.length > before) {
      clearInterval(pairTimer);
      const d = list.find((x) => x.prefix === pc.prefix) || list[list.length - 1];
      $('#pair-wait').innerHTML = `<span style="color:var(--accent-dark);font-weight:700">✓ 已配對：${esc(d.name)}（${esc(d.prefix)}）</span>`;
      renderSyncCard();
    }
  }, 2000);
});
$('#pair-close').addEventListener('click', () => { clearInterval(pairTimer); $('#pair-back').classList.remove('open'); renderSyncCard(); });

const CONFLICT_LABEL = {
  both_edited: '兩邊都修改了草稿',
  pc_overwritten: '手持機版本已覆蓋電腦的修改',
  locked: '已列入對帳單，手持機修改未套用',
  deleted_on_pc: '電腦上刪除過的單',
  delete_rejected: '手持機刪除未執行',
  late_period: '沒有列入已建立的對帳單',
};

async function renderSyncBanner() {
  let list = [];
  try { list = await invoke('list_sync_conflicts'); } catch { return; }
  setNavCount('ship', list.length);
  $('#sync-banner').innerHTML = list.length
    ? `<div class="banner warn" style="display:flex;justify-content:space-between;align-items:center;gap:12px;flex-wrap:wrap">
        <span>手持機同步有 <b>${list.length}</b> 筆需要你看一下</span>
        <button type="button" class="btn small" id="btn-conflicts">查看</button></div>`
    : '';
  if (list.length) $('#btn-conflicts').onclick = openConflicts;
}

async function openConflicts() {
  const list = await call('list_sync_conflicts');
  const docs = await call('list_documents', { filter: { customer_id: null, from: null, to: null, limit: 2000 } });
  const byNo = Object.fromEntries(docs.map((d) => [d.doc_no, d.id]));
  $('#conflict-list').innerHTML = list.length ? list.map((c) => {
    let detail = '';
    if (c.payload) {
      try {
        const w = JSON.parse(c.payload);
        const label = c.kind === 'pc_overwritten' ? '電腦原本的版本' : '手持機的版本';
        detail = `<details><summary class="hint" style="cursor:pointer">${label}：${w.lines.length} 項${w.total ? `，合計 ${fmtInt(w.total)}` : ''}</summary>
          <div class="hint" style="margin-top:6px">${w.lines.map((l) => `${esc(l.name)}　${esc(l.qty_text)} × ${fmtNum(l.price)}`).join('<br>')}</div></details>`;
      } catch { /* 略過 */ }
    }
    const id = byNo[c.doc_no];
    return `<div class="conflict-item">
      <div class="meta"><b>${esc(c.doc_no || '（無單號）')}</b><span class="badge hh">手持機 ${esc(c.device_prefix)}</span><span class="badge review">${esc(CONFLICT_LABEL[c.kind] || c.kind)}</span><span class="hint">${esc(fmtTime(c.created_at))}</span></div>
      <div>${esc(c.message)}</div>${detail}
      <div class="actions">
        ${id ? `<button type="button" class="btn small" data-open-doc="${id}">開啟單據</button>` : ''}
        ${c.can_use_handheld
          ? `<button type="button" class="btn small" data-resolve="${c.id}" data-action="dismiss">保留電腦版本</button>
             <button type="button" class="btn small primary" data-resolve="${c.id}" data-action="use_handheld">改用手持機版本</button>`
          : `<button type="button" class="btn small primary" data-resolve="${c.id}" data-action="dismiss">知道了</button>`}
      </div></div>`;
  }).join('') : '<p class="hint">目前沒有需要處理的項目。</p>';
  $('#conflict-back').classList.add('open');
}
$('#conflict-close').addEventListener('click', () => { $('#conflict-back').classList.remove('open'); renderSyncBanner(); if (currentView === 'ship') renderShipList(); });
$('#conflict-list').addEventListener('click', async (e) => {
  const open = e.target.closest('[data-open-doc]');
  if (open) { $('#conflict-back').classList.remove('open'); openDoc(Number(open.dataset.openDoc)); return; }
  const b = e.target.closest('[data-resolve]');
  if (!b) return;
  if (b.dataset.action === 'use_handheld' && !(await confirmBox('改用手持機的版本？電腦上這張單目前的內容會被取代。', '改用手持機版本'))) return;
  await call('resolve_sync_conflict', { id: Number(b.dataset.resolve), action: b.dataset.action });
  toast(b.dataset.action === 'use_handheld' ? '已改用手持機版本' : '已處理');
  openConflicts();
  renderSyncBanner();
});

// 手持機新增的客戶：確認無誤，或合併到既有客戶
async function renderCustReview(c) {
  const box = $('#c-review');
  if (!c || !c.needs_review) { box.hidden = true; box.innerHTML = ''; return; }
  const dups = await call('possible_duplicates', { customerId: c.id });
  box.hidden = false;
  box.innerHTML = `<div class="review-box">
    <div><b>手持機 ${esc(c.origin)} 新增的客戶</b>，請確認資料，或合併到既有客戶。</div>
    ${dups.length ? `<div class="warn-text">可能跟這些客戶重複（名稱或電話相同）：</div>
      ${dups.map(([id, name]) => `<div style="display:flex;justify-content:space-between;align-items:center;gap:8px"><span>${esc(name)}</span>
        <button type="button" class="btn small" data-merge-to="${id}" data-merge-name="${esc(name)}">合併到這位</button></div>`).join('')}` : ''}
    <div style="display:flex;gap:8px;align-items:center">
      <input type="text" id="c-merge-target" data-combo="customers" autocomplete="off" placeholder="合併到其他客戶…">
      <button type="button" class="btn small" id="c-merge-pick">合併</button>
    </div>
    <div class="actions"><button type="button" class="btn small primary" id="c-confirm">資料正確，確認</button></div>
  </div>`;
  const merge = async (toId, toName) => {
    if (toId === c.id) { toast('不能合併到自己', 'error'); return; }
    if (!(await confirmBox(`把「${c.name}」合併到「${toName}」？\n「${c.name}」的出貨單、收款、對帳單都會移到「${toName}」，然後刪除「${c.name}」。\n手持機下次同步也會跟著更新。這個動作無法復原。`, '合併'))) return;
    await call('merge_customers', { fromId: c.id, toId });
    await loadCustomers();
    toast(`已合併到 ${toName}`);
    fillCustForm(customerById(toId));
  };
  box.querySelectorAll('[data-merge-to]').forEach((b) => { b.onclick = () => merge(Number(b.dataset.mergeTo), b.dataset.mergeName); });
  $('#c-merge-pick').onclick = () => {
    const t = findCustomer($('#c-merge-target').value);
    if (!t) { toast('請先選擇要合併到哪位客戶', 'error'); return; }
    merge(t.id, t.name);
  };
  $('#c-confirm').onclick = async () => {
    await call('confirm_customer', { id: c.id });
    await loadCustomers();
    toast(`已確認 ${c.name}`);
    fillCustForm(customerById(c.id));
  };
}

// 手持機隨時可能同步：定時更新提醒與清單（只在不會打斷輸入的畫面）
setInterval(async () => {
  if (document.querySelector('.modal-back.open')) return;
  // 客戶、品項清單（選擇器也會用到）、待確認數量
  await Promise.all([loadCustomers(), loadProducts()]).catch(() => {});
  await renderSyncBanner();
  if (currentView === 'ship') renderShipList();
  if (currentView === 'cust') renderCustList();
  if (currentView === 'prod') renderProdList();
}, 30000);

// ============================================================
// 畫面切換
// ============================================================
let currentView = 'ship';

async function showView(name) {
  if (currentView === 'ship-edit' && name !== 'ship-edit' && E.dirty) {
    if (!(await confirmBox('出貨單還沒存檔，確定要離開嗎？', '離開不存'))) return false;
    E.dirty = false;
  }
  if (currentView === 'man-edit' && name !== 'man-edit' && M.dirty) {
    if (!(await confirmBox('對帳單還沒存檔，確定要離開嗎？', '離開不存'))) return false;
    M.dirty = false;
  }
  currentView = name;
  $$('.view').forEach((v) => v.classList.remove('active'));
  const id = name === 'ship' ? 'view-ship-list' : name === 'ship-edit' ? 'view-ship-edit' : 'view-' + name;
  $('#' + id).classList.add('active');
  const navName = name.startsWith('ship') ? 'ship' : name.startsWith('man') ? 'man' : name;
  $$('#nav button').forEach((b) => b.classList.toggle('active', b.dataset.view === navName));
  $('#main').scrollTop = 0;
  if (name === 'ship') renderShipList();
  if (name === 'stmt') stmtRefresh();
  if (name === 'man') renderManList();
  // 手持機可能新增了客戶或品項：進到這兩頁時重新讀取
  if (name === 'prod') { await loadProducts(); renderProdList(); }
  if (name === 'cust') { await loadCustomers(); renderCustList(); }
  if (name === 'set') renderSettings();
  return true;
}

// ============================================================
// 出貨單列表
// ============================================================
async function renderShipList() {
  renderSyncBanner();
  const cust = findCustomer($('#ship-f-cust').value);
  const ym = $('#ship-f-month').value;
  const filter = { customer_id: cust ? cust.id : null, from: null, to: null, limit: 500 };
  if (ym) [filter.from, filter.to] = monthRange(ym);
  const docs = await call('list_documents', { filter });
  const box = $('#ship-list');
  if (!docs.length) {
    box.innerHTML = '<div class="empty">這段期間沒有單據。按右上角「新增出貨單」開始。</div>';
    return;
  }
  box.innerHTML = `<table class="grid"><thead><tr>
      <th>日期</th><th>單號</th><th>客戶</th><th class="num">品項</th><th class="num">金額</th><th>狀態</th></tr></thead><tbody>
    ${docs.map((d) => `<tr class="click" data-id="${d.id}">
      <td>${esc(slashDate(d.doc_date))}</td>
      <td>${esc(d.doc_no)} ${d.kind === 'adjustment' ? '<span class="badge adj">退貨／折讓</span>' : ''}${d.origin ? ` <span class="badge hh">手持機 ${esc(d.origin)}</span>` : ''}</td>
      <td>${esc(d.cust_name)}</td>
      <td class="num">${d.item_count}</td>
      <td class="num">${fmtInt(d.total)}</td>
      <td>${d.statement_id ? '<span class="badge locked">已對帳</span>' : d.status === 'draft' ? '<span class="badge draft">草稿</span>' : '<span class="badge">已開立</span>'}</td>
    </tr>`).join('')}</tbody></table>`;
}

$('#ship-list').addEventListener('click', (e) => {
  const tr = e.target.closest('tr[data-id]');
  if (tr) openDoc(Number(tr.dataset.id));
});
$('#ship-f-cust').addEventListener('change', renderShipList);
$('#ship-f-month').addEventListener('change', renderShipList);
$('#btn-new-ship').addEventListener('click', () => newDoc('shipment'));
$('#btn-new-adj').addEventListener('click', () => newDoc('adjustment'));

// ============================================================
// 出貨單編輯
// ============================================================
const E = { doc: null, dirty: false };

function blankLine() {
  return { id: null, product_id: null, name: '', unit: '', qty_text: '', qty: 0, price: null, amount: 0, note: '', _hint: '' };
}

async function newDoc(kind) {
  if (!(await showView('ship-edit'))) return;
  const preset = findCustomer($('#ship-f-cust').value);
  E.doc = {
    id: null, doc_no: '', kind, customer_id: preset ? preset.id : null, doc_date: today(),
    maker: S.settings.default_maker || '', note: '', status: 'draft', statement_id: null, statement_no: null,
    cust_name: '', cust_phone: '', cust_address: '', tax_mode: preset ? preset.tax_mode : 'exclusive',
    subtotal: 0, tax: 0, total: 0, lines: [blankLine()],
  };
  E.dirty = false;
  renderEditor();
  setTimeout(() => $(E.doc.customer_id ? '[data-f="name"]' : '#ed-cust')?.focus(), 0);
}

async function openDoc(id) {
  const doc = await call('get_document', { id });
  if (!(await showView('ship-edit'))) return;
  // 草稿裡還沒定價的品項（存成 0）顯示成空白，提醒要補價格
  doc.lines = doc.lines.map((l) => ({ ...l, _hint: '', price: doc.status === 'draft' && l.price === 0 ? null : l.price }));
  if (!doc.statement_id) doc.lines.push(blankLine());
  E.doc = doc;
  E.dirty = false;
  renderEditor();
}

const locked = () => !!(E.doc && E.doc.statement_id);

function renderEditor() {
  const d = E.doc;
  const isAdj = d.kind === 'adjustment';
  const title = d.id ? `${isAdj ? '退貨／折讓' : '出貨單'} ${d.doc_no}` : isAdj ? '新增退貨／折讓' : '新增出貨單';
  const cust = customerById(d.customer_id);
  const dis = locked() ? 'disabled' : '';
  $('#view-ship-edit').innerHTML = `
    <div class="head">
      <div style="display:flex;align-items:center;gap:12px;flex-wrap:wrap">
        <button type="button" class="btn" id="ed-back">‹ 返回列表</button>
        <h1>${esc(title)}</h1>
        ${d.id ? (d.statement_id ? '<span class="badge locked">已對帳</span>' : d.status === 'draft' ? '<span class="badge draft">草稿</span>' : '<span class="badge">已開立</span>') : ''}
      </div>
      <div class="actions">
        ${d.id && !locked() ? '<button type="button" class="btn danger" id="ed-delete">刪除</button>' : ''}
        ${!locked() ? '<button type="button" class="btn" id="ed-draft">存草稿</button><button type="button" class="btn" id="ed-save">存檔 Ctrl+S</button>' : ''}
        <button type="button" class="btn" id="ed-preview">預覽</button>
        <button type="button" class="btn primary" id="ed-print">列印 F9</button>
      </div>
    </div>
    ${locked() ? `<div class="banner">這張單已列入對帳單 ${esc(d.statement_no || '')}，不能修改。要更正請開退貨／折讓單，或到對帳單頁作廢該對帳單。</div>` : ''}
    ${isAdj ? '<div class="banner warn">退貨／折讓單的金額會從客戶應收中扣除，品項請填正數。</div>' : ''}
    <div class="card pad">
      <div class="fields">
        <label class="f span2"><span class="lbl">客戶（輸入名稱或代號）</span>
          <input type="text" id="ed-cust" data-combo="customers" autocomplete="off" value="${esc(cust ? customerLabel(cust) : d.cust_name)}" ${dis}>
          <span class="info-line" id="ed-cust-info"></span></label>
        <label class="f"><span class="lbl">${isAdj ? '日期' : '出貨日期'}</span><input type="date" id="ed-date" value="${esc(d.doc_date)}" ${dis}></label>
        <label class="f"><span class="lbl">製表人員</span><input type="text" id="ed-maker" value="${esc(d.maker)}" ${dis}></label>
      </div>
    </div>
    <div class="row">
      <div class="grow card">
        <div class="table-wrap">
          <table class="grid lines" id="ed-lines">
            <thead><tr><th>#</th><th class="c-name">品名</th><th class="c-qty">數量</th><th class="c-unit">單位</th>
              <th class="c-price" style="text-align:right">單價</th><th class="c-amt">金額</th><th class="c-note">訂貨備註</th><th class="c-del"></th></tr></thead>
            <tbody></tbody>
          </table>
        </div>
        <div class="totals-bar">
          <span class="hint" id="ed-count"></span>
          <div class="nums">
            <span class="hint">未稅 <b id="ed-sub"></b></span>
            <span class="hint" id="ed-tax-label">稅金 <b id="ed-tax"></b></span>
            <span class="grand">合計 <span id="ed-total"></span></span>
          </div>
        </div>
      </div>
      <aside class="side card pad">
        <h2>此客戶最近的單</h2>
        <div id="ed-recent"><p class="hint">選擇客戶後顯示。</p></div>
        <p class="hint" style="margin-top:12px">單價依序帶入：客戶專屬價、此客戶上次成交價、品項預設價。秤重品項可輸入「2斤5兩」。</p>
      </aside>
    </div>`;
  renderLines();
  updateCustInfo();
  bindEditor();
}

function lineRowHtml(l, i) {
  const dis = locked() ? 'disabled' : '';
  const q = parseQty(l.qty_text);
  const qtyBad = l.qty_text && Number.isNaN(q);
  const prod = l.product_id ? S.products.find((p) => p.id === l.product_id) : null;
  const qtyHint = q !== null && !Number.isNaN(q) && /[斤兩]/.test(l.qty_text) ? `＝ ${fmtNum(q)} 斤` : '';
  const nameVal = prod && prod.name === l.name ? productLabel(prod) : l.name;
  return `<tr data-i="${i}">
    <td>${l.name || l.qty_text ? i + 1 : ''}</td>
    <td class="c-name"><input type="text" data-f="name" data-combo="products" autocomplete="off" value="${esc(nameVal)}" ${dis} aria-label="第 ${i + 1} 列品名"></td>
    <td class="c-qty"><input type="text" data-f="qty_text" value="${esc(l.qty_text)}" class="${qtyBad ? 'invalid' : ''}" ${dis} aria-label="數量"><div class="qty-hint">${esc(qtyHint)}</div></td>
    <td class="c-unit"><input type="text" data-f="unit" value="${esc(l.unit)}" ${dis} aria-label="單位"></td>
    <td class="c-price"><input type="text" data-f="price" value="${l.price === null || l.price === undefined ? '' : esc(l.price)}" style="text-align:right" ${dis} aria-label="單價"><div class="price-hint">${esc(l._hint || '')}</div></td>
    <td class="c-amt" data-amt>${lineAmountText(l)}</td>
    <td class="c-note"><input type="text" data-f="note" value="${esc(l.note)}" ${dis} aria-label="備註"></td>
    <td class="c-del">${!locked() && (l.name || l.qty_text) ? `<button type="button" class="icon-btn" data-del aria-label="刪除第 ${i + 1} 列"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg></button>` : ''}</td>
  </tr>`;
}

function lineAmount(l) {
  const q = parseQty(l.qty_text);
  const p = l.price === null || l.price === undefined || l.price === '' ? null : Number(l.price);
  if (q === null || Number.isNaN(q) || p === null || Number.isNaN(p)) return null;
  return Math.round(q * p);
}
function lineAmountText(l) {
  const a = lineAmount(l);
  return a === null ? '' : fmtInt(a);
}

function renderLines() {
  $('#ed-lines tbody').innerHTML = E.doc.lines.map(lineRowHtml).join('');
  updateTotals();
}

function ensureTrailingBlank() {
  const ls = E.doc.lines;
  const last = ls[ls.length - 1];
  if (!last || last.name || last.qty_text) {
    ls.push(blankLine());
    $('#ed-lines tbody').insertAdjacentHTML('beforeend', lineRowHtml(ls[ls.length - 1], ls.length - 1));
  }
}

function updateRow(i) {
  const tr = $(`#ed-lines tr[data-i="${i}"]`);
  if (!tr) return;
  const l = E.doc.lines[i];
  tr.querySelector('[data-amt]').textContent = lineAmountText(l);
  const q = parseQty(l.qty_text);
  const qi = tr.querySelector('[data-f="qty_text"]');
  qi.classList.toggle('invalid', !!l.qty_text && Number.isNaN(q));
  tr.querySelector('.qty-hint').textContent = q !== null && !Number.isNaN(q) && /[斤兩]/.test(l.qty_text) ? `＝ ${fmtNum(q)} 斤` : '';
  tr.querySelector('.price-hint').textContent = l._hint || '';
  tr.cells[0].textContent = l.name || l.qty_text ? i + 1 : '';
}

// 與 Rust 端 paginate() 相同的分頁估算：表頭 9 行、合計區 6 行；
// 非連續報表時，續頁另有欄位列 2 行、每頁頁碼 1 行
function countPages(n, usable, continuous, pair) {
  const footer = continuous ? 0 : 1;
  const capFirst = usable - 9 - footer, capNext = usable - (continuous ? 0 : 2) - footer, t = 6;
  if (pair) {
    // 兩欄：一行放兩筆
    const linesFor = (m) => Math.ceil(m / 2);
    let pages = 0, i = 0;
    for (let guard = 0; guard < 200; guard++) {
      const capL = pages === 0 ? capFirst : capNext;
      pages++;
      const remaining = n - i;
      if (linesFor(remaining) + t <= capL) return pages;
      const cap = capL * 2;
      if (remaining <= cap) {
        const take = remaining > 1 ? remaining - 1 : remaining;
        i += take;
        if (i === n) return pages + 1;
        continue;
      }
      i += cap;
    }
    return pages;
  }
  let pages = 0, i = 0;
  for (let guard = 0; guard < 200; guard++) {
    const cap = pages === 0 ? capFirst : capNext;
    pages++;
    const remaining = n - i;
    if (remaining + t <= cap) return pages;
    if (remaining <= cap) {
      const take = remaining > 1 ? remaining - 1 : remaining;
      i += take;
      if (i === n) return pages + 1;
      continue;
    }
    i += cap;
  }
  return pages;
}
function estimatePages(n) {
  const p = S.settings.print;
  const full = p.page_lines - p.top_lines - p.bottom_lines;
  const half = Math.floor(p.page_lines / 2) - p.top_lines - p.bottom_lines;
  const mode = p.paper_mode || 'half';
  const cont = p.continuous !== false;
  if (mode === 'half') return `${countPages(n, half, cont)} 張中一刀`;
  if (mode === 'auto' && countPages(n, half, cont) === 1) return '半張';
  return `${countPages(n, full, cont)} 頁`;
}

function updateTotals() {
  const d = E.doc;
  const cust = customerById(d.customer_id);
  const taxMode = cust ? cust.tax_mode : d.tax_mode;
  const filled = d.lines.filter((l) => l.name || l.qty_text);
  const amounts = filled.map((l) => lineAmount(l) || 0);
  const t = computeTotals(amounts, taxMode, d.kind === 'adjustment' ? -1 : 1);
  $('#ed-sub').textContent = fmtInt(t.sub);
  $('#ed-tax-label').innerHTML = `稅金（${esc(TAX_LABEL[taxMode] || '')}） <b>${fmtInt(t.tax)}</b>`;
  $('#ed-total').textContent = fmtInt(t.total);
  $('#ed-count').textContent = `共 ${filled.length} 項　列印約 ${estimatePages(filled.length)}`;
}

function updateCustInfo() {
  const c = customerById(E.doc.customer_id);
  const info = $('#ed-cust-info');
  if (!c) {
    info.textContent = E.doc.customer_id ? '' : '請選擇客戶';
    $('#ed-recent').innerHTML = '<p class="hint">選擇客戶後顯示。</p>';
    return;
  }
  const parts = [c.phone && `電話 ${c.phone}`, c.address && `地址 ${c.address}`, `結帳 ${closingLabel(c.closing_day)}`, TAX_LABEL[c.tax_mode]].filter(Boolean);
  info.textContent = parts.join('　') + (c.note ? `　｜ ${c.note}` : '');
  loadRecent();
}

async function loadRecent() {
  const c = customerById(E.doc.customer_id);
  if (!c) return;
  const docs = (await call('list_documents', { filter: { customer_id: c.id, from: null, to: null, limit: 6 } }))
    .filter((d) => d.id !== E.doc.id).slice(0, 5);
  $('#ed-recent').innerHTML = docs.length
    ? docs.map((d) => `<div class="list-item">
        <div><div>${esc(slashDate(d.doc_date).slice(5))}　${fmtInt(d.total)}</div><div class="hint">${esc(d.doc_no)}・${d.item_count} 項</div></div>
        ${locked() ? '' : `<button type="button" class="btn small" data-copy="${d.id}">複製品項</button>`}
      </div>`).join('')
    : '<p class="hint">這位客戶還沒有單據。</p>';
}

async function applyProduct(i, prod) {
  const l = E.doc.lines[i];
  l.product_id = prod.id;
  l.name = prod.name;
  l.unit = prod.unit;
  l._hint = '';
  if (E.doc.customer_id) {
    const s = await call('suggest_price', { customerId: E.doc.customer_id, productId: prod.id });
    l.price = s.price;
    l._hint = { customer: '客戶專屬價', last: '上次成交價', default: '預設價', none: '時價，請輸入' }[s.source] || '';
    if (s.source === 'customer' && s.last_price !== null && s.last_price !== s.price) l._hint += `（上次 ${fmtNum(s.last_price)}）`;
  } else {
    l.price = prod.price;
    l._hint = prod.price === null ? '時價，請輸入' : '';
  }
  const tr = $(`#ed-lines tr[data-i="${i}"]`);
  tr.querySelector('[data-f="name"]').value = productLabel(prod);
  tr.querySelector('[data-f="unit"]').value = l.unit;
  tr.querySelector('[data-f="price"]').value = l.price ?? '';
  updateRow(i);
  updateTotals();
}

function bindEditor() {
  $('#ed-back').onclick = () => showView('ship');
  $('#ed-preview').onclick = () => editorPreview();
  $('#ed-print').onclick = () => editorPrint();
  if ($('#ed-save')) $('#ed-save').onclick = () => editorSave('issued');
  if ($('#ed-draft')) $('#ed-draft').onclick = () => editorSave('draft');
  if ($('#ed-delete')) $('#ed-delete').onclick = async () => {
    if (!(await confirmBox(`確定刪除 ${E.doc.doc_no}？刪除後無法復原。`, '刪除'))) return;
    await call('delete_document', { id: E.doc.id });
    E.dirty = false;
    toast(`已刪除 ${E.doc.doc_no}`);
    showView('ship');
  };
  if (locked()) return;

  $('#ed-cust').onchange = (e) => {
    const c = findCustomer(e.target.value);
    if (!c) { toast('找不到這位客戶，請從清單選擇，或先到「客戶」新增', 'error'); return; }
    E.doc.customer_id = c.id;
    E.doc.tax_mode = c.tax_mode;
    E.dirty = true;
    e.target.value = customerLabel(c);
    updateCustInfo();
    updateTotals();
  };
  $('#ed-date').onchange = (e) => { E.doc.doc_date = e.target.value; E.dirty = true; };
  $('#ed-maker').oninput = (e) => { E.doc.maker = e.target.value; E.dirty = true; };

  const tbody = $('#ed-lines tbody');
  tbody.addEventListener('input', (e) => {
    const f = e.target.dataset.f;
    if (!f) return;
    const i = Number(e.target.closest('tr').dataset.i);
    const l = E.doc.lines[i];
    E.dirty = true;
    if (f === 'price') {
      const v = parseNum(e.target.value);
      l.price = v === null ? null : v;
      e.target.classList.toggle('invalid', Number.isNaN(v));
      l._hint = '';
    } else if (f === 'name') {
      l.name = e.target.value;
      const prod = findProduct(e.target.value);
      if (!prod || prod.id !== l.product_id) l.product_id = null;
      // 從下拉清單點選品項：直接帶入單位與單價，游標跳到數量
      if (prod && e.inputType === 'insertReplacementText') {
        applyProduct(i, prod).then(() => e.target.closest('tr').querySelector('[data-f="qty_text"]').focus());
      }
    } else {
      l[f] = e.target.value;
    }
    updateRow(i);
    updateTotals();
    if (f === 'name' || f === 'qty_text') ensureTrailingBlank();
  });
  tbody.addEventListener('change', (e) => {
    if (e.target.dataset.f !== 'name') return;
    const i = Number(e.target.closest('tr').dataset.i);
    const prod = findProduct(e.target.value);
    if (prod && prod.id === E.doc.lines[i].product_id && E.doc.lines[i].price !== null) return;
    if (prod) applyProduct(i, prod);
    else {
      E.doc.lines[i].name = e.target.value.trim();
      E.doc.lines[i].product_id = null;
    }
  });
  tbody.addEventListener('click', (e) => {
    const del = e.target.closest('[data-del]');
    if (!del) return;
    const i = Number(del.closest('tr').dataset.i);
    E.doc.lines.splice(i, 1);
    if (!E.doc.lines.length) E.doc.lines.push(blankLine());
    E.dirty = true;
    renderLines();
    ensureTrailingBlank();
  });
  // Enter：往右跳一格，最後一格跳到下一列品名
  tbody.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter' || !e.target.dataset.f) return;
    e.preventDefault();
    const tr = e.target.closest('tr');
    if (e.target.dataset.f === 'name') {
      const i = Number(tr.dataset.i);
      const prod = findProduct(e.target.value);
      if (prod && prod.id !== E.doc.lines[i].product_id) applyProduct(i, prod);
    }
    // Enter 的順序：品名 → 數量 →（單位空白時）單位 → 單價 → 備註 → 下一列
    const f = e.target.dataset.f;
    const unitEmpty = !tr.querySelector('[data-f="unit"]').value.trim();
    const next = { name: 'qty_text', qty_text: unitEmpty ? 'unit' : 'price', unit: 'price', price: 'note' }[f];
    if (next) {
      tr.querySelector(`[data-f="${next}"]`).focus();
    } else {
      ensureTrailingBlank();
      const next = tr.nextElementSibling;
      if (next) next.querySelector('[data-f="name"]').focus();
    }
  });
  $('#ed-recent').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-copy]');
    if (!b) return;
    const src = await call('get_document', { id: Number(b.dataset.copy) });
    const keep = E.doc.lines.filter((l) => l.name || l.qty_text);
    E.doc.lines = keep.concat(src.lines.map((l) => ({
      id: null, product_id: l.product_id, name: l.name, unit: l.unit, qty_text: l.qty_text, qty: l.qty, price: l.price, amount: l.amount, note: l.note, _hint: '',
    })));
    E.doc.lines.push(blankLine());
    E.dirty = true;
    renderLines();
    toast(`已複製 ${src.lines.length} 項，數量與單價請再確認`);
  });
}

// 存草稿可以先不填單價（時價品項等採購後再補）；開立時每一列都要有單價
function validateEditor(status) {
  const d = E.doc;
  if (!d.customer_id) { toast('請先選擇客戶', 'error'); $('#ed-cust')?.focus(); return false; }
  if (!d.doc_date) { toast('請填日期', 'error'); return false; }
  for (const [i, l] of d.lines.entries()) {
    if (!l.name && !l.qty_text) continue;
    if (!String(l.name).trim()) { toast(`第 ${i + 1} 列沒有品名`, 'error'); return false; }
    const q = parseQty(l.qty_text);
    if (q === null || Number.isNaN(q)) { toast(`第 ${i + 1} 列數量不正確`, 'error'); return false; }
    if (l.price !== null && l.price !== undefined && l.price !== '' && Number.isNaN(Number(l.price))) { toast(`第 ${i + 1} 列單價不是數字`, 'error'); return false; }
    if (status !== 'draft' && (l.price === null || l.price === undefined || l.price === '')) { toast(`第 ${i + 1} 列沒有單價（存草稿可以先不填）`, 'error'); return false; }
  }
  return true;
}

async function editorSave(status) {
  if (locked()) return E.doc;
  if (!validateEditor(status || E.doc.status)) return null;
  const d = E.doc;
  const payload = {
    ...d,
    status: status || d.status,
    lines: d.lines.filter((l) => l.name || l.qty_text).map((l) => ({
      id: null, product_id: l.product_id, name: String(l.name).trim(), unit: l.unit || '', qty_text: l.qty_text,
      qty: 0, price: Number(l.price), amount: 0, note: l.note || '',
    })),
  };
  const before = new Set(S.products.map((p) => p.id));
  const saved = await call('save_document', { doc: payload });
  await loadProducts();
  const added = S.products.filter((p) => !before.has(p.id));
  if (added.length) toast(`已把 ${added.length} 個新品項加入品項清單：${added.map((p) => p.name).join('、')}。可到「品項」補代號或改售價`);
  saved.lines = saved.lines.map((l) => ({ ...l, _hint: '', price: saved.status === 'draft' && l.price === 0 ? null : l.price }));
  saved.lines.push(blankLine());
  E.doc = saved;
  E.dirty = false;
  renderEditor();
  toast(`${saved.status === 'draft' ? '已存草稿' : '已存檔'} ${saved.doc_no}`);
  await loadCustomers();
  return saved;
}

async function editorPreview() {
  let d = E.doc;
  if (!locked() && (E.dirty || !d.id)) {
    d = await editorSave(d.id ? d.status : 'draft');
    if (!d) return;
  }
  const r = await call('render_document', { id: d.id });
  openPreview(r, `${d.kind === 'adjustment' ? '退貨／折讓' : '出貨單'} ${d.doc_no}`, () => printDocById(d.id));
}

async function editorPrint() {
  let d = E.doc;
  if (!locked()) {
    d = await editorSave('issued');
    if (!d) return;
  }
  await printDocById(d.id);
}

async function printDocById(id) {
  if (S.settings.print.mode === 'escp') {
    await call('print_document', { id });
    toast('已送到印表機');
    if (currentView === 'ship-edit' && E.doc && E.doc.id === id && E.doc.status === 'draft') {
      E.doc.status = 'issued';
      renderEditor();
    }
  } else {
    const r = await call('render_document', { id });
    await driverPrint(r);
    await call('mark_issued', { id });
  }
}

document.addEventListener('keydown', (e) => {
  if ($('#preview-back').classList.contains('open') || $('#confirm-back').classList.contains('open')) return;
  const save = (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's';
  if (currentView === 'ship-edit') {
    if (e.key === 'F9') { e.preventDefault(); editorPrint(); }
    if (save) { e.preventDefault(); if (!locked()) editorSave('issued'); }
  } else if (currentView === 'man-edit') {
    if (e.key === 'F9') { e.preventDefault(); manPrint(); }
    if (save) { e.preventDefault(); manSave('issued'); }
  }
});

// ============================================================
// 預覽與驅動程式列印
// ============================================================
function charWidth(c) {
  const u = c.codePointAt(0);
  if (u < 0x1100) return 1;
  return ((u >= 0x1100 && u <= 0x115F) || (u >= 0x2E80 && u <= 0xA4CF) || (u >= 0xAC00 && u <= 0xD7A3) ||
    (u >= 0xF900 && u <= 0xFAFF) || (u >= 0xFE30 && u <= 0xFE4F) || (u >= 0xFF00 && u <= 0xFF60) ||
    (u >= 0xFFE0 && u <= 0xFFE6) || (u >= 0x20000 && u <= 0x3FFFD) || (u >= 0x2500 && u <= 0x257F)) ? 2 : 1;
}

// forDriver＝要交給 Windows 驅動程式列印：頁面尺寸用驅動程式裡的自訂紙張（例如中一刀 21×14 cm），
// 不含左右齒孔邊；否則是畫面預覽，畫出整張 9½ 吋寬的報表紙。
function pagesHtml(r, forDriver) {
  const c = r.config;
  const cw = 1 / (c.cpi || 10); // 半形字寬（吋）
  const lh = (c.line_spacing_180 || 30) / 180; // 行高（吋）
  const fs = Math.min(cw * 2, lh) * 0.92;
  const left = c.left_margin_cols * cw;
  const lineHtml = (ln) => {
    let out = '';
    let spaces = 0;
    const flush = () => { if (spaces) { out += `<span class="ch" style="width:${spaces * cw}in"></span>`; spaces = 0; } };
    for (const ch of ln.text) {
      if (ch === ' ') { spaces++; continue; }
      flush();
      out += `<span class="ch" style="width:${charWidth(ch) * cw}in">${esc(ch)}</span>`;
    }
    flush();
    return `<div class="ln${ln.bold ? ' b' : ''}" style="height:${lh}in;line-height:${lh}in">${out || '&nbsp;'}</div>`;
  };
  const pageLines = r.page_lines || c.page_lines;
  if (forDriver) {
    const { w, h } = driverPaper(r);
    // 高度略小於紙張 0.5 mm，避免捨入誤差把內容擠到下一頁
    return r.pages.map((p) => `<div class="paper driver" style="width:${w}cm;height:${h - 0.05}cm">
        <div class="content" style="left:0;padding-top:${c.top_lines * lh}in;padding-left:${left}in;font-size:${fs}in">${p.map(lineHtml).join('')}</div>
      </div>`).join('');
  }
  const paperH = pageLines * lh; // 整張 11 吋、半張 5.5 吋
  return r.pages.map((p) => `<div class="paper" style="height:${paperH}in">
      <div class="strip l"></div><div class="strip r"></div>
      <div class="content" style="padding-top:${c.top_lines * lh}in;padding-left:${left}in;font-size:${fs}in">${p.map(lineHtml).join('')}</div>
    </div>`).join('');
}

let previewPrintAction = null;
function openPreview(r, title, onPrint) {
  const unit = r.half ? '張中一刀' : '頁';
  $('#preview-title').textContent = `${title}（共 ${r.pages.length} ${unit}）`;
  $('#preview-pages').innerHTML = pagesHtml(r);
  previewPrintAction = onPrint;
  $('#preview-print').style.display = onPrint ? '' : 'none';
  $('#preview-back').classList.add('open');
}
function closePreview() { $('#preview-back').classList.remove('open'); $('#preview-pages').innerHTML = ''; }
$('#preview-close').onclick = closePreview;
$('#preview-print').onclick = async () => {
  if (!previewPrintAction) return;
  const act = previewPrintAction;
  closePreview();
  await act();
};

// 驅動程式的紙張：半張＝設定的自訂紙張（中一刀），整張＝同寬、兩倍長
function driverPaper(r) {
  const c = r.config;
  const w = Number(c.driver_paper_w_cm) || 21;
  const h = Number(c.driver_paper_h_cm) || 14;
  return { w, h: r.half ? h : h * 2 };
}

function driverPrint(r) {
  return new Promise((resolve) => {
    const root = $('#print-root');
    root.innerHTML = pagesHtml(r, true);
    // 頁面尺寸與驅動程式的紙張完全相同，WebView2 就不會轉向或縮放
    const { w, h } = driverPaper(r);
    let st = document.getElementById('page-size');
    if (!st) { st = document.createElement('style'); st.id = 'page-size'; document.head.appendChild(st); }
    st.textContent = `@media print { @page { size: ${w}cm ${h}cm; margin: 0; } }`;
    const done = () => { window.removeEventListener('afterprint', done); root.innerHTML = ''; resolve(); };
    window.addEventListener('afterprint', done);
    setTimeout(() => window.print(), 50);
  });
}

// ============================================================
// 對帳單（手開）：只有日期、金額、備註，獨立作業
// ============================================================
const M = { doc: null, dirty: false };

function periodLabel(p) {
  if (!p) return '';
  const [y, m] = p.split('-');
  return `${y} 年 ${Number(m)} 月`;
}
function periodTitle(p) { return p ? `${Number(p.split('-')[1])}月份對帳單` : '對帳單'; }

// 日期快速輸入：「28」→ 對帳月份的 28 日；「10/28」→ 對帳月份那一年的 10/28；也接受完整日期
function parseManDate(text, period) {
  const t = String(text ?? '').replace(/[０-９]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xFEE0)).replace(/[．.\-]/g, '/').replace(/\s/g, '');
  if (!t) return null;
  const [py, pm] = (period || thisMonth()).split('-').map(Number);
  let y, m, d, parts = t.split('/').filter(Boolean);
  if (parts.length === 1) { y = py; m = pm; d = Number(parts[0]); }
  else if (parts.length === 2) { y = py; m = Number(parts[0]); d = Number(parts[1]); }
  else if (parts.length === 3) { y = Number(parts[0]); m = Number(parts[1]); d = Number(parts[2]); if (y < 1000) y += 1911; }
  else return NaN;
  if (![y, m, d].every(Number.isInteger) || m < 1 || m > 12 || d < 1) return NaN;
  if (d > new Date(y, m, 0).getDate()) return NaN;
  return `${y}-${pad2(m)}-${pad2(d)}`;
}

function blankManLine() { return { id: null, line_date: '', amount: null, note: '', _text: '' }; }

async function renderManList() {
  if (!$('#man-f-month').dataset.init) { $('#man-f-month').dataset.init = '1'; }
  const cust = findCustomer($('#man-f-cust').value);
  const period = $('#man-f-month').value || null;
  const list = await call('list_manual', { customerId: cust ? cust.id : null, period, limit: 500 });
  const box = $('#man-list');
  if (!list.length) {
    box.innerHTML = '<div class="empty">還沒有手開對帳單。按右上角「新增對帳單」開始。</div>';
    return;
  }
  box.innerHTML = `<table class="grid"><thead><tr>
      <th>對帳月份</th><th>單號</th><th>客戶</th><th class="num">筆數</th><th class="num">合計</th><th>狀態</th></tr></thead><tbody>
    ${list.map((m) => `<tr class="click" data-id="${m.id}">
      <td>${esc(periodLabel(m.period))}</td><td>${esc(m.stmt_no)}</td><td>${esc(m.cust_name)}</td>
      <td class="num">${m.line_count}</td><td class="num">${fmtInt(m.total)}</td>
      <td>${m.status === 'draft' ? '<span class="badge draft">草稿</span>' : '<span class="badge">已開立</span>'}</td>
    </tr>`).join('')}</tbody></table>`;
}
$('#man-list').addEventListener('click', (e) => {
  const tr = e.target.closest('tr[data-id]');
  if (tr) openManual(Number(tr.dataset.id));
});
$('#man-f-cust').addEventListener('change', renderManList);
$('#man-f-month').addEventListener('change', renderManList);
$('#btn-new-man').addEventListener('click', () => newManual());

async function newManual() {
  if (!(await showView('man-edit'))) return;
  const preset = findCustomer($('#man-f-cust').value);
  // 預設對帳月份：每月 10 號以前是上個月，之後是這個月；單據日期為該月最後一天
  const period = $('#man-f-month').value || prevMonth();
  M.doc = {
    id: null, stmt_no: '', customer_id: preset ? preset.id : null, period, doc_date: monthRange(period)[1],
    maker: S.settings.default_maker || '', note: '', status: 'draft', layout: S.settings.manual_layout || 'double',
    cust_name: '', cust_phone: '', cust_address: '', tax_mode: preset ? preset.tax_mode : 'exclusive',
    subtotal: 0, tax: 0, total: 0, lines: [blankManLine()],
  };
  M.dirty = false;
  renderManEditor();
  setTimeout(() => $(M.doc.customer_id ? '#man-lines [data-f="date"]' : '#man-cust')?.focus(), 0);
}

async function openManual(id) {
  const doc = await call('get_manual', { id });
  if (!(await showView('man-edit'))) return;
  doc.lines = doc.lines.map((l) => ({ ...l, _text: slashDate(l.line_date) }));
  doc.lines.push(blankManLine());
  M.doc = doc;
  M.dirty = false;
  renderManEditor();
}

function renderManEditor() {
  const d = M.doc;
  const cust = customerById(d.customer_id);
  $('#view-man-edit').innerHTML = `
    <div class="head">
      <div style="display:flex;align-items:center;gap:12px;flex-wrap:wrap">
        <button type="button" class="btn" id="mn-back">‹ 返回列表</button>
        <h1>${d.id ? `對帳單 ${esc(d.stmt_no)}` : '新增對帳單'}</h1>
        ${d.id ? (d.status === 'draft' ? '<span class="badge draft">草稿</span>' : '<span class="badge">已開立</span>') : '<span class="sub">單號存檔時自動編號</span>'}
      </div>
      <div class="actions">
        ${d.id ? '<button type="button" class="btn danger" id="mn-delete">刪除</button>' : ''}
        <button type="button" class="btn" id="mn-draft">存草稿</button>
        <button type="button" class="btn" id="mn-save">存檔 Ctrl+S</button>
        <button type="button" class="btn" id="mn-preview">預覽</button>
        <button type="button" class="btn primary" id="mn-print">列印 F9</button>
      </div>
    </div>
    <div class="card pad">
      <div class="fields">
        <label class="f span2"><span class="lbl">客戶（輸入名稱或代號）</span>
          <input type="text" id="man-cust" data-combo="customers" autocomplete="off" value="${esc(cust ? customerLabel(cust) : d.cust_name)}">
          <span class="info-line" id="mn-cust-info"></span></label>
        <label class="f"><span class="lbl">對帳月份（印成標題）</span><input type="month" id="mn-period" value="${esc(d.period)}">
          <span class="info-line" id="mn-title" style="color:var(--accent)">標題：${esc(periodTitle(d.period))}</span></label>
        <label class="f"><span class="lbl">單據日期</span><input type="date" id="mn-date" value="${esc(d.doc_date)}"></label>
        <label class="f"><span class="lbl">製表人員</span><input type="text" id="mn-maker" value="${esc(d.maker)}"></label>
        <div class="f"><span class="lbl">列印版面</span>
          <div class="seg" id="mn-layout">
            <button type="button" data-layout="double" aria-pressed="${d.layout !== 'single'}">左右兩欄</button>
            <button type="button" data-layout="single" aria-pressed="${d.layout === 'single'}">單欄</button>
          </div>
        </div>
      </div>
    </div>
    <div class="row">
      <div class="grow card">
        <div class="table-wrap">
          <table class="grid lines" id="man-lines">
            <thead><tr><th>#</th><th style="width:170px">日期</th><th style="width:150px;text-align:right">金額</th><th>備註</th><th class="c-del"></th></tr></thead>
            <tbody></tbody>
          </table>
        </div>
        <div class="totals-bar">
          <span class="hint" id="mn-count"></span>
          <div class="nums">
            <span class="hint">未稅 <b id="mn-sub"></b></span>
            <span class="hint" id="mn-tax-label"></span>
            <span class="grand">合計 <span id="mn-total"></span></span>
          </div>
        </div>
      </div>
      <aside class="side stack">
        <div class="card pad">
          <h2>快速輸入</h2>
          <p class="hint">日期只要打「日」，例如打 <b>28</b> 會自動變成對帳月份的 28 日；也可以打 10/28 或完整日期。</p>
          <p class="hint" style="margin-top:6px">日期按 Enter 跳到金額，金額按 Enter 跳到下一列。存檔時會依日期自動排序。</p>
          <p class="hint warn-text" style="margin-top:6px">同一天重複、或日期不在對帳月份內會提醒，但仍可存檔。</p>
        </div>
        <div class="card pad">
          <h2>此客戶最近的對帳單</h2>
          <div id="mn-recent"><p class="hint">選擇客戶後顯示。</p></div>
        </div>
      </aside>
    </div>`;
  renderManLines();
  updateManCust();
  bindManEditor();
}

function manRowHtml(l, i) {
  const filled = l._text || l.amount !== null && l.amount !== '' || l.note;
  return `<tr data-i="${i}">
    <td>${filled ? i + 1 : ''}</td>
    <td><input type="text" data-f="date" value="${esc(l._text)}" aria-label="第 ${i + 1} 列日期" placeholder="${filled ? '' : '日期'}"><div class="qty-hint"></div></td>
    <td><input type="text" data-f="amount" value="${l.amount === null || l.amount === undefined ? '' : esc(l.amount)}" style="text-align:right" inputmode="numeric" aria-label="金額"></td>
    <td><input type="text" data-f="note" value="${esc(l.note)}" aria-label="備註"></td>
    <td class="c-del">${filled ? `<button type="button" class="icon-btn" data-del aria-label="刪除第 ${i + 1} 列"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg></button>` : ''}</td>
  </tr>`;
}

function renderManLines() {
  $('#man-lines tbody').innerHTML = M.doc.lines.map(manRowHtml).join('');
  M.doc.lines.forEach((_, i) => updateManRow(i));
  updateManTotals();
}

function manFilled(l) { return !!(l._text || (l.amount !== null && l.amount !== '') || l.note); }

function ensureManBlank() {
  const ls = M.doc.lines;
  if (!ls.length || manFilled(ls[ls.length - 1])) {
    ls.push(blankManLine());
    $('#man-lines tbody').insertAdjacentHTML('beforeend', manRowHtml(ls[ls.length - 1], ls.length - 1));
  }
}

function updateManRow(i) {
  const tr = $(`#man-lines tr[data-i="${i}"]`);
  if (!tr) return;
  const l = M.doc.lines[i];
  const di = tr.querySelector('[data-f="date"]');
  const hint = tr.querySelector('.qty-hint');
  const parsed = parseManDate(l._text, M.doc.period);
  l.line_date = parsed && !Number.isNaN(parsed) ? parsed : '';
  di.classList.toggle('invalid', !!l._text && (parsed === null || Number.isNaN(parsed)));
  let msg = '', warn = false;
  if (l.line_date) {
    msg = `＝ ${slashDate(l.line_date)}`;
    if (l.line_date.slice(0, 7) !== M.doc.period) { msg += '（不在對帳月份）'; warn = true; }
    else if (M.doc.lines.some((o, j) => j !== i && o.line_date === l.line_date)) { msg += '（日期重複）'; warn = true; }
  } else if (l._text) { msg = '看不懂這個日期'; warn = true; }
  // 已經是完整日期又沒有問題時，不必再重複顯示
  if (!warn && l.line_date && l._text === slashDate(l.line_date)) msg = '';
  hint.textContent = msg;
  hint.style.color = warn ? 'var(--warn)' : '';
  tr.cells[0].textContent = manFilled(l) ? i + 1 : '';
}

function updateManTotals() {
  const d = M.doc;
  const cust = customerById(d.customer_id);
  const taxMode = cust ? cust.tax_mode : d.tax_mode;
  const filled = d.lines.filter(manFilled);
  const sum = filled.reduce((a, l) => a + (Number.isFinite(Number(l.amount)) ? Math.round(Number(l.amount)) : 0), 0);
  const t = computeTotals([sum], taxMode, 1);
  $('#mn-sub').textContent = fmtInt(t.sub);
  $('#mn-tax-label').innerHTML = `稅金（${esc(TAX_LABEL[taxMode] || '')}） <b>${fmtInt(t.tax)}</b>`;
  $('#mn-total').textContent = fmtInt(t.total);
  const p = S.settings.print;
  const usable = (p.paper_mode === 'full' ? p.page_lines : Math.floor(p.page_lines / 2)) - p.top_lines - p.bottom_lines;
  const sheets = countPages(filled.length, usable, p.continuous !== false, d.layout !== 'single');
  $('#mn-count').textContent = `共 ${filled.length} 筆　列印約 ${sheets} ${p.paper_mode === 'full' ? '頁' : '張中一刀'}`;
}

function updateManCust() {
  const c = customerById(M.doc.customer_id);
  if (!c) {
    $('#mn-cust-info').textContent = M.doc.customer_id ? '' : '請選擇客戶';
    $('#mn-recent').innerHTML = '<p class="hint">選擇客戶後顯示。</p>';
    return;
  }
  $('#mn-cust-info').textContent = [c.phone && `電話 ${c.phone}`, c.address && `地址 ${c.address}`, TAX_LABEL[c.tax_mode]].filter(Boolean).join('　');
  loadManRecent();
}

async function loadManRecent() {
  const c = customerById(M.doc.customer_id);
  if (!c) return;
  const list = (await call('list_manual', { customerId: c.id, period: null, limit: 6 })).filter((m) => m.id !== M.doc.id).slice(0, 5);
  $('#mn-recent').innerHTML = list.length
    ? list.map((m) => `<div class="list-item"><div><div>${esc(periodTitle(m.period).replace('對帳單', ''))}　${fmtInt(m.total)}</div>
        <div class="hint">${esc(m.stmt_no)}・${m.line_count} 筆</div></div>
        <button type="button" class="btn small" data-open="${m.id}">開啟</button></div>`).join('')
    : '<p class="hint">這位客戶還沒有手開對帳單。</p>';
}

function bindManEditor() {
  $('#mn-back').onclick = () => showView('man');
  $('#mn-save').onclick = () => manSave('issued');
  $('#mn-draft').onclick = () => manSave('draft');
  $('#mn-preview').onclick = () => manPreview();
  $('#mn-print').onclick = () => manPrint();
  if ($('#mn-delete')) $('#mn-delete').onclick = async () => {
    if (!(await confirmBox(`確定刪除對帳單 ${M.doc.stmt_no}？刪除後無法復原。`, '刪除'))) return;
    await call('delete_manual', { id: M.doc.id });
    M.dirty = false;
    toast(`已刪除 ${M.doc.stmt_no}`);
    showView('man');
  };
  $('#man-cust').onchange = (e) => {
    const c = findCustomer(e.target.value);
    if (!c) { toast('找不到這位客戶，請從清單選擇，或先到「客戶」新增', 'error'); return; }
    M.doc.customer_id = c.id;
    M.doc.tax_mode = c.tax_mode;
    M.dirty = true;
    e.target.value = customerLabel(c);
    updateManCust();
    updateManTotals();
  };
  $('#mn-period').onchange = (e) => {
    if (!e.target.value) return;
    M.doc.period = e.target.value;
    M.doc.doc_date = monthRange(M.doc.period)[1];
    $('#mn-date').value = M.doc.doc_date;
    $('#mn-title').textContent = `標題：${periodTitle(M.doc.period)}`;
    M.dirty = true;
    M.doc.lines.forEach((_, i) => updateManRow(i));
  };
  $('#mn-date').onchange = (e) => { M.doc.doc_date = e.target.value; M.dirty = true; };
  $('#mn-maker').oninput = (e) => { M.doc.maker = e.target.value; M.dirty = true; };
  $('#mn-layout').onclick = (e) => {
    const b = e.target.closest('button[data-layout]');
    if (!b) return;
    M.doc.layout = b.dataset.layout;
    $$('#mn-layout button').forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
    M.dirty = true;
    updateManTotals();
  };
  const tbody = $('#man-lines tbody');
  tbody.addEventListener('input', (e) => {
    const f = e.target.dataset.f;
    if (!f) return;
    const i = Number(e.target.closest('tr').dataset.i);
    const l = M.doc.lines[i];
    M.dirty = true;
    if (f === 'date') l._text = e.target.value;
    if (f === 'amount') {
      const v = parseNum(e.target.value);
      l.amount = v === null ? null : v;
      e.target.classList.toggle('invalid', Number.isNaN(v));
    }
    if (f === 'note') l.note = e.target.value;
    if (f === 'date') M.doc.lines.forEach((_, j) => updateManRow(j)); else updateManRow(i);
    updateManTotals();
    ensureManBlank();
  });
  // 日期欄離開時改成完整日期顯示
  tbody.addEventListener('change', (e) => {
    if (e.target.dataset.f !== 'date') return;
    const i = Number(e.target.closest('tr').dataset.i);
    const l = M.doc.lines[i];
    if (l.line_date) { l._text = slashDate(l.line_date); e.target.value = l._text; }
  });
  tbody.addEventListener('click', (e) => {
    const del = e.target.closest('[data-del]');
    if (!del) return;
    M.doc.lines.splice(Number(del.closest('tr').dataset.i), 1);
    if (!M.doc.lines.length) M.doc.lines.push(blankManLine());
    M.dirty = true;
    renderManLines();
    ensureManBlank();
  });
  tbody.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter' || !e.target.dataset.f) return;
    e.preventDefault();
    const tr = e.target.closest('tr');
    const order = ['date', 'amount', 'note'];
    const idx = order.indexOf(e.target.dataset.f);
    if (e.target.dataset.f === 'date') e.target.dispatchEvent(new Event('change', { bubbles: true }));
    // 金額按 Enter 直接跳下一列（備註通常空白）
    if (e.target.dataset.f === 'date') { tr.querySelector('[data-f="amount"]').focus(); return; }
    ensureManBlank();
    const next = tr.nextElementSibling;
    if (next) next.querySelector('[data-f="date"]').focus();
    void idx;
  });
  $('#mn-recent').addEventListener('click', (e) => {
    const b = e.target.closest('[data-open]');
    if (b) openManual(Number(b.dataset.open));
  });
}

function validateManual() {
  const d = M.doc;
  if (!d.customer_id) { toast('請先選擇客戶', 'error'); $('#man-cust')?.focus(); return false; }
  if (!d.period) { toast('請選擇對帳月份', 'error'); return false; }
  for (const [i, l] of d.lines.entries()) {
    if (!manFilled(l)) continue;
    if (!l.line_date) { toast(`第 ${i + 1} 列日期不正確`, 'error'); return false; }
    if (l.amount === null || l.amount === '' || Number.isNaN(Number(l.amount))) { toast(`第 ${i + 1} 列沒有金額`, 'error'); return false; }
  }
  return true;
}

async function manSave(status) {
  if (!validateManual()) return null;
  const d = M.doc;
  const payload = {
    ...d,
    status: status || d.status,
    lines: d.lines.filter(manFilled).map((l) => ({ id: null, line_date: l.line_date, amount: Math.round(Number(l.amount)), note: l.note || '' })),
  };
  const saved = await call('save_manual', { doc: payload });
  saved.lines = saved.lines.map((l) => ({ ...l, _text: slashDate(l.line_date) }));
  saved.lines.push(blankManLine());
  M.doc = saved;
  M.dirty = false;
  renderManEditor();
  toast(`${saved.status === 'draft' ? '已存草稿' : '已存檔'} ${saved.stmt_no}`);
  return saved;
}

async function manPreview() {
  let d = M.doc;
  if (M.dirty || !d.id) { d = await manSave(d.id ? d.status : 'draft'); if (!d) return; }
  const r = await call('render_manual', { id: d.id });
  openPreview(r, `${periodTitle(d.period)} ${d.stmt_no}`, () => printManById(d.id));
}

async function manPrint() {
  const d = await manSave('issued');
  if (d) await printManById(d.id);
}

async function printManById(id) {
  if (S.settings.print.mode === 'escp') {
    await call('print_manual', { id });
    toast('已送到印表機');
  } else {
    const r = await call('render_manual', { id });
    await driverPrint(r);
    await call('mark_manual_issued', { id });
  }
  if (currentView === 'man-edit' && M.doc && M.doc.id === id && M.doc.status === 'draft') {
    M.doc.status = 'issued';
    renderManEditor();
  }
}

// ============================================================
// 對帳單・收款
// ============================================================
const ST = { mode: 'summary', preview: null };

function stCustomer() { return findCustomer($('#st-cust').value); }

async function stmtRefresh() {
  if (!$('#st-month').value) $('#st-month').value = prevMonth();
  if (!$('#pay-date').value) $('#pay-date').value = today();
  const c = stCustomer();
  $('#st-cust-info').textContent = c ? `結帳 ${closingLabel(c.closing_day)}　${TAX_LABEL[c.tax_mode]}　目前未收 ${fmtInt(S.balances[c.id] || 0)}` : '';
  await renderStatementList();
  await renderPayments();
  await stmtPreview();
}

// 每月 10 號以前預設對上個月，之後預設這個月
function prevMonth() {
  const d = new Date();
  if (d.getDate() <= 10) d.setMonth(d.getMonth() - 1, 1);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}`;
}

async function stmtPeriod() {
  const c = stCustomer();
  const ym = $('#st-month').value;
  if (!c || !ym) return;
  const [y, m] = ym.split('-').map(Number);
  const [a, b] = await call('statement_period', { customerId: c.id, year: y, month: m });
  $('#st-start').value = a;
  $('#st-end').value = b;
}

async function stmtPreview() {
  const c = stCustomer();
  const start = $('#st-start').value, end = $('#st-end').value;
  const box = $('#st-preview');
  if (!c || !start || !end) {
    box.innerHTML = '<div class="empty">選擇客戶和月份，就會列出這段期間的出貨單。</div>';
    $('#st-summary').innerHTML = '<h2>本期金額</h2><p class="hint">尚未選擇客戶。</p>';
    ST.preview = null;
    return;
  }
  const p = await call('preview_statement', { customerId: c.id, start, end });
  ST.preview = p;
  const warn = [];
  if (p.drafts_excluded) warn.push(`有 ${p.drafts_excluded} 張草稿不會列入，請先到出貨單存檔。`);
  if (p.already_locked) warn.push(`有 ${p.already_locked} 張已經列入其他對帳單，需要先作廢那張才能重建。`);
  box.innerHTML = `
    ${warn.map((w) => `<div class="banner warn" style="margin:14px 16px 0">${esc(w)}</div>`).join('')}
    ${p.docs.length ? `<table class="grid"><thead><tr><th>日期</th><th>單號</th><th>說明</th><th class="num">品項</th><th class="num">金額（含稅）</th><th>狀態</th></tr></thead><tbody>
      ${p.docs.map((d) => `<tr class="click" data-id="${d.id}"><td>${esc(slashDate(d.doc_date).slice(5))}</td><td>${esc(d.doc_no)}</td>
        <td>${d.kind === 'adjustment' ? '<span class="badge adj">退貨／折讓</span>' : '出貨'}</td><td class="num">${d.item_count}</td>
        <td class="num">${fmtInt(d.total)}</td><td>${d.statement_id ? '<span class="badge locked">已對帳</span>' : ''}</td></tr>`).join('')}
      </tbody></table>` : '<div class="empty">這段期間沒有已開立的出貨單。</div>'}
    <p class="hint" style="padding:12px 20px">對帳單建立後，這些單據就會鎖定不能修改；事後更正請開退貨／折讓單，會列入下一期。</p>`;
  $('#st-summary').innerHTML = `<h2>本期金額</h2>
    <div class="kv"><span class="hint">前期未收</span><span>${fmtInt(p.prev_balance)}</span></div>
    <div class="kv"><span class="hint">本期金額（${p.docs.length} 張）</span><span>${fmtInt(p.period_amount)}</span></div>
    <div class="kv"><span class="hint">本期收款</span><span>${fmtInt(-p.paid_in_period)}</span></div>
    <div class="kv total"><span>本期應收</span><span>${fmtInt(p.total_due)}</span></div>
    <div class="actions" style="margin-top:14px;flex-direction:column">
      <button type="button" class="btn primary" id="st-create" ${p.already_locked || !p.docs.length ? 'disabled' : ''}>建立對帳單並預覽</button>
    </div>`;
  $('#st-create').onclick = stmtCreate;
}

async function stmtCreate() {
  const c = stCustomer();
  if (!c || !ST.preview) return;
  const ok = await confirmBox(`建立 ${c.name} ${slashDate($('#st-start').value)}–${slashDate($('#st-end').value)} 的對帳單？\n建立後這 ${ST.preview.docs.length} 張單會鎖定。`, '建立');
  if (!ok) return;
  const st = await call('create_statement', { customerId: c.id, start: $('#st-start').value, end: $('#st-end').value, detailMode: ST.mode, maker: S.settings.default_maker || '' });
  toast(`已建立對帳單 ${st.stmt_no}`);
  await stmtRefresh();
  await previewStatement(st.id, st.stmt_no);
}

async function previewStatement(id, no) {
  const r = await call('render_statement', { id });
  openPreview(r, `對帳單 ${no}`, () => printStatement(id));
}
async function printStatement(id) {
  if (S.settings.print.mode === 'escp') {
    await call('print_statement', { id });
    toast('已送到印表機');
  } else {
    const r = await call('render_statement', { id });
    await driverPrint(r);
  }
}

async function renderStatementList() {
  const c = stCustomer();
  const list = await call('list_statements', { customerId: c ? c.id : null });
  const box = $('#st-list');
  if (!list.length) { box.innerHTML = '<div class="empty">還沒有對帳單。</div>'; return; }
  box.innerHTML = `<table class="grid"><thead><tr><th>單號</th><th>客戶</th><th>期間</th><th class="num">本期金額</th><th class="num">應收</th><th></th></tr></thead><tbody>
    ${list.map((s) => `<tr><td>${esc(s.stmt_no)}</td><td>${esc(s.cust_name)}</td><td>${esc(slashDate(s.period_start))}–${esc(slashDate(s.period_end).slice(5))}</td>
      <td class="num">${fmtInt(s.period_amount)}</td><td class="num">${fmtInt(s.total_due)}</td>
      <td style="white-space:nowrap"><button type="button" class="btn link" data-sv="${s.id}" data-no="${esc(s.stmt_no)}">預覽／列印</button>
        <button type="button" class="btn link" data-void="${s.id}" data-no="${esc(s.stmt_no)}" style="color:var(--danger)">作廢</button></td></tr>`).join('')}
  </tbody></table>`;
}

$('#st-list').addEventListener('click', async (e) => {
  const v = e.target.closest('[data-sv]');
  if (v) return previewStatement(Number(v.dataset.sv), v.dataset.no);
  const x = e.target.closest('[data-void]');
  if (x) {
    if (!(await confirmBox(`作廢對帳單 ${x.dataset.no}？\n裡面的出貨單會解除鎖定，可以重新修改後再建立。`, '作廢'))) return;
    await call('void_statement', { id: Number(x.dataset.void) });
    toast(`已作廢 ${x.dataset.no}`);
    stmtRefresh();
  }
});
$('#st-preview').addEventListener('click', (e) => {
  const tr = e.target.closest('tr[data-id]');
  if (tr) openDoc(Number(tr.dataset.id));
});
$('#st-cust').addEventListener('change', async () => {
  const c = stCustomer();
  if (!c && $('#st-cust').value) toast('找不到這位客戶', 'error');
  await stmtPeriod();
  stmtRefresh();
});
$('#st-month').addEventListener('change', async () => { await stmtPeriod(); stmtPreview(); });
$('#st-start').addEventListener('change', stmtPreview);
$('#st-end').addEventListener('change', stmtPreview);
$('#st-mode').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-mode]');
  if (!b) return;
  ST.mode = b.dataset.mode;
  $$('#st-mode button').forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
});

async function renderPayments() {
  const c = stCustomer();
  const box = $('#pay-list');
  if (!c) { box.innerHTML = '<p class="hint">選擇客戶後顯示收款紀錄。</p>'; return; }
  const list = await call('list_payments', { customerId: c.id });
  box.innerHTML = list.length
    ? list.slice(0, 20).map((p) => `<div class="list-item"><div><div>${esc(slashDate(p.pay_date))}　${fmtInt(p.amount)}</div>
        <div class="hint">${esc([p.method, p.note].filter(Boolean).join('・'))}</div></div>
        <button type="button" class="btn link" data-delpay="${p.id}" style="color:var(--danger)">刪除</button></div>`).join('')
    : '<p class="hint">還沒有收款紀錄。</p>';
}
$('#pay-list').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-delpay]');
  if (!b) return;
  if (!(await confirmBox('刪除這筆收款紀錄？', '刪除'))) return;
  await call('delete_payment', { id: Number(b.dataset.delpay) });
  await loadCustomers();
  stmtRefresh();
});
$('#pay-add').addEventListener('click', async () => {
  const c = stCustomer();
  if (!c) { toast('請先選擇客戶', 'error'); return; }
  const amt = parseNum($('#pay-amount').value);
  if (!amt || Number.isNaN(amt)) { toast('請輸入收款金額', 'error'); return; }
  await call('add_payment', { payment: { id: null, customer_id: c.id, pay_date: $('#pay-date').value || today(), amount: Math.round(amt), method: $('#pay-method').value, note: $('#pay-note').value } });
  $('#pay-amount').value = '';
  $('#pay-note').value = '';
  toast(`已登錄收款 ${fmtInt(amt)}`);
  await loadCustomers();
  stmtRefresh();
});

// ============================================================
// 品項
// ============================================================
let prodSel = null;

function renderProdList() {
  const q = $('#prod-search').value.trim().toLowerCase();
  const showInactive = $('#prod-show-inactive').checked;
  const list = S.products.filter((p) => (showInactive || p.active) && (!q || p.name.toLowerCase().includes(q) || (p.code || '').toLowerCase().includes(q)));
  $('#prod-list').innerHTML = list.length ? `<table class="grid"><thead><tr><th>代號</th><th>品名</th><th>單位</th><th class="num">預設單價</th><th>計價</th><th>狀態</th></tr></thead><tbody>
    ${list.map((p) => `<tr class="click ${p.id === prodSel ? 'selected' : ''} ${p.active ? '' : 'inactive'}" data-id="${p.id}">
      <td>${esc(p.code)}</td><td>${esc(p.name)}</td><td>${esc(p.unit)}</td><td class="num">${p.price === null ? '時價' : fmtNum(p.price)}</td>
      <td>${p.weighed ? '秤重' : '件數'}</td><td>${p.active ? '使用中' : '停用'}</td></tr>`).join('')}</tbody></table>`
    : '<div class="empty">還沒有品項，在右邊填好按「儲存品項」。</div>';
}

function fillProdForm(p) {
  prodSel = p ? p.id : null;
  // 清楚標示目前是「新增」還是「編輯哪一筆」，避免把下一筆打在剛存好的那筆上
  $('#prod-form-title').textContent = p ? `編輯品項：${p.name}` : '新增品項';
  $('#p-save').textContent = p ? '儲存修改' : '新增品項';
  $('#p-to-new').hidden = !p;
  $('#p-code').value = p ? p.code : '';
  $('#p-name').value = p ? p.name : '';
  $('#p-unit').value = p ? p.unit : '';
  $('#p-price').value = p && p.price !== null ? p.price : '';
  $('#p-weighed').checked = p ? p.weighed : false;
  $('#p-active').checked = p ? p.active : true;
  $('#p-name-warn').textContent = '';
  $('#prod-prices').style.display = p ? '' : 'none';
  renderProdList();
  if (p) renderCustPrices();
}

async function renderCustPrices() {
  const list = await call('list_customer_prices', { productId: prodSel });
  $('#cp-list').innerHTML = list.length
    ? list.map((x) => `<div class="list-item"><span>${esc(x.customer_name)}</span><span style="display:flex;align-items:center;gap:6px">${fmtNum(x.price)}
        <button type="button" class="btn link" data-delcp="${x.customer_id}" style="color:var(--danger)">移除</button></span></div>`).join('')
    : '<p class="hint">沒有設定。</p>';
}

async function warnBig5(input, warnEl) {
  const bad = await call('check_text', { text: input.value });
  warnEl.textContent = bad.length ? `印表機可能印不出：${bad.join('、')}（會印成？）` : '';
}

$('#prod-list').addEventListener('click', (e) => {
  const tr = e.target.closest('tr[data-id]');
  if (tr) fillProdForm(S.products.find((p) => p.id === Number(tr.dataset.id)));
});
$('#prod-search').addEventListener('input', renderProdList);
$('#prod-show-inactive').addEventListener('change', renderProdList);
$('#prod-new').addEventListener('click', () => { fillProdForm(null); $('#p-name').focus(); });
$('#p-to-new').addEventListener('click', () => { fillProdForm(null); $('#p-name').focus(); });
$('#p-name').addEventListener('change', (e) => warnBig5(e.target, $('#p-name-warn')));
$('#p-save').addEventListener('click', async () => {
  const price = parseNum($('#p-price').value);
  if (Number.isNaN(price)) { toast('預設單價不是數字', 'error'); return; }
  const prod = { id: prodSel, code: $('#p-code').value, name: $('#p-name').value, unit: $('#p-unit').value, price, weighed: $('#p-weighed').checked, active: $('#p-active').checked };
  const isNew = !prodSel;
  const id = await call('save_product', { product: prod });
  await loadProducts();
  if (isNew) {
    // 新增完自動清空，直接輸入下一筆
    toast(`已新增 ${prod.name.trim()}，可以繼續輸入下一筆`);
    fillProdForm(null);
    $('#p-name').focus();
  } else {
    toast(`已更新 ${prod.name.trim()}`);
    fillProdForm(S.products.find((p) => p.id === id));
  }
});
$('#cp-add').addEventListener('click', async () => {
  const c = findCustomer($('#cp-cust').value);
  const price = parseNum($('#cp-price').value);
  if (!c) { toast('請選擇客戶', 'error'); return; }
  if (price === null || Number.isNaN(price)) { toast('請輸入單價', 'error'); return; }
  await call('set_customer_price', { customerId: c.id, productId: prodSel, price });
  $('#cp-cust').value = ''; $('#cp-price').value = '';
  renderCustPrices();
});
$('#cp-list').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-delcp]');
  if (!b) return;
  await call('set_customer_price', { customerId: Number(b.dataset.delcp), productId: prodSel, price: null });
  renderCustPrices();
});

// ============================================================
// 客戶
// ============================================================
let custSel = null;

function renderCustList() {
  const q = $('#cust-search').value.trim().toLowerCase();
  const showInactive = $('#cust-show-inactive').checked;
  const list = S.customers.filter((c) => (showInactive || c.active) &&
    (!q || c.name.toLowerCase().includes(q) || (c.code || '').toLowerCase().includes(q) || (c.phone || '').includes(q)))
    .sort((a, b) => Number(b.needs_review) - Number(a.needs_review));
  $('#cust-list').innerHTML = list.length ? `<table class="grid"><thead><tr><th>代號</th><th>客戶名稱</th><th>電話</th><th>結帳日</th><th class="num">未收金額</th></tr></thead><tbody>
    ${list.map((c) => {
      const bal = S.balances[c.id] || 0;
      return `<tr class="click ${c.id === custSel ? 'selected' : ''} ${c.active ? '' : 'inactive'}" data-id="${c.id}">
        <td>${esc(c.code)}</td><td>${esc(c.name)}${c.needs_review ? ` <span class="badge review">手持機 ${esc(c.origin)} 新增・待確認</span>` : ''}</td><td>${esc(c.phone)}</td><td>${closingLabel(c.closing_day)}</td>
        <td class="num" style="${bal > 0 ? 'color:var(--warn);font-weight:600' : ''}">${fmtInt(bal)}</td></tr>`;
    }).join('')}</tbody></table>`
    : '<div class="empty">還沒有客戶，在右邊填好按「儲存客戶」。</div>';
}

function fillCustForm(c) {
  custSel = c ? c.id : null;
  $('#cust-form-title').textContent = c ? `編輯客戶：${c.name}` : '新增客戶';
  $('#c-save').textContent = c ? '儲存修改' : '新增客戶';
  $('#c-to-new').hidden = !c;
  $('#c-name').value = c ? c.name : '';
  $('#c-code').value = c ? c.code : '';
  $('#c-taxid').value = c ? c.tax_id : '';
  $('#c-phone').value = c ? c.phone : '';
  $('#c-contact').value = c ? c.contact : '';
  $('#c-address').value = c ? c.address : '';
  $('#c-closing').value = String(c ? c.closing_day : 0);
  $('#c-tax').value = c ? c.tax_mode : 'exclusive';
  $('#c-note').value = c ? c.note : '';
  $('#c-active').checked = c ? c.active : true;
  $('#c-name-warn').textContent = '';
  renderCustList();
  renderCustReview(c);
}

$('#c-closing').innerHTML = '<option value="0">月底</option>' + Array.from({ length: 28 }, (_, i) => `<option value="${i + 1}">每月 ${i + 1} 日</option>`).join('');
$('#cust-list').addEventListener('click', (e) => {
  const tr = e.target.closest('tr[data-id]');
  if (tr) fillCustForm(customerById(Number(tr.dataset.id)));
});
$('#cust-search').addEventListener('input', renderCustList);
$('#cust-show-inactive').addEventListener('change', renderCustList);
$('#cust-new').addEventListener('click', () => { fillCustForm(null); $('#c-name').focus(); });
$('#c-to-new').addEventListener('click', () => { fillCustForm(null); $('#c-name').focus(); });
$('#c-name').addEventListener('change', (e) => warnBig5(e.target, $('#c-name-warn')));
$('#c-save').addEventListener('click', async () => {
  const c = {
    id: custSel, code: $('#c-code').value, name: $('#c-name').value, tax_id: $('#c-taxid').value, phone: $('#c-phone').value,
    contact: $('#c-contact').value, address: $('#c-address').value, closing_day: Number($('#c-closing').value),
    tax_mode: $('#c-tax').value, note: $('#c-note').value, active: $('#c-active').checked,
  };
  const isNew = !custSel;
  const id = await call('save_customer', { customer: c });
  await loadCustomers();
  if (isNew) {
    toast(`已新增 ${c.name.trim()}，可以繼續輸入下一筆`);
    fillCustForm(null);
    $('#c-name').focus();
  } else {
    toast(`已更新 ${c.name.trim()}`);
    fillCustForm(customerById(id));
  }
});

// ============================================================
// 設定
// ============================================================
async function renderSettings() {
  const s = S.settings, p = s.print;
  $('#s-company').value = s.company_name;
  $('#s-maker').value = s.default_maker;
  $('#s-manual-layout').value = s.manual_layout || 'double';
  $('#s-sync-enabled').checked = s.sync_enabled !== false;
  $('#s-sync-port').value = s.sync_port || 47110;
  renderSyncCard();
  $('#s-sign').value = s.signatures.join(',');
  $('#s-extra').value = s.extra_backup_dir;
  $('#s-mode').value = p.mode;
  $('#s-lines').value = p.page_lines;
  $('#s-spacing').value = p.line_spacing_180;
  $('#s-top').value = p.top_lines;
  $('#s-bottom').value = p.bottom_lines;
  $('#s-left').value = p.left_margin_cols;
  $('#s-cols').value = p.total_cols;
  $('#s-cpi').value = String(p.cpi);
  $('#s-chinese').checked = p.chinese_cmd;
  $('#s-paper').value = p.paper_mode || 'half';
  $('#s-pw').value = p.driver_paper_w_cm;
  $('#s-ph').value = p.driver_paper_h_cm;
  $('#s-continuous').checked = p.continuous !== false;
  $('#s-datadir').textContent = S.info.data_dir;
  $('#s-lastbackup').textContent = S.info.last_backup ? `最近一次備份：${S.info.last_backup}` : '還沒有備份。';
  $('#s-version').textContent = `版本 ${S.info.version}`;
  try {
    const pr = await invoke('list_printers');
    const opts = ['<option value="">（使用 Windows 預設印表機' + (pr.default ? `：${esc(pr.default)}` : '') + '）</option>']
      .concat(pr.printers.map((n) => `<option value="${esc(n)}">${esc(n)}</option>`));
    if (p.printer_name && !pr.printers.includes(p.printer_name)) opts.push(`<option value="${esc(p.printer_name)}">${esc(p.printer_name)}（目前找不到）</option>`);
    $('#s-printer').innerHTML = opts.join('');
  } catch (e) {
    $('#s-printer').innerHTML = `<option value="${esc(p.printer_name)}">${esc(p.printer_name || '（無法取得印表機清單）')}</option>`;
  }
  $('#s-printer').value = p.printer_name;
}

function readSettingsForm() {
  const n = (id) => Number($(id).value);
  return {
    company_name: $('#s-company').value.trim(),
    default_maker: $('#s-maker').value.trim(),
    manual_layout: $('#s-manual-layout').value,
    signatures: $('#s-sign').value.split(/[,，、]/).map((x) => x.trim()).filter(Boolean),
    extra_backup_dir: $('#s-extra').value.trim(),
    sync_enabled: $('#s-sync-enabled').checked,
    sync_port: n('#s-sync-port') || 47110,
    print: {
      mode: $('#s-mode').value, printer_name: $('#s-printer').value, page_lines: n('#s-lines'), line_spacing_180: n('#s-spacing'),
      top_lines: n('#s-top'), bottom_lines: n('#s-bottom'), left_margin_cols: n('#s-left'), total_cols: n('#s-cols'),
      cpi: n('#s-cpi'), chinese_cmd: $('#s-chinese').checked, paper_mode: $('#s-paper').value,
      driver_paper_w_cm: Number($('#s-pw').value) || 21, driver_paper_h_cm: Number($('#s-ph').value) || 14,
      continuous: $('#s-continuous').checked,
    },
  };
}

async function saveSettings(silent) {
  const s = readSettingsForm();
  const syncChanged = !S.settings || s.sync_enabled !== S.settings.sync_enabled || s.sync_port !== S.settings.sync_port;
  await call('save_settings', { settings: s });
  S.settings = s;
  if (syncChanged) { await call('restart_sync_server'); renderSyncCard(); }
  $('#brand').textContent = s.company_name || '出貨對帳系統';
  if (!silent) toast('設定已儲存');
}

$('#set-save').addEventListener('click', () => saveSettings(false));
$('#set-test-preview').addEventListener('click', async () => {
  await saveSettings(true);
  const r = await call('render_test_page');
  openPreview(r, '對位測試頁', async () => {
    if (S.settings.print.mode === 'escp') { await call('print_test_page'); toast('已送出測試頁'); }
    else await driverPrint(r);
  });
});
$('#set-test-print').addEventListener('click', async () => {
  await saveSettings(true);
  if (S.settings.print.mode === 'escp') { await call('print_test_page'); toast('已送出測試頁'); }
  else await driverPrint(await call('render_test_page'));
});
$('#set-backup').addEventListener('click', async () => {
  await saveSettings(true);
  const path = await call('backup_now');
  S.info = await call('app_info');
  renderSettings();
  toast(`已備份到 ${path}`);
});

// ============================================================
// 啟動
// ============================================================
$('#nav').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-view]');
  if (b) showView(b.dataset.view);
});

async function boot() {
  try {
    S.info = await invoke('app_info');
  } catch (e) {
    document.body.innerHTML = `<div style="padding:40px;font-size:16px">${esc(e)}</div>`;
    return;
  }
  if (S.info.init_error) {
    document.body.innerHTML = `<div style="padding:40px;font-size:16px;line-height:1.8"><h1>資料庫無法開啟</h1><p>${esc(S.info.init_error)}</p><p>資料位置：${esc(S.info.data_dir)}</p></div>`;
    return;
  }
  S.settings = await call('get_settings');
  $('#brand').textContent = S.settings.company_name || '出貨對帳系統';
  $('#nav-foot').textContent = S.info.last_backup ? `上次備份 ${S.info.last_backup}` : '';
  await Promise.all([loadCustomers(), loadProducts()]);
  $('#ship-f-month').value = thisMonth();
  fillProdForm(null);
  fillCustForm(null);
  renderShipList();
}

boot();
