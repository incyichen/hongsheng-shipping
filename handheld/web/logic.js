/* 手機：資料與同步邏輯（不碰畫面）
 * 在 Android 上由原生外殼提供 Native（存檔、網路、列印）；測試時可用替身。
 * 計算規則跟電腦版（core/src/util.rs）一致：每列金額四捨五入到元，稅金依客戶稅別。
 */
(function (root) {
  'use strict';

  // ---------------- 小工具 ----------------
  const pad2 = (n) => String(n).padStart(2, '0');
  function today() {
    const d = new Date();
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  }
  function nowText() {
    const d = new Date();
    return `${today()} ${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
  }
  function newUid() {
    const b = new Uint8Array(16);
    (root.crypto || require('crypto').webcrypto).getRandomValues(b);
    return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
  }
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
  const halfwidth = (s) => String(s ?? '').replace(/[０-９]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xFEE0)).replace(/．/g, '.');
  function parseNum(s) {
    const t = halfwidth(s).replace(/[,，\s]/g, '');
    if (t === '') return null;
    const v = Number(t);
    return Number.isFinite(v) ? v : NaN;
  }
  // 「3」「2.5」「2斤5兩」「2斤」「5兩」「1斤8」→ 斤（1 斤 = 16 兩）
  function parseQty(text) {
    const s = halfwidth(text).replace(/\s|　/g, '');
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
    } catch (e) { return NaN; }
  }
  const lineAmount = (qty, price) => Math.round(qty * price);
  function totalsFromSum(sum, taxMode) {
    if (taxMode === 'inclusive') { const sub = Math.round(sum / 1.05); return { sub, tax: sum - sub, total: sum }; }
    if (taxMode === 'none') return { sub: sum, tax: 0, total: sum };
    const tax = Math.round(sum * 0.05);
    return { sub: sum, tax, total: sum + tax };
  }
  const TAX_LABEL = { exclusive: '外加 5%', inclusive: '內含 5%', none: '免稅' };

  // ---------------- 本機資料 ----------------
  function emptyState() {
    return {
      version: 1,
      config: { server: '', token: '', prefix: '', deviceName: '', companyName: '泓昇海鮮', maker: '', printer: 'auto' },
      since: 0,
      customers: {},        // uid → WireCustomer（含手機新增、尚未上傳的）
      products: {},         // uid → WireProduct
      customerPrices: [],   // 客戶專屬價
      lastPrices: [],       // 上次成交價
      docs: {},             // uid → { doc: WireDoc, base_rev, dirty, deleted, error }
      pendingCustomers: {}, // uid → true：還沒上傳的新客戶
      pendingProducts: {},  // uid → true：還沒上傳的新品項
      merges: {},           // 舊客戶 uid → 新客戶 uid
      shipped: {},          // uid → 時間：手機已列印給客戶、還沒告訴電腦的單
      seq: {},              // 年月 → 已用到的流水號
      lastSyncAt: '',
      lastSyncError: '',
    };
  }

  function resolveCustomer(state, uid) {
    let cur = uid;
    for (let i = 0; i < 20 && state.merges[cur]; i++) cur = state.merges[cur];
    return cur;
  }

  const customerList = (state) => Object.values(state.customers).filter((c) => c.active !== false)
    .sort((a, b) => Number(!!b.needs_review) - Number(!!a.needs_review) || (a.code || 'zzzz').localeCompare(b.code || 'zzzz') || a.name.localeCompare(b.name));
  const productList = (state) => Object.values(state.products).filter((p) => p.active !== false)
    .sort((a, b) => (a.code || 'zzzz').localeCompare(b.code || 'zzzz') || a.name.localeCompare(b.name));

  // 修改既有客戶（代號、結帳日只能在電腦改）。同步時電腦上沒改過才套用，否則以電腦為準
  function updateCustomer(state, uid, f) {
    const c = state.customers[uid];
    if (!c) throw new Error('找不到這個客戶');
    const name = String(f.name ?? c.name).trim();
    if (!name) throw new Error('客戶名稱不可空白');
    Object.assign(c, {
      name,
      phone: String(f.phone ?? c.phone ?? '').trim(),
      address: String(f.address ?? c.address ?? '').trim(),
      tax_id: String(f.tax_id ?? c.tax_id ?? '').trim(),
      note: String(f.note ?? c.note ?? ''),
      tax_mode: ['exclusive', 'inclusive', 'none'].includes(f.tax_mode) ? f.tax_mode : c.tax_mode,
    });
    state.pendingCustomers[uid] = true;
    return c;
  }

  function addCustomer(state, { name, phone, address, tax_mode, note, tax_id }) {
    name = String(name || '').trim();
    if (!name) throw new Error('客戶名稱不可空白');
    const c = { uid: newUid(), code: '', name, tax_id: String(tax_id || '').trim(), phone: String(phone || '').trim(), contact: '', address: String(address || '').trim(),
      closing_day: 0, tax_mode: ['inclusive', 'none'].includes(tax_mode) ? tax_mode : 'exclusive', note: String(note || ''), active: true,
      origin: state.config.prefix, needs_review: true, rev: 0 };
    state.customers[c.uid] = c;
    state.pendingCustomers[c.uid] = true;
    return c;
  }

  // 開單時打了清單沒有的品名：本機先新增，同步時上傳（電腦上已有同名品項會自動對應）
  function ensureProduct(state, { name, unit, price, qtyText }) {
    name = String(name || '').trim();
    const same = Object.values(state.products).find((p) => p.name === name);
    if (same) return same;
    const p = { uid: newUid(), code: '', name, unit: String(unit || '').trim(), price: price > 0 ? price : null,
      weighed: /[斤兩]/.test(String(unit || '') + String(qtyText || '')), active: true, origin: state.config.prefix, rev: 0 };
    state.products[p.uid] = p;
    state.pendingProducts[p.uid] = true;
    return p;
  }

  // 單價建議：客戶專屬價 → 此客戶上次成交價 → 品項預設價（都沒有＝時價，留空）
  function suggestPrice(state, customerUid, productUid) {
    const cu = resolveCustomer(state, customerUid);
    const special = state.customerPrices.find((x) => x.customer_uid === cu && x.product_uid === productUid);
    if (special) return { price: special.price, source: '客戶專屬價' };
    // 本機已開立的單可能比電腦送來的上次成交價更新
    let best = null;
    for (const r of Object.values(state.docs)) {
      const d = r.doc;
      if (r.deleted || d.status !== 'issued' || resolveCustomer(state, d.customer_uid) !== cu) continue;
      for (const l of d.lines) if (l.product_uid === productUid && (!best || d.doc_date > best.date)) best = { price: l.price, date: d.doc_date };
    }
    const last = state.lastPrices.find((x) => x.customer_uid === cu && x.product_uid === productUid);
    if (last && (!best || last.doc_date > best.date)) best = { price: last.price, date: last.doc_date };
    if (best) return { price: best.price, source: `上次成交價（${best.date.slice(5).replace('-', '/')}）` };
    const p = state.products[productUid];
    if (p && p.price !== null && p.price !== undefined) return { price: p.price, source: '預設價' };
    return { price: null, source: '時價，請輸入' };
  }

  // 單號：V1-202610-0001，依單據日期的年月編號；重裝後由電腦告知已用到幾號
  function nextDocNo(state, docDate) {
    const ym = docDate.slice(0, 4) + docDate.slice(5, 7);
    const used = Object.values(state.docs).map((r) => r.doc.doc_no)
      .filter((no) => no.startsWith(`${state.config.prefix}-${ym}-`)).map((no) => Number(no.split('-')[2]) || 0);
    const n = Math.max(state.seq[ym] || 0, ...used, 0) + 1;
    state.seq[ym] = n;
    return `${state.config.prefix}-${ym}-${String(n).padStart(4, '0')}`;
  }

  function newDoc(state, customerUid) {
    const c = state.customers[customerUid];
    return { uid: newUid(), doc_no: '', kind: 'shipment', customer_uid: customerUid, doc_date: today(), maker: state.config.maker || '',
      note: '', status: 'draft', locked: false, statement_no: null, cust_name: c ? c.name : '', cust_phone: c ? c.phone : '',
      cust_address: c ? c.address : '', tax_mode: c ? c.tax_mode : 'exclusive', subtotal: 0, tax: 0, total: 0,
      origin: state.config.prefix, shipped_at: '', rev: 0, lines: [] };
  }

  // 依目前的客戶稅別重算金額（電腦收到後也會用同樣規則重算）
  function recalc(state, doc) {
    const c = state.customers[resolveCustomer(state, doc.customer_uid)];
    if (c) { doc.cust_name = c.name; doc.cust_phone = c.phone; doc.cust_address = c.address; doc.tax_mode = c.tax_mode; }
    let sum = 0;
    for (const l of doc.lines) {
      const q = parseQty(l.qty_text);
      l.qty = q === null || Number.isNaN(q) ? 0 : q;
      l.amount = l.price === null || l.price === undefined || l.price === '' ? 0 : lineAmount(l.qty, Number(l.price));
      sum += l.amount;
    }
    const t = totalsFromSum(sum, doc.tax_mode);
    doc.subtotal = t.sub; doc.tax = t.tax; doc.total = t.total;
    return doc;
  }

  // 檢查：草稿可以缺單價；開立時每一項都要有數量與單價
  function validate(doc, issuing) {
    if (!doc.customer_uid) return '請先選擇客戶';
    if (issuing && !doc.lines.length) return '至少要有一個品項';
    for (const [i, l] of doc.lines.entries()) {
      if (!String(l.name || '').trim()) return `第 ${i + 1} 項沒有品名`;
      const q = parseQty(l.qty_text);
      if (q === null || Number.isNaN(q)) return `「${l.name}」的數量不正確`;
      if (issuing && (l.price === null || l.price === undefined || l.price === '')) return `「${l.name}」還沒有單價`;
    }
    return '';
  }

  // 存檔（草稿或開立）：第一次存檔時給單號；標記為待上傳
  function saveDoc(state, doc, issuing) {
    const err = validate(doc, issuing);
    if (err) throw new Error(err);
    if (issuing) doc.status = 'issued';
    for (const l of doc.lines) {
      if (!l.product_uid || !state.products[l.product_uid]) {
        l.product_uid = ensureProduct(state, { name: l.name, unit: l.unit, price: Number(l.price) || 0, qtyText: l.qty_text }).uid;
      }
    }
    recalc(state, doc);
    // 電腦端單價是數字：草稿的空白單價存成 0（跟電腦版一樣，草稿 0 元＝還沒填）
    for (const l of doc.lines) if (l.price === null || l.price === undefined || l.price === '') l.price = 0;
    if (!doc.doc_no) doc.doc_no = nextDocNo(state, doc.doc_date);
    const rec = state.docs[doc.uid] || { base_rev: null };
    state.docs[doc.uid] = { doc, base_rev: rec.base_rev, dirty: true, deleted: false, error: '' };
    return doc;
  }

  function deleteDoc(state, uid) {
    const r = state.docs[uid];
    if (!r) return;
    if (r.doc.status === 'issued' && r.base_rev !== null) throw new Error('已開立並同步的單不能在手機刪除，請在電腦上處理');
    if (r.base_rev === null && r.doc.status === 'draft') { delete state.docs[uid]; return; } // 從沒上傳過，直接刪
    r.deleted = true; r.dirty = true;
  }

  // 已列入對帳單的單不能改；其他（包含電腦開好的已開立單）都可以在手機核對修改
  const canEdit = (r) => !r.doc.locked;
  // 從沒上傳過的草稿或手機剛開的單才能刪；電腦上已有的已開立單要在電腦處理
  const canDelete = (r) => !r.doc.locked && !(r.doc.status === 'issued' && r.base_rev !== null);

  // 列印給客戶＝出貨。只記第一次的時間
  function markShipped(state, uid) {
    const r = state.docs[uid];
    state.shipped = state.shipped || {};
    if (!r || r.doc.shipped_at) return;
    const at = nowText();
    r.doc.shipped_at = at;
    state.shipped[uid] = at;
  }

  function pendingCount(state) {
    return Object.keys(state.pendingCustomers).length + Object.values(state.docs).filter((r) => r.dirty).length +
      Object.keys(state.shipped || {}).filter((u) => !(state.docs[u] && state.docs[u].dirty)).length;
  }

  // ---------------- 同步 ----------------
  function buildRequest(state) {
    state.shipped = state.shipped || {};
    return {
      since_rev: state.since,
      customers: Object.keys(state.pendingCustomers).map((u) => state.customers[u]).filter(Boolean),
      products: Object.keys(state.pendingProducts).map((u) => state.products[u]).filter(Boolean),
      documents: Object.values(state.docs).filter((r) => r.dirty).map((r) => ({ doc: r.doc, base_rev: r.base_rev, deleted: !!r.deleted })),
      shipped: Object.entries(state.shipped || {}).map(([uid, at]) => ({ uid, at })),
    };
  }

  // 套用電腦的回覆。sent＝這次送出去的內容（用來判斷哪些上傳成功）
  function applyResponse(state, resp, sent) {
    const summary = { uploaded: 0, conflicts: [], rejected: [], received: 0 };
    const results = Object.fromEntries((resp.results || []).map((r) => [r.uid, r]));

    for (const p of sent.products) {
      const r = results[p.uid];
      if (r && r.status !== 'rejected') delete state.pendingProducts[p.uid];
    }
    for (const c of sent.customers) {
      const r = results[c.uid];
      if (!r) continue;
      // 同步中又修改過，就留到下次再送
      const changed = JSON.stringify(state.customers[c.uid]) !== JSON.stringify(c);
      if (r.status === 'rejected') summary.rejected.push(`${c.name}：${r.message}`);
      else if (!changed) delete state.pendingCustomers[c.uid];
      if (r.status === 'conflict') summary.conflicts.push(`客戶「${c.name}」：${r.message}`);
    }
    for (const m of sent.shipped || []) if (state.shipped[m.uid] === m.at) delete state.shipped[m.uid];
    // 送出的單：成功或衝突都以電腦回傳的版本為準；被拒絕的保留在手機、顯示原因
    for (const pd of sent.documents) {
      const r = results[pd.doc.uid];
      const rec = state.docs[pd.doc.uid];
      if (!r || !rec) continue;
      if (r.status === 'rejected') { rec.error = r.message; summary.rejected.push(`${pd.doc.doc_no}：${r.message}`); continue; }
      if (r.status === 'conflict') summary.conflicts.push(`${pd.doc.doc_no}：${r.message}`); else summary.uploaded++;
      // 這段時間又被修改過（例如同步中再存檔），就保留修改、下次再送
      if (JSON.stringify(rec.doc) !== JSON.stringify(pd.doc) || rec.deleted !== pd.deleted) continue;
      if (pd.deleted || (r.status === 'ignored' && pd.deleted)) { delete state.docs[pd.doc.uid]; continue; }
      rec.dirty = false; rec.error = '';
    }

    // 品項對應（要在比對「同步中是否又修改過」之後才做，不然會被誤判成有修改）
    // 手機新增的品項在電腦上已有同名 → 改用電腦的品項
    for (const [from, to] of Object.entries(resp.product_remaps || {})) {
      for (const r of Object.values(state.docs)) for (const l of r.doc.lines) if (l.product_uid === from) l.product_uid = to;
      delete state.products[from];
      delete state.pendingProducts[from];
    }
    if (resp.full) {
      // 第一次（或重新）同步：電腦送來的就是全部
      const keepCust = Object.fromEntries(Object.keys(state.pendingCustomers).map((u) => [u, state.customers[u]]));
      const keepProd = Object.fromEntries(Object.keys(state.pendingProducts).map((u) => [u, state.products[u]]));
      state.customers = { ...keepCust };
      state.products = { ...keepProd };
      for (const [u, r] of Object.entries(state.docs)) if (!r.dirty) delete state.docs[u];
    }
    for (const c of resp.customers || []) { if (!state.pendingCustomers[c.uid]) state.customers[c.uid] = c; summary.received++; }
    for (const p of resp.products || []) { if (!state.pendingProducts[p.uid]) state.products[p.uid] = p; summary.received++; }
    for (const d of resp.documents || []) {
      const rec = state.docs[d.uid];
      if (rec && rec.dirty) continue; // 手機上還有沒送出的修改：保留，下次上傳時由電腦判斷
      if (!d.shipped_at && state.shipped[d.uid]) d.shipped_at = state.shipped[d.uid]; // 剛印、還沒告訴電腦
      state.docs[d.uid] = { doc: d, base_rev: d.rev, dirty: false, deleted: false, error: '' };
      summary.received++;
    }
    for (const del of resp.deleted || []) {
      if (del.tbl === 'customers') delete state.customers[del.uid];
      if (del.tbl === 'products') delete state.products[del.uid];
      if (del.tbl === 'documents') { const rec = state.docs[del.uid]; if (rec && !rec.dirty) delete state.docs[del.uid]; }
    }
    state.merges = { ...state.merges, ...(resp.customer_merges || {}) };
    for (const r of Object.values(state.docs)) if (r.dirty) r.doc.customer_uid = resolveCustomer(state, r.doc.customer_uid);
    state.customerPrices = resp.customer_prices || [];
    state.lastPrices = resp.last_prices || [];
    for (const [ym, n] of Object.entries(resp.prefix_last_seq || {})) state.seq[ym] = Math.max(state.seq[ym] || 0, n);
    if (resp.company_name) state.config.companyName = resp.company_name;
    if (resp.device_prefix) state.config.prefix = resp.device_prefix;
    state.since = resp.server_rev;
    state.lastSyncAt = nowText();
    state.lastSyncError = '';
    prune(state);
    return summary;
  }

  // 已同步、已開立、超過 60 天的單從手機移除（電腦上都還在）
  function prune(state, days = 60) {
    const d = new Date(); d.setDate(d.getDate() - days);
    const cutoff = `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
    for (const [u, r] of Object.entries(state.docs)) if (!r.dirty && !state.shipped[u] && r.doc.status === 'issued' && r.doc.doc_date < cutoff) delete state.docs[u];
  }

  function baseUrl(server) {
    let s = String(server || '').trim();
    if (!s) throw new Error('還沒設定電腦位址');
    if (!/^https?:\/\//.test(s)) s = 'http://' + s;
    if (!/:\d+$/.test(s.replace(/\/+$/, ''))) s = s.replace(/\/+$/, '') + ':47110';
    return s.replace(/\/+$/, '');
  }

  // http(method, url, headers, body) → Promise<{status, body}>，由 Native 或測試替身提供
  async function pair(state, http, server, code, deviceName) {
    const url = baseUrl(server);
    const r = await http('POST', url + '/api/pair', { 'Content-Type': 'application/json' }, JSON.stringify({ code: String(code).trim(), device_name: deviceName }));
    const body = r.body ? JSON.parse(r.body) : {};
    if (r.status !== 200) throw new Error(body.error || `配對失敗（${r.status}）`);
    Object.assign(state.config, { server: url, token: body.token, prefix: body.prefix, deviceName: body.device_name, companyName: body.company_name || state.config.companyName });
    state.since = 0;
    return body;
  }

  async function sync(state, http) {
    if (!state.config.token) throw new Error('還沒跟電腦配對');
    const sent = JSON.parse(JSON.stringify(buildRequest(state)));
    let r;
    try {
      r = await http('POST', baseUrl(state.config.server) + '/api/sync',
        { 'Content-Type': 'application/json', Authorization: 'Bearer ' + state.config.token }, JSON.stringify(sent));
    } catch (e) {
      state.lastSyncError = '連不到電腦（離線中）';
      throw new Error(state.lastSyncError);
    }
    const body = r.body ? JSON.parse(r.body) : {};
    if (r.status === 401) { state.lastSyncError = body.error || '手機已被停用或需要重新配對'; throw new Error(state.lastSyncError); }
    if (r.status !== 200) { state.lastSyncError = body.error || `同步失敗（${r.status}）`; throw new Error(state.lastSyncError); }
    return applyResponse(state, body, sent);
  }

  const api = { pad2, today, nowText, newUid, fmtInt, fmtNum, parseNum, parseQty, lineAmount, totalsFromSum, TAX_LABEL,
    emptyState, resolveCustomer, customerList, productList, addCustomer, updateCustomer, canDelete, markShipped, ensureProduct, suggestPrice, nextDocNo, newDoc,
    recalc, validate, saveDoc, deleteDoc, canEdit, pendingCount, buildRequest, applyResponse, prune, baseUrl, pair, sync };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.HH = api;
})(typeof window !== 'undefined' ? window : globalThis);
