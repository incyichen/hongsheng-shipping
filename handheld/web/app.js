/* 手持機畫面：開單、選客戶／品項、填數量單價、開立列印、同步 */
(function () {
  'use strict';
  const H = window.HH;
  const NB = window.NativeBridge;
  const $ = (id) => document.getElementById(id);
  const clone = (x) => JSON.parse(JSON.stringify(x));
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const noop = () => {};
  const blankPrice = (p) => p === null || p === undefined || p === '';

  // ---------------- 本機資料 ----------------
  function load() {
    try {
      const t = NB.loadState();
      if (t) {
        const s = JSON.parse(t);
        const base = H.emptyState();
        return Object.assign(base, s, { config: Object.assign(base.config, s.config || {}) });
      }
    } catch (e) { /* 檔案壞掉就重新開始 */ }
    return H.emptyState();
  }
  let S = load();
  function persist() {
    try { NB.saveState(JSON.stringify(S)); } catch (e) { toast('存檔失敗：' + e.message, true); }
  }

  // ---------------- 提示、確認 ----------------
  function toast(msg, isError, ms) {
    const el = document.createElement('div');
    el.className = 'toast' + (isError ? ' error' : '');
    el.textContent = msg;
    $('toast').appendChild(el);
    setTimeout(() => el.remove(), ms || (isError ? 5000 : 2600));
  }
  let confirmResolve = null;
  function ask(msg, okText) {
    $('confirm-msg').textContent = msg;
    $('confirm-ok').textContent = okText || '確定';
    $('confirm-back').classList.add('open');
    return new Promise((res) => { confirmResolve = res; });
  }
  function closeAsk(v) {
    $('confirm-back').classList.remove('open');
    const r = confirmResolve; confirmResolve = null;
    if (r) r(v);
  }
  $('confirm-ok').onclick = () => closeAsk(true);
  $('confirm-cancel').onclick = () => closeAsk(false);

  // ---------------- 畫面切換 ----------------
  const stack = [];
  const enter = {};
  const leave = {};
  function show(id) {
    document.querySelectorAll('.view').forEach((v) => v.classList.toggle('active', v.id === id));
    window.scrollTo(0, 0);
    (enter[id] || noop)();
  }
  function go(id) { stack.push(id); show(id); }
  function top() { return stack[stack.length - 1]; }
  function back() {
    if ($('confirm-back').classList.contains('open')) { closeAsk(false); return true; }
    if (stack.length <= 1) return false;
    const guard = leave[top()];
    if (guard && guard() === false) return true;
    stack.pop();
    show(top());
    return true;
  }
  // 退回到指定畫面（不經過離開確認）
  function popTo(id) {
    while (stack.length > 1 && top() !== id) stack.pop();
    if (top() !== id) stack.push(id);
    show(id);
  }
  function home() { stack.length = 0; go('v-home'); }
  document.querySelectorAll('[data-back]').forEach((b) => { b.onclick = () => back(); });
  window.onAndroidBack = () => back(); // 回傳 false 時由 Android 關閉 App

  // ---------------- 同步 ----------------
  let syncing = false;
  let syncAgain = false;
  async function doSync(manual) {
    if (!S.config.token) return;
    if (syncing) { syncAgain = syncAgain || !!manual || true; return; }
    syncing = true;
    renderChip();
    try {
      const sum = await H.sync(S, NB.http);
      persist();
      for (const m of sum.conflicts) toast('有衝突，已交給電腦處理：' + m, true, 7000);
      for (const m of sum.rejected) toast('電腦沒有收下：' + m, true, 7000);
      if (manual && !sum.conflicts.length && !sum.rejected.length) toast(sum.uploaded ? `同步完成，上傳 ${sum.uploaded} 筆` : '同步完成');
    } catch (e) {
      persist();
      if (manual) toast(e.message, true);
    } finally {
      syncing = false;
      renderChip();
      refreshAfterSync();
      if (syncAgain) { syncAgain = false; setTimeout(() => doSync(false), 300); }
    }
  }
  function refreshAfterSync() {
    if (top() === 'v-home') renderHome();
    if (top() === 'v-settings') renderSettings();
  }
  setInterval(() => doSync(false), 120000);
  window.onNetworkAvailable = () => doSync(false);
  window.onNativeResume = () => doSync(false);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) doSync(false); });

  function renderChip() {
    const chip = $('sync-chip');
    if (!chip) return;
    const n = H.pendingCount(S);
    let cls = 'sync-chip';
    let text;
    let act = '立即同步';
    if (syncing) { cls += ' busy'; text = '同步中…'; act = ''; }
    else if (S.lastSyncError) {
      cls += S.lastSyncError.includes('離線') ? ' offline' : ' error';
      text = S.lastSyncError + (n ? `・${n} 筆待上傳` : '');
      act = '重試';
    } else if (n) { cls += ' pending'; text = `${n} 筆待上傳`; }
    else text = S.lastSyncAt ? `已同步 ${S.lastSyncAt.slice(5, 16).replace('-', '/')}` : '尚未同步';
    chip.className = cls;
    chip.innerHTML = `<span class="dot"></span><span>${esc(text)}</span>${act ? `<span class="act">${act}</span>` : ''}`;
  }
  $('sync-chip').onclick = () => doSync(true);

  // ---------------- 共用顯示 ----------------
  const qtyText = (l) => {
    const q = l.qty_text || H.fmtNum(l.qty);
    return /[斤兩]/.test(q) ? q : q + (l.unit || '');
  };
  // 草稿裡單價 0 視為還沒填
  const missingCount = (doc) => doc.status === 'draft' ? doc.lines.filter((l) => blankPrice(l.price) || Number(l.price) === 0).length : 0;
  function totalsHtml(doc) {
    const r = (k, v, cls) => `<div class="r${cls ? ' ' + cls : ''}"><span>${k}</span><span>${v}</span></div>`;
    if (doc.tax_mode === 'inclusive') return r('總金額（內含稅）', H.fmtInt(doc.total), 'grand') + r('其中稅金 5%', H.fmtInt(doc.tax));
    if (doc.tax_mode === 'none') return r('總金額（免稅）', H.fmtInt(doc.total), 'grand');
    return r('小計', H.fmtInt(doc.subtotal)) + r('稅金 5%', H.fmtInt(doc.tax)) + r('總金額', H.fmtInt(doc.total), 'grand');
  }

  // ---------------- 首頁 ----------------
  enter['v-home'] = () => renderHome();
  function docCard(r) {
    const d = r.doc;
    const badges = [];
    const miss = missingCount(d);
    if (d.status === 'draft') badges.push('<span class="badge draft">草稿</span>');
    if (miss) badges.push(`<span class="badge need">缺 ${miss} 項單價</span>`);
    if (d.locked) badges.push('<span class="badge lock">已入對帳單</span>');
    if (r.error) badges.push('<span class="badge err">電腦沒收下</span>');
    else if (r.dirty) badges.push('<span class="badge up">待上傳</span>');
    if (!d.origin) badges.push('<span class="badge">電腦開的</span>');
    return `<button type="button" class="doc-card" data-uid="${esc(d.uid)}">
      <span class="row1"><span class="cust">${esc(d.cust_name || '（未選客戶）')}</span><span class="amt">${miss ? '—' : H.fmtInt(d.total)}</span></span>
      <span class="row2"><span>${esc(d.doc_no || '')}</span><span>${esc(d.doc_date.slice(5).replace('-', '/'))}</span><span>${d.lines.length} 項</span>${badges.join('')}</span>
    </button>`;
  }
  function renderHome() {
    $('home-title').textContent = S.config.companyName || '出貨單';
    renderChip();
    const recs = Object.values(S.docs).filter((r) => !r.deleted && r.doc.kind !== 'statement');
    const byNew = (a, b) => b.doc.doc_date.localeCompare(a.doc.doc_date) || b.doc.doc_no.localeCompare(a.doc.doc_no);
    const drafts = recs.filter((r) => r.doc.status === 'draft').sort(byNew);
    const issued = recs.filter((r) => r.doc.status !== 'draft').sort(byNew);
    const t = H.today();
    const todays = issued.filter((r) => r.doc.doc_date === t);
    const older = issued.filter((r) => r.doc.doc_date !== t);
    let html = '';
    if (drafts.length) html += `<p class="section-title">草稿（${drafts.length}）</p>` + drafts.map(docCard).join('');
    if (todays.length) html += `<p class="section-title">今天開立（${todays.length} 張，${H.fmtInt(todays.reduce((s, r) => s + r.doc.total, 0))} 元）</p>` + todays.map(docCard).join('');
    if (older.length) html += '<p class="section-title">之前開立</p>' + older.slice(0, 80).map(docCard).join('');
    if (!html) html = '<div class="empty">還沒有出貨單。<br>電腦上開的草稿同步後會出現在這裡，<br>也可以按下方「開新出貨單」。</div>';
    $('home-list').innerHTML = html;
    $('home-list').querySelectorAll('.doc-card').forEach((b) => { b.onclick = () => openDoc(b.dataset.uid); });
  }
  $('home-new').onclick = () => { pickMode = 'new'; go('v-customers'); };
  $('home-settings').onclick = () => go('v-settings');

  // ---------------- 開單／編輯 ----------------
  let cur = null; // { doc, isNew, changed, readonly }
  function openDoc(uid) {
    const r = S.docs[uid];
    if (!r) return;
    const doc = clone(r.doc);
    if (doc.status === 'draft') doc.lines.forEach((l) => { if (Number(l.price) === 0) l.price = null; });
    cur = { doc, isNew: false, changed: false, readonly: !H.canEdit(r) };
    go('v-doc');
  }
  enter['v-doc'] = () => renderDoc();
  leave['v-doc'] = () => {
    if (cur && cur.changed && cur.doc.lines.length) {
      ask('這張單還沒存檔，要放棄修改嗎？', '放棄修改').then((ok) => { if (ok) { cur.changed = false; back(); } });
      return false;
    }
    return true;
  };
  function renderDoc() {
    const d = cur.doc;
    const rec = S.docs[d.uid];
    H.recalc(S, d);
    $('doc-title').textContent = d.doc_no || '新出貨單';
    $('doc-delete').hidden = cur.isNew || cur.readonly;
    let banner = '';
    if (rec && rec.error) banner += `<div class="banner warn">電腦沒有收下這張單：${esc(rec.error)}</div>`;
    if (d.locked) banner += `<div class="banner">已列入對帳單 ${esc(d.statement_no || '')}，不能再修改，只能補印。</div>`;
    else if (cur.readonly) banner += '<div class="banner">已開立並同步到電腦，只能補印。要修改請在電腦上處理。</div>';
    else if (d.status === 'draft' && missingCount(d)) banner += '<div class="banner warn">有品項還沒填單價（橘色框）。填好後按「開立並列印」。</div>';
    $('doc-banner').innerHTML = banner;

    const c = S.customers[H.resolveCustomer(S, d.customer_uid)];
    const cp = $('doc-customer');
    cp.disabled = cur.readonly;
    cp.classList.toggle('empty-pick', !d.customer_uid);
    cp.innerHTML = d.customer_uid
      ? `<span class="label">客戶・${esc(d.doc_date.replace(/-/g, '/'))}</span><span class="value">${esc(d.cust_name)}</span><span class="sub">${esc(H.TAX_LABEL[d.tax_mode] || '')}${c && c.phone ? '・' + esc(c.phone) : ''}${cur.readonly ? '' : '・點這裡換客戶'}</span>`
      : '<span class="label">客戶</span><span class="value">點這裡選客戶</span>';

    $('doc-lines').innerHTML = d.lines.length ? d.lines.map((l, i) => {
      const miss = blankPrice(l.price);
      return `<button type="button" class="line-card${miss ? ' missing' : ''}" data-i="${i}" ${cur.readonly ? 'disabled' : ''}>
        <span class="name">${esc(l.name)}</span>
        <span class="calc"><span>${esc(qtyText(l))} × ${miss ? '？' : H.fmtNum(l.price)}</span><b>${miss ? '' : H.fmtInt(l.amount)}</b></span>
        ${miss ? '<span class="warn">還沒填單價，點這裡輸入</span>' : ''}
        ${l.note ? `<span class="hint">備註：${esc(l.note)}</span>` : ''}
      </button>`;
    }).join('') : '<div class="empty">還沒有品項，按下方「＋ 加品項」。</div>';
    $('doc-lines').querySelectorAll('.line-card').forEach((b) => { b.onclick = () => openItem(Number(b.dataset.i)); });
    $('doc-lines').style.display = 'flex';
    $('doc-lines').style.flexDirection = 'column';
    $('doc-lines').style.gap = '10px';
    $('doc-add').hidden = cur.readonly;
    $('doc-totals').innerHTML = totalsHtml(d);

    const acts = $('doc-actions');
    if (cur.readonly) acts.innerHTML = '<button type="button" class="btn primary" id="da-reprint">補印</button>';
    else if (d.status === 'draft') acts.innerHTML = '<button type="button" class="btn ghost" id="da-draft">存草稿</button><button type="button" class="btn primary" id="da-issue">開立並列印</button>';
    else acts.innerHTML = '<button type="button" class="btn ghost" id="da-draft">儲存</button><button type="button" class="btn primary" id="da-issue">儲存並列印</button>';
    if ($('da-reprint')) $('da-reprint').onclick = () => showReceipt(rec ? rec.doc : d, true);
    if ($('da-draft')) $('da-draft').onclick = () => saveCurrent(false);
    if ($('da-issue')) $('da-issue').onclick = () => saveCurrent(true);
  }
  $('doc-customer').onclick = () => { if (!cur.readonly) { pickMode = 'change'; go('v-customers'); } };
  $('doc-add').onclick = () => go('v-products');
  $('doc-delete').onclick = async () => {
    const d = cur.doc;
    if (!(await ask(`刪除 ${d.doc_no || '這張單'}（${d.cust_name}）？`, '刪除'))) return;
    try {
      H.deleteDoc(S, d.uid);
      persist();
      cur.changed = false;
      toast('已刪除');
      home();
      doSync(false);
    } catch (e) { toast(e.message, true); }
  };

  async function saveCurrent(issuing) {
    const d = clone(cur.doc);
    H.recalc(S, d);
    const err = H.validate(d, issuing || d.status === 'issued');
    if (err) { toast(err, true); return; }
    if (issuing && d.status === 'draft') {
      const ok = await ask(`開立給「${d.cust_name}」\n${d.lines.length} 項，總金額 ${H.fmtInt(d.total)} 元\n\n開立後就不能在手持機刪除。`, '開立並列印');
      if (!ok) return;
    }
    try {
      H.saveDoc(S, d, issuing || d.status === 'issued');
    } catch (e) { toast(e.message, true); return; }
    persist();
    cur.changed = false;
    doSync(false);
    if (issuing) {
      showReceipt(d, false);
    } else {
      toast(d.status === 'draft' ? `草稿已存（${d.doc_no}）` : `已儲存（${d.doc_no}）`);
      home();
    }
  }

  // ---------------- 選客戶 ----------------
  let pickMode = 'new'; // new：開新單；change：換這張單的客戶
  enter['v-customers'] = () => { $('cp-search').value = ''; renderCustomers(); };
  $('cp-search').oninput = () => renderCustomers();
  function renderCustomers() {
    const q = $('cp-search').value.trim().toLowerCase();
    const list = H.customerList(S).filter((c) => !q || [c.name, c.code, c.phone].some((v) => String(v || '').toLowerCase().includes(q)));
    let html = `<button type="button" class="pick-item new" data-new="1"><span class="t">＋ 新增客戶${q ? `「${esc($('cp-search').value.trim())}」` : ''}</span></button>`;
    html += list.map((c) => `<button type="button" class="pick-item" data-uid="${esc(c.uid)}">
      <span class="t">${esc(c.name)}${c.needs_review ? ' <span class="badge need">新</span>' : ''}</span>
      <span class="s">${esc(c.code ? c.code + '・' : '')}${esc(H.TAX_LABEL[c.tax_mode] || '')}</span></button>`).join('');
    if (!list.length && q) html += '<div class="empty">找不到這個客戶</div>';
    $('cp-list').innerHTML = html;
    $('cp-list').querySelectorAll('.pick-item').forEach((b) => {
      b.onclick = () => (b.dataset.new ? go('v-newcust') : chooseCustomer(b.dataset.uid));
    });
  }
  function chooseCustomer(uid) {
    if (pickMode === 'new') {
      cur = { doc: H.newDoc(S, uid), isNew: true, changed: true, readonly: false };
      stack.length = 0;
      stack.push('v-home');
      go('v-doc');
    } else {
      cur.doc.customer_uid = uid;
      H.recalc(S, cur.doc);
      cur.changed = true;
      popTo('v-doc');
    }
  }

  // ---------------- 新增客戶 ----------------
  let ncTax = 'exclusive';
  enter['v-newcust'] = () => {
    $('nc-name').value = $('cp-search').value.trim();
    ['nc-phone', 'nc-address', 'nc-note'].forEach((id) => { $(id).value = ''; });
    setTax('exclusive');
    $('nc-name').classList.remove('invalid');
  };
  function setTax(v) {
    ncTax = v;
    $('nc-tax').querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.v === v)));
  }
  $('nc-tax').querySelectorAll('button').forEach((b) => { b.onclick = () => setTax(b.dataset.v); });
  $('nc-save').onclick = () => {
    const name = $('nc-name').value.trim();
    if (!name) { $('nc-name').classList.add('invalid'); toast('請輸入客戶名稱', true); $('nc-name').focus(); return; }
    const same = H.customerList(S).find((c) => c.name === name);
    if (same) { toast(`已經有「${name}」，直接選用`); chooseCustomer(same.uid); return; }
    const c = H.addCustomer(S, { name, phone: $('nc-phone').value, address: $('nc-address').value, tax_mode: ncTax, note: $('nc-note').value });
    persist();
    doSync(false);
    chooseCustomer(c.uid);
  };

  // ---------------- 選品項 ----------------
  enter['v-products'] = () => { $('pp-search').value = ''; renderProducts(); };
  $('pp-search').oninput = () => renderProducts();
  $('pp-search').onkeydown = (e) => {
    if (e.key !== 'Enter') return;
    const first = $('pp-list').querySelector('.pick-item');
    if (first) first.click();
  };
  function renderProducts() {
    const raw = $('pp-search').value.trim();
    const q = raw.toLowerCase();
    const list = H.productList(S).filter((p) => !q || [p.name, p.code].some((v) => String(v || '').toLowerCase().includes(q)));
    const exact = list.some((p) => p.name === raw);
    let html = '';
    if (raw && !exact) html += `<button type="button" class="pick-item new" data-new="1"><span class="t">＋ 新品項「${esc(raw)}」</span><span class="s">會自動加入品項清單</span></button>`;
    html += list.map((p) => {
      const sug = cur && cur.doc.customer_uid ? H.suggestPrice(S, cur.doc.customer_uid, p.uid) : { price: p.price };
      return `<button type="button" class="pick-item" data-uid="${esc(p.uid)}">
        <span class="t">${esc(p.name)}</span>
        <span class="s">${esc(p.unit || '')}${blankPrice(sug.price) ? '・時價' : '・' + H.fmtNum(sug.price)}</span></button>`;
    }).join('');
    if (!html) html = '<div class="empty">品項清單是空的，直接輸入品名就能新增。</div>';
    $('pp-list').innerHTML = html;
    $('pp-list').querySelectorAll('.pick-item').forEach((b) => {
      b.onclick = () => {
        if (b.dataset.new) {
          openItem(null, { product_uid: null, name: raw, unit: '', qty_text: '', qty: 0, price: null, amount: 0, note: '' });
          return;
        }
        const p = S.products[b.dataset.uid];
        const sug = H.suggestPrice(S, cur.doc.customer_uid, p.uid);
        openItem(null, { product_uid: p.uid, name: p.name, unit: p.unit || '', qty_text: '', qty: 0, price: sug.price, amount: 0, note: '' }, sug);
      };
    });
  }

  // ---------------- 品項：數量、單價 ----------------
  let item = null; // { idx, line, sug }
  const UNITS = ['斤', '箱', '包', '盒', '尾', '隻', '個', '公斤'];
  function openItem(idx, line, sug) {
    const l = line || clone(cur.doc.lines[idx]);
    const s = sug || (l.product_uid ? H.suggestPrice(S, cur.doc.customer_uid, l.product_uid) : null);
    item = { idx, line: l, sug: s, origName: l.name };
    if (idx === null) { stack.pop(); } // 選品項畫面不留在返回路徑上
    go('v-item');
  }
  enter['v-item'] = () => {
    const l = item.line;
    $('it-title').textContent = item.idx === null ? '加品項' : '修改品項';
    $('it-name').value = l.name;
    $('it-qty').value = l.qty_text || (l.qty ? H.fmtNum(l.qty) : '');
    $('it-unit').value = l.unit || '';
    $('it-price').value = blankPrice(l.price) ? '' : String(l.price);
    $('it-note').value = l.note || '';
    $('it-remove').hidden = item.idx === null;
    const units = [...new Set([l.unit, ...UNITS].filter(Boolean))];
    $('it-unit-chips').innerHTML = units.map((u) => `<button type="button" data-u="${esc(u)}">${esc(u)}</button>`).join('');
    $('it-unit-chips').querySelectorAll('button').forEach((b) => { b.onclick = () => { $('it-unit').value = b.dataset.u; updateItem(); }; });
    ['it-name', 'it-qty', 'it-price'].forEach((id) => $(id).classList.remove('invalid'));
    updateItem();
    // 新加的品項直接跳到數量；補單價的跳到單價
    setTimeout(() => {
      const target = item.idx === null ? (l.name ? 'it-qty' : 'it-name') : (blankPrice(l.price) ? 'it-price' : null);
      if (target) $(target).focus();
    }, 80);
  };
  function updateItem() {
    const qText = $('it-qty').value.trim();
    const q = H.parseQty(qText);
    const price = H.parseNum($('it-price').value);
    const qh = $('it-qty-hint');
    if (qText && Number.isNaN(q)) { qh.textContent = '數量格式不對，例如 3、2.5、2斤5兩'; qh.style.color = 'var(--danger)'; }
    else if (/[斤兩]/.test(qText) && q !== null) { qh.textContent = `＝ ${H.fmtNum(Math.round(q * 10000) / 10000)} 斤`; qh.style.color = ''; }
    else { qh.textContent = '重量可以打「2斤5兩」，按下面的斤、兩'; qh.style.color = ''; }
    $('it-unit-chips').querySelectorAll('button').forEach((b) => b.classList.toggle('on', b.dataset.u === $('it-unit').value.trim()));

    const ph = $('it-price-hint');
    const sug = item.sug;
    if (sug && !blankPrice(sug.price) && price !== sug.price) {
      ph.innerHTML = `<button type="button" class="btn ghost" id="it-use-sug" style="min-height:46px;font-size:18px">用 ${esc(H.fmtNum(sug.price))}（${esc(sug.source)}）</button>`;
      $('it-use-sug').onclick = () => { $('it-price').value = String(sug.price); updateItem(); };
    } else ph.textContent = sug ? sug.source : '';

    let html;
    if (Number.isNaN(price)) html = '<span style="color:var(--danger)">單價格式不對</span>';
    else if (price === null) html = '單價空白（可以先存草稿，開立前要填）';
    else if (q === null || Number.isNaN(q)) html = `單價 ${H.fmtNum(price)}`;
    else html = `${esc(qText)} × ${H.fmtNum(price)} ＝ <b>${H.fmtInt(H.lineAmount(q, price))}</b> 元`;
    $('it-amount').innerHTML = html;
  }
  ['it-qty', 'it-price', 'it-unit', 'it-name'].forEach((id) => { $(id).oninput = updateItem; });
  $('it-qty-chips').querySelectorAll('button').forEach((b) => {
    b.onclick = () => {
      const el = $('it-qty');
      if (b.dataset.clear !== undefined) el.value = '';
      else if (!el.value.includes(b.dataset.add)) {
        el.value += b.dataset.add;
        if (!$('it-unit').value.trim()) $('it-unit').value = '斤';
      }
      updateItem();
      el.focus();
    };
  });
  // Enter：品名 → 數量 → 單價 → 確定
  const nextOf = { 'it-name': 'it-qty', 'it-qty': 'it-price', 'it-unit': 'it-price', 'it-price': null, 'it-note': null };
  Object.keys(nextOf).forEach((id) => {
    $(id).addEventListener('keydown', (e) => {
      if (e.key !== 'Enter') return;
      e.preventDefault();
      if (nextOf[id]) $(nextOf[id]).focus(); else $('it-ok').click();
    });
  });
  $('it-ok').onclick = () => {
    const name = $('it-name').value.trim();
    const qText = $('it-qty').value.trim();
    const q = H.parseQty(qText);
    const price = H.parseNum($('it-price').value);
    if (!name) { $('it-name').classList.add('invalid'); toast('請輸入品名', true); return; }
    if (q === null || Number.isNaN(q)) { $('it-qty').classList.add('invalid'); toast('請輸入正確的數量', true); $('it-qty').focus(); return; }
    if (Number.isNaN(price) || (price !== null && price < 0)) { $('it-price').classList.add('invalid'); toast('單價格式不對', true); $('it-price').focus(); return; }
    let productUid = item.line.product_uid;
    if (name !== item.origName || !productUid) {
      const p = Object.values(S.products).find((x) => x.name === name);
      productUid = p ? p.uid : null;
    }
    const line = { product_uid: productUid, name, unit: $('it-unit').value.trim(), qty_text: qText, qty: q,
      price, amount: price === null ? 0 : H.lineAmount(q, price), note: $('it-note').value.trim() };
    if (item.idx === null) cur.doc.lines.push(line); else cur.doc.lines[item.idx] = line;
    H.recalc(S, cur.doc);
    cur.changed = true;
    popTo('v-doc');
  };
  $('it-remove').onclick = async () => {
    if (!(await ask(`刪除「${item.line.name}」這一項？`, '刪除'))) return;
    cur.doc.lines.splice(item.idx, 1);
    H.recalc(S, cur.doc);
    cur.changed = true;
    popTo('v-doc');
  };

  // ---------------- 熱感單 ----------------
  let rc = null; // { doc, reprint }
  function showReceipt(doc, reprint) {
    rc = { doc: clone(doc), reprint };
    if (top() !== 'v-receipt') go('v-receipt');
    printNow();
  }
  enter['v-receipt'] = () => {};
  leave['v-receipt'] = () => { home(); return false; };
  async function printNow() {
    const img = window.Receipt.render(rc.doc, { company: S.config.companyName, reprint: rc.reprint });
    $('rc-image').src = img.dataUrl;
    const st = $('rc-status');
    st.className = 'status-line';
    st.textContent = '列印中…';
    $('rc-print').disabled = true;
    try {
      await NB.print(img.base64, S.config.printer);
      st.className = 'status-line ok';
      st.textContent = rc.reprint ? '已補印' : `已開立 ${rc.doc.doc_no}，已送出列印`;
    } catch (e) {
      st.className = 'status-line err';
      st.textContent = `列印失敗：${e.message}\n單據已存好，處理後按「再印一張」。`;
    } finally {
      $('rc-print').disabled = false;
    }
  }
  $('rc-print').onclick = () => printNow();
  $('rc-done').onclick = () => home();

  // ---------------- 設定 ----------------
  enter['v-settings'] = () => renderSettings();
  function renderSettings() {
    const c = S.config;
    const n = H.pendingCount(S);
    $('st-conn').innerHTML = `
      <div style="display:grid;gap:6px;font-size:17px;line-height:1.5">
        <div><b>電腦</b>　${esc(c.server || '未設定')}</div>
        <div><b>手持機</b>　${esc(c.deviceName || '')}（單號開頭 ${esc(c.prefix || '—')}）</div>
        <div><b>上次同步</b>　${esc(S.lastSyncAt || '尚未同步')}</div>
        <div><b>待上傳</b>　${n} 筆</div>
        ${S.lastSyncError ? `<div style="color:var(--danger);font-weight:700">${esc(S.lastSyncError)}</div>` : ''}
      </div>`;
    if (document.activeElement !== $('st-maker')) $('st-maker').value = c.maker || '';
    const info = NB.printerInfo();
    const opts = [{ v: 'auto', t: info.sunmi ? '商米內建印表機（已連線）' : '自動（商米內建印表機）' }]
      .concat((info.bluetooth || []).map((b) => ({ v: 'bt:' + b.address, t: `藍牙：${b.name}` })));
    $('st-printers').innerHTML = opts.map((o) => `<button type="button" class="opt" data-v="${esc(o.v)}" aria-pressed="${c.printer === o.v}">${esc(o.t)}</button>`).join('')
      + (info.error ? `<p class="hint">${esc(info.error)}</p>` : '');
    $('st-printers').querySelectorAll('.opt').forEach((b) => { b.onclick = () => { S.config.printer = b.dataset.v; persist(); renderSettings(); }; });
    const v = NB.appVersion();
    $('st-version').textContent = `App 版本 ${v || '—'}`;
  }
  $('st-maker').onchange = () => { S.config.maker = $('st-maker').value.trim(); persist(); toast('已儲存'); };
  $('st-test').onclick = () => {
    const doc = { doc_no: `${S.config.prefix || 'V1'}-測試`, doc_date: H.today(), cust_name: '列印測試', tax_mode: 'exclusive',
      lines: [{ name: '白蝦', unit: '斤', qty_text: '2斤8兩', qty: 2.5, price: 380, amount: 950, note: '' },
        { name: '透抽', unit: '箱', qty_text: '1', qty: 1, price: 1200, amount: 1200, note: '測試備註' }] };
    const t = H.totalsFromSum(2150, 'exclusive');
    Object.assign(doc, { subtotal: t.sub, tax: t.tax, total: t.total });
    showReceipt(doc, false);
  };
  $('st-sync').onclick = () => doSync(true);
  $('st-repair').onclick = async () => {
    const n = H.pendingCount(S);
    const msg = n ? `還有 ${n} 筆沒上傳到電腦。\n重新配對後會用新的配對上傳，確定要重新配對？` : '重新配對這台手持機？\n（電腦上舊的配對可以在電腦端停用）';
    if (!(await ask(msg, '重新配對'))) return;
    stack.length = 0;
    go('v-setup');
  };
  $('st-reset').onclick = async () => {
    const n = H.pendingCount(S);
    if (n && !(await ask(`⚠ 還有 ${n} 筆沒上傳到電腦，清除後就會消失！\n確定要清除？`, '仍要清除'))) return;
    if (!(await ask('清除手持機上所有資料並取消配對？\n（電腦上的資料不受影響）', '清除'))) return;
    S = H.emptyState();
    persist();
    stack.length = 0;
    go('v-setup');
  };

  // ---------------- 配對 ----------------
  enter['v-setup'] = () => {
    $('su-server').value = (S.config.server || '').replace(/^https?:\/\//, '').replace(/:47110$/, '');
    $('su-code').value = '';
    if (!$('su-name').value) $('su-name').value = S.config.deviceName || NB.deviceName() || '手持機';
    $('su-error').textContent = '';
  };
  $('su-pair').onclick = async () => {
    const server = $('su-server').value.trim();
    const code = $('su-code').value.trim();
    const name = $('su-name').value.trim() || '手持機';
    const err = $('su-error');
    if (!server) { err.textContent = '請輸入電腦位址'; return; }
    if (!/^\d{6}$/.test(code)) { err.textContent = '配對碼是 6 位數字'; return; }
    const btn = $('su-pair');
    btn.disabled = true;
    btn.textContent = '配對中…';
    err.textContent = '';
    try {
      await H.pair(S, NB.http, server, code, name);
      persist();
      btn.textContent = '下載資料中…';
      try { await H.sync(S, NB.http); } catch (e) { toast('配對成功，但下載資料失敗：' + e.message, true); }
      persist();
      toast('配對完成');
      home();
    } catch (e) {
      const m = e.message || '';
      err.textContent = /offline|fetch|network|connect|timeout|refused|unreachable/i.test(m) || m === '失敗'
        ? '連不到電腦。請確認：手持機的 Tailscale 已連線、電腦開著出貨系統、位址正確。'
        : m;
    } finally {
      btn.disabled = false;
      btn.textContent = '配對並下載資料';
    }
  };
  $('su-code').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('su-pair').click(); });

  // ---------------- 開始 ----------------
  if (S.config.token) { home(); doSync(false); } else go('v-setup');
})();
