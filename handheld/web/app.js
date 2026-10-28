/* 手機畫面：出貨單（待出貨／已出貨）、核對修改、列印給客戶、客戶、同步 */
(function () {
  'use strict';
  const H = window.HH;
  const NB = window.NativeBridge;
  const $ = (id) => document.getElementById(id);
  const clone = (x) => JSON.parse(JSON.stringify(x));
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const noop = () => {};
  const blankPrice = (p) => p === null || p === undefined || p === '';
  const shortTime = (t) => (t ? t.slice(5, 16).replace('-', '/') : '');

  // ---------------- 本機資料 ----------------
  function load() {
    try {
      const t = NB.loadState();
      if (t) {
        const s = JSON.parse(t);
        const base = H.emptyState();
        return Object.assign(base, s, { config: Object.assign(base.config, s.config || {}), shipped: s.shipped || {} });
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
  const TABS = ['v-home', 'v-clients'];
  function show(id) {
    document.querySelectorAll('.view').forEach((v) => v.classList.toggle('active', v.id === id));
    window.scrollTo(0, 0);
    (enter[id] || noop)();
  }
  function go(id) { stack.push(id); show(id); }
  function top() { return stack[stack.length - 1]; }
  function back() {
    if ($('confirm-back').classList.contains('open')) { closeAsk(false); return true; }
    if (stack.length <= 1) {
      // 在「客戶」頁按返回 → 回到出貨單
      if (top() === 'v-clients') { tab('v-home'); return true; }
      return false;
    }
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
  function tab(id) { stack.length = 0; go(id); }
  function home() { tab('v-home'); }
  document.querySelectorAll('[data-back]').forEach((b) => { b.onclick = () => back(); });
  document.querySelectorAll('[data-tab]').forEach((b) => { b.onclick = () => tab(b.dataset.tab); });
  window.onAndroidBack = () => back(); // 回傳 false 時由 Android 把 App 縮到背景

  // ---------------- 同步 ----------------
  let syncing = false;
  let syncAgain = false;
  async function doSync(manual) {
    if (!S.config.token) return;
    if (syncing) { syncAgain = true; return; }
    syncing = true;
    renderChip();
    try {
      const sum = await H.sync(S, NB.http);
      persist();
      for (const m of sum.conflicts) toast('以電腦為準：' + m, true, 7000);
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
    if (top() === 'v-clients') renderClients();
    if (top() === 'v-settings') renderSettings();
  }
  setInterval(() => doSync(false), 120000);
  window.onNetworkAvailable = () => doSync(false);
  window.onNativeResume = () => doSync(false);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) doSync(false); });

  function renderChip() {
    const chip = $('sync-chip');
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
    else text = S.lastSyncAt ? `已同步 ${shortTime(S.lastSyncAt)}` : '尚未同步';
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

  // ---------------- 出貨單清單 ----------------
  enter['v-home'] = () => renderHome();
  function docCard(r) {
    const d = r.doc;
    const badges = [];
    const miss = missingCount(d);
    if (d.status === 'draft') badges.push('<span class="badge draft">草稿</span>');
    else if (!d.shipped_at) badges.push('<span class="badge todo">已開立・未列印</span>');
    if (miss) badges.push(`<span class="badge need">缺 ${miss} 項單價</span>`);
    if (d.shipped_at) badges.push(`<span class="badge ship">已出貨 ${esc(shortTime(d.shipped_at))}</span>`);
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
    const byDate = (a, b) => a.doc.doc_date.localeCompare(b.doc.doc_date) || a.doc.doc_no.localeCompare(b.doc.doc_no);
    const byShipped = (a, b) => b.doc.shipped_at.localeCompare(a.doc.shipped_at);
    // 待出貨：草稿＋已開立還沒列印給客戶的（日期早的在前面）
    const todo = recs.filter((r) => !r.doc.shipped_at).sort(byDate);
    const done = recs.filter((r) => r.doc.shipped_at).sort(byShipped);
    const t = H.today();
    const todays = done.filter((r) => r.doc.shipped_at.startsWith(t));
    const older = done.filter((r) => !r.doc.shipped_at.startsWith(t));
    let html = '';
    html += `<p class="section-title">待出貨（${todo.length}）</p>`;
    html += todo.length ? todo.map(docCard).join('') : '<div class="empty" style="padding:18px">沒有待出貨的單</div>';
    if (todays.length) html += `<p class="section-title">今天已出貨（${todays.length} 張，${H.fmtInt(todays.reduce((s, r) => s + r.doc.total, 0))} 元）</p>` + todays.map(docCard).join('');
    if (older.length) html += '<p class="section-title">之前已出貨</p>' + older.slice(0, 80).map(docCard).join('');
    $('home-list').innerHTML = html;
    $('home-list').querySelectorAll('.doc-card').forEach((b) => { b.onclick = () => openDoc(b.dataset.uid); });
  }
  $('home-new').onclick = () => { pickMode = 'new'; go('v-customers'); };
  $('home-settings').onclick = () => go('v-settings');

  // ---------------- 單據：核對、修改、列印 ----------------
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
      ask('這張單的修改還沒存檔，要放棄修改嗎？', '放棄修改').then((ok) => { if (ok) { cur.changed = false; back(); } });
      return false;
    }
    return true;
  };
  function renderDoc() {
    const d = cur.doc;
    const rec = S.docs[d.uid];
    H.recalc(S, d);
    $('doc-title').textContent = d.doc_no || '新出貨單';
    $('doc-delete').hidden = cur.isNew || cur.readonly || (rec && !H.canDelete(rec));
    let banner = '';
    if (rec && rec.error) banner += `<div class="banner warn">電腦沒有收下這張單：${esc(rec.error)}</div>`;
    if (d.locked) banner += `<div class="banner">已列入對帳單 ${esc(d.statement_no || '')}，不能再修改，只能補印。</div>`;
    else if (cur.changed && d.status === 'issued') banner += '<div class="banner warn">已修改，按「儲存並列印」會更新電腦上的這張單並印出。</div>';
    else if (d.status === 'draft' && missingCount(d)) banner += '<div class="banner warn">有品項還沒填單價（橘色框）。填好後按「開立並列印」。</div>';
    else if (d.status === 'issued' && !d.shipped_at) banner += '<div class="banner">請核對品項和數量。有錯可以點品項修改；沒問題按「列印出貨單」給客戶簽收。</div>';
    else if (d.shipped_at) banner += `<div class="banner">已在 ${esc(shortTime(d.shipped_at))} 列印出貨。再印會標示「補印」。</div>`;
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
    Object.assign($('doc-lines').style, { display: 'flex', flexDirection: 'column', gap: '10px' });
    $('doc-add').hidden = cur.readonly;
    $('doc-totals').innerHTML = totalsHtml(d);

    const acts = $('doc-actions');
    const reprint = !!(rec && rec.doc.shipped_at);
    if (cur.readonly) acts.innerHTML = '<button type="button" class="btn primary" id="da-print">補印</button>';
    else if (d.status === 'draft') acts.innerHTML = '<button type="button" class="btn ghost" id="da-draft">存草稿</button><button type="button" class="btn primary" id="da-issue">開立並列印</button>';
    else if (cur.changed) acts.innerHTML = '<button type="button" class="btn primary" id="da-issue">儲存並列印</button>';
    else acts.innerHTML = `<button type="button" class="btn ${reprint ? 'ghost' : 'primary'}" id="da-print">${reprint ? '補印' : '列印出貨單'}</button>`;
    if ($('da-print')) $('da-print').onclick = () => printForCustomer(rec ? rec.doc : d);
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
    const wasIssued = d.status === 'issued';
    const err = H.validate(d, issuing || wasIssued);
    if (err) { toast(err, true); return; }
    if (issuing && !wasIssued) {
      const ok = await ask(`開立給「${d.cust_name}」\n${d.lines.length} 項，總金額 ${H.fmtInt(d.total)} 元\n\n開立後就不能在手機刪除。`, '開立並列印');
      if (!ok) return;
    }
    try {
      H.saveDoc(S, d, issuing || wasIssued);
    } catch (e) { toast(e.message, true); return; }
    persist();
    cur.changed = false;
    if (issuing) {
      printForCustomer(S.docs[d.uid].doc);
    } else {
      toast(`草稿已存（${d.doc_no}）`);
      home();
    }
    doSync(false);
  }

  // ---------------- 列印給客戶（＝出貨） ----------------
  let rc = null; // { uid, reprint }
  function printForCustomer(doc) {
    rc = { uid: doc.uid, doc: clone(doc), reprint: !!doc.shipped_at };
    if (top() !== 'v-receipt') go('v-receipt');
    printNow();
  }
  enter['v-receipt'] = () => {};
  leave['v-receipt'] = () => { home(); return false; };
  async function printNow() {
    const img = window.Receipt.render(rc.doc, { company: S.config.companyName, reprint: rc.reprint, paper: S.config.paper });
    $('rc-image').src = img.dataUrl;
    const st = $('rc-status');
    st.className = 'status-line';
    st.textContent = '列印中…';
    $('rc-print').disabled = true;
    try {
      await NB.print(img.base64, { printer: S.config.printer, lang: S.config.printLang });
      st.className = 'status-line ok';
      st.textContent = rc.reprint ? '已補印' : `${rc.doc.doc_no} 已列印，記為已出貨`;
      if (rc.uid && S.docs[rc.uid] && !S.docs[rc.uid].doc.shipped_at) {
        H.markShipped(S, rc.uid);
        persist();
        doSync(false);
      }
    } catch (e) {
      st.className = 'status-line err';
      st.textContent = `列印失敗：${e.message}\n單據已存好，處理後按「再印一張」。`;
    } finally {
      $('rc-print').disabled = false;
    }
  }
  $('rc-print').onclick = () => printNow();
  $('rc-done').onclick = () => home();

  // ---------------- 選客戶（開單用） ----------------
  let pickMode = 'new'; // new：開新單；change：換這張單的客戶
  enter['v-customers'] = () => { $('cp-search').value = ''; renderPicker(); };
  $('cp-search').oninput = () => renderPicker();
  function customerRows(q, withNew) {
    const list = H.customerList(S).filter((c) => !q || [c.name, c.code, c.phone].some((v) => String(v || '').toLowerCase().includes(q)));
    let html = withNew ? `<button type="button" class="pick-item new" data-new="1"><span class="t">＋ 新增客戶${q ? `「${esc(withNew)}」` : ''}</span></button>` : '';
    html += list.map((c) => `<button type="button" class="pick-item" data-uid="${esc(c.uid)}">
      <span class="t">${esc(c.name)}${c.needs_review ? ' <span class="badge need">待電腦確認</span>' : ''}</span>
      <span class="s">${esc(c.phone || c.code || '')}<br>${esc(H.TAX_LABEL[c.tax_mode] || '')}</span></button>`).join('');
    if (!list.length && q) html += '<div class="empty">找不到這個客戶</div>';
    return html;
  }
  function renderPicker() {
    const raw = $('cp-search').value.trim();
    $('cp-list').innerHTML = customerRows(raw.toLowerCase(), raw || ' ');
    $('cp-list').querySelectorAll('.pick-item').forEach((b) => {
      b.onclick = () => (b.dataset.new ? openCustomerForm(null, 'picker', raw) : chooseCustomer(b.dataset.uid));
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

  // ---------------- 客戶（下方工具列） ----------------
  enter['v-clients'] = () => renderClients();
  $('cl-search').oninput = () => renderClients();
  function renderClients() {
    const q = $('cl-search').value.trim().toLowerCase();
    const html = customerRows(q, '');
    $('cl-list').innerHTML = html || '<div class="empty">還沒有客戶</div>';
    $('cl-list').querySelectorAll('.pick-item').forEach((b) => { b.onclick = () => openCustomerForm(b.dataset.uid, 'tab'); });
  }
  $('cl-new').onclick = () => openCustomerForm(null, 'tab', $('cl-search').value.trim());
  $('cl-settings').onclick = () => go('v-settings');

  // ---------------- 客戶資料（新增／修改） ----------------
  let form = null; // { uid, from, name }
  let ncTax = 'exclusive';
  function openCustomerForm(uid, from, name) {
    form = { uid, from, name: name || '' };
    go('v-newcust');
  }
  enter['v-newcust'] = () => {
    const c = form.uid ? S.customers[form.uid] : null;
    $('nc-title').textContent = c ? '客戶資料' : '新增客戶';
    $('nc-name').value = c ? c.name : form.name;
    $('nc-phone').value = c ? c.phone || '' : '';
    $('nc-address').value = c ? c.address || '' : '';
    $('nc-taxid').value = c ? c.tax_id || '' : '';
    $('nc-note').value = c ? c.note || '' : '';
    setTax(c ? c.tax_mode : 'exclusive');
    $('nc-name').classList.remove('invalid');
    $('nc-save').textContent = c ? '儲存修改' : form.from === 'picker' ? '儲存並開單' : '儲存';
    $('nc-banner').innerHTML = c && c.code ? `<div class="banner">代號 ${esc(c.code)}${c.closing_day ? `・結帳日每月 ${c.closing_day} 日` : ''}（代號、結帳日在電腦上修改）</div>`
      : c && c.needs_review ? '<div class="banner warn">手機新增的客戶，還沒在電腦上確認。</div>' : '';
    $('nc-hint').textContent = c ? '修改會同步到電腦；如果電腦上同時也改了這個客戶，以電腦為準。' : '新增的客戶會同步到電腦，在電腦上確認或合併。';
    updateCall();
  };
  function updateCall() {
    const tel = $('nc-phone').value.replace(/[^\d+#*]/g, '');
    $('nc-call').hidden = !(form && form.uid && tel);
    $('nc-call').href = 'tel:' + tel;
  }
  $('nc-phone').oninput = updateCall;
  function setTax(v) {
    ncTax = v;
    $('nc-tax').querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.v === v)));
  }
  $('nc-tax').querySelectorAll('button').forEach((b) => { b.onclick = () => setTax(b.dataset.v); });
  $('nc-save').onclick = () => {
    const name = $('nc-name').value.trim();
    if (!name) { $('nc-name').classList.add('invalid'); toast('請輸入客戶名稱', true); $('nc-name').focus(); return; }
    const taxId = $('nc-taxid').value.trim();
    if (taxId && !/^\d{8}$/.test(taxId)) { toast('統一編號是 8 位數字', true); $('nc-taxid').focus(); return; }
    const fields = { name, phone: $('nc-phone').value, address: $('nc-address').value, tax_mode: ncTax, tax_id: taxId, note: $('nc-note').value };
    const same = H.customerList(S).find((c) => c.name === name && c.uid !== form.uid);
    if (same && !form.uid) {
      if (form.from === 'picker') { toast(`已經有「${name}」，直接選用`); chooseCustomer(same.uid); return; }
      toast(`已經有「${name}」這個客戶`, true);
      return;
    }
    let c;
    try {
      c = form.uid ? H.updateCustomer(S, form.uid, fields) : H.addCustomer(S, fields);
    } catch (e) { toast(e.message, true); return; }
    persist();
    doSync(false);
    if (form.from === 'picker') { chooseCustomer(c.uid); return; }
    toast(form.uid ? '已儲存修改' : `已新增 ${c.name}`);
    back();
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
  let item = null; // { idx, line, sug, origName }
  const UNITS = ['斤', '箱', '包', '盒', '尾', '隻', '個', '公斤'];
  function openItem(idx, line, sug) {
    const l = line || clone(cur.doc.lines[idx]);
    const s = sug || (l.product_uid ? H.suggestPrice(S, cur.doc.customer_uid, l.product_uid) : null);
    item = { idx, line: l, sug: s, origName: l.name };
    if (idx === null) stack.pop(); // 選品項畫面不留在返回路徑上
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
    const before = item.idx === null ? null : JSON.stringify(cur.doc.lines[item.idx]);
    if (item.idx === null) cur.doc.lines.push(line); else cur.doc.lines[item.idx] = line;
    H.recalc(S, cur.doc);
    if (before === null || before !== JSON.stringify(cur.doc.lines[item.idx])) cur.changed = true;
    popTo('v-doc');
  };
  $('it-remove').onclick = async () => {
    if (!(await ask(`刪除「${item.line.name}」這一項？`, '刪除'))) return;
    cur.doc.lines.splice(item.idx, 1);
    H.recalc(S, cur.doc);
    cur.changed = true;
    popTo('v-doc');
  };

  // ---------------- 設定 ----------------
  enter['v-settings'] = () => renderSettings();
  function renderSettings() {
    const c = S.config;
    const n = H.pendingCount(S);
    $('st-conn').innerHTML = `
      <div style="display:grid;gap:6px;font-size:17px;line-height:1.5">
        <div><b>電腦</b>　${esc(c.server || '未設定')}</div>
        <div><b>這支手機</b>　${esc(c.deviceName || '')}（單號開頭 ${esc(c.prefix || '—')}）</div>
        <div><b>上次同步</b>　${esc(S.lastSyncAt || '尚未同步')}</div>
        <div><b>待上傳</b>　${n} 筆</div>
        ${S.lastSyncError ? `<div style="color:var(--danger);font-weight:700">${esc(S.lastSyncError)}</div>` : ''}
      </div>`;
    if (document.activeElement !== $('st-maker')) $('st-maker').value = c.maker || '';
    const info = NB.printerInfo();
    const opts = [{ v: 'auto', t: info.sunmi ? '內建印表機（已連線）' : '自動（使用已配對的藍牙印表機）' }]
      .concat((info.bluetooth || []).map((b) => ({ v: 'bt:' + b.address, t: `藍牙：${b.name}` })));
    $('st-printers').innerHTML = opts.map((o) => `<button type="button" class="opt" data-v="${esc(o.v)}" aria-pressed="${c.printer === o.v}">${esc(o.t)}</button>`).join('')
      + `<p class="hint">${info.bluetooth && info.bluetooth.length ? '配對了好幾台藍牙裝置時，請直接選印表機（例如 ZR328）。' : '找不到已配對的藍牙裝置：請先到手機的「設定 → 藍牙」配對印表機。'}</p>`
      + (info.error ? `<p class="hint" style="color:var(--danger)">${esc(info.error)}</p>` : '');
    $('st-printers').querySelectorAll('.opt').forEach((b) => { b.onclick = () => { S.config.printer = b.dataset.v; persist(); renderSettings(); }; });
    const seg = (id, key, val) => $(id).querySelectorAll('button').forEach((b) => {
      b.setAttribute('aria-pressed', String(b.dataset.v === val));
      b.onclick = () => { S.config[key] = b.dataset.v; persist(); renderSettings(); };
    });
    seg('st-paper', 'paper', c.paper || '80');
    seg('st-lang', 'printLang', c.printLang || 'escpos');
    const v = NB.appVersion();
    $('st-version').textContent = `App 版本 ${v || '—'}`;
  }
  $('st-maker').onchange = () => { S.config.maker = $('st-maker').value.trim(); persist(); toast('已儲存'); };
  $('st-test').onclick = () => {
    const doc = { uid: '', doc_no: `${S.config.prefix || 'V1'}-測試`, doc_date: H.today(), cust_name: '列印測試', tax_mode: 'exclusive', shipped_at: '',
      lines: [{ name: '白蝦', unit: '斤', qty_text: '2斤8兩', qty: 2.5, price: 380, amount: 950, note: '' },
        { name: '透抽', unit: '箱', qty_text: '1', qty: 1, price: 1200, amount: 1200, note: '測試備註' }] };
    const t = H.totalsFromSum(2150, 'exclusive');
    Object.assign(doc, { subtotal: t.sub, tax: t.tax, total: t.total });
    printForCustomer(doc);
  };
  $('st-sync').onclick = () => doSync(true);
  $('st-repair').onclick = async () => {
    const n = H.pendingCount(S);
    const msg = n ? `還有 ${n} 筆沒上傳到電腦。\n重新配對後會用新的配對上傳，確定要重新配對？` : '重新配對這支手機？\n（電腦上舊的配對可以在電腦端停用）';
    if (!(await ask(msg, '重新配對'))) return;
    stack.length = 0;
    go('v-setup');
  };
  $('st-reset').onclick = async () => {
    const n = H.pendingCount(S);
    if (n && !(await ask(`⚠ 還有 ${n} 筆沒上傳到電腦，清除後就會消失！\n確定要清除？`, '仍要清除'))) return;
    if (!(await ask('清除手機上所有資料並取消配對？\n（電腦上的資料不受影響）', '清除'))) return;
    S = H.emptyState();
    persist();
    stack.length = 0;
    go('v-setup');
  };

  // ---------------- 配對 ----------------
  enter['v-setup'] = () => {
    $('su-server').value = (S.config.server || '').replace(/^https?:\/\//, '').replace(/:47110$/, '');
    $('su-code').value = '';
    if (!$('su-name').value) $('su-name').value = S.config.deviceName || NB.deviceName() || '手機';
    $('su-error').textContent = '';
  };
  $('su-pair').onclick = async () => {
    const server = $('su-server').value.trim();
    const code = $('su-code').value.trim();
    const name = $('su-name').value.trim() || '手機';
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
        ? '連不到電腦。請確認：手機和電腦在同一個網路（或手機的 Tailscale 已連線）、電腦開著出貨系統、位址正確。'
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
