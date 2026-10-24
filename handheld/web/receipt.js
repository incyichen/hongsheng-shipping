/* 熱感單（58mm 紙，384 點寬）：畫成圖片再送印表機，預覽看到的就是印出來的樣子。
 * 內容：公司、出貨單、單號、日期、客戶、品項（數量 × 單價 ＝ 金額）、依客戶稅別的稅金、總金額、簽收欄。
 */
(function (root) {
  'use strict';
  const W = 384;          // 58mm 熱感紙的列印寬度（點）
  const PAD = 6;
  const FONT = '"Noto Sans CJK TC", "Noto Sans TC", "Droid Sans Fallback", sans-serif';

  function font(size, bold) { return `${bold ? 'bold ' : ''}${size}px ${FONT}`; }

  // 依寬度斷行（中文逐字、英數不切開單字太長時也照樣切）
  function wrap(ctx, text, maxW) {
    const out = [];
    let line = '';
    for (const ch of String(text)) {
      if (ctx.measureText(line + ch).width > maxW && line) { out.push(line); line = ch; } else line += ch;
    }
    if (line || !out.length) out.push(line);
    return out;
  }

  function build(doc, opt) {
    const H = root.HH;
    const items = [];
    const text = (t, size, o = {}) => items.push({ kind: 'text', t, size, bold: !!o.bold, align: o.align || 'left', indent: o.indent || 0 });
    const lr = (l, r, size, o = {}) => items.push({ kind: 'lr', l, r, size, bold: !!o.bold, indent: o.indent || 0 });
    const rule = () => items.push({ kind: 'rule' });
    const gap = (h) => items.push({ kind: 'gap', h });

    text(opt.company || '出貨單', 34, { bold: true, align: 'center' });
    text(opt.reprint ? '出貨單（補印）' : '出貨單', 28, { align: 'center' });
    gap(6); rule();
    text(`單號：${doc.doc_no}`, 24);
    text(`日期：${doc.doc_date.replace(/-/g, '/')}`, 24);
    text(`客戶：${doc.cust_name}`, 26, { bold: true });
    rule();
    for (const l of doc.lines) {
      text(l.name, 25, { bold: true });
      const qty = l.qty_text || H.fmtNum(l.qty);
      lr(`${qty}${l.unit && !/[斤兩]/.test(qty) ? l.unit : ''} × ${H.fmtNum(l.price)}`, H.fmtInt(l.amount), 24, { indent: 18 });
      if (l.note) text(`備註：${l.note}`, 20, { indent: 18 });
    }
    rule();
    if (doc.tax_mode === 'inclusive') {
      lr('總金額', H.fmtInt(doc.total), 34, { bold: true });
      lr('（含稅金 5%）', H.fmtInt(doc.tax), 22);
    } else if (doc.tax_mode === 'none') {
      lr('總金額', H.fmtInt(doc.total), 34, { bold: true });
      text('（免稅）', 22);
    } else {
      lr('小計', H.fmtInt(doc.subtotal), 24);
      lr('稅金 5%', H.fmtInt(doc.tax), 24);
      lr('總金額', H.fmtInt(doc.total), 34, { bold: true });
    }
    rule();
    text('簽收：', 24);
    gap(64);
    items.push({ kind: 'sign' });
    gap(30);
    return items;
  }

  // 回傳 { dataUrl, base64, height }
  function render(doc, opt = {}) {
    const canvas = document.getElementById('receipt-canvas') || document.createElement('canvas');
    const ctx = canvas.getContext('2d');
    const items = build(doc, opt);
    // 第一次：量高度
    const lines = [];
    let y = PAD;
    for (const it of items) {
      if (it.kind === 'gap') { y += it.h; continue; }
      if (it.kind === 'rule' || it.kind === 'sign') { lines.push({ ...it, y: y + 8 }); y += 18; continue; }
      ctx.font = font(it.size, it.bold);
      const lh = Math.round(it.size * 1.35);
      if (it.kind === 'text') {
        for (const part of wrap(ctx, it.t, W - PAD * 2 - it.indent)) { lines.push({ ...it, t: part, y }); y += lh; }
      } else {
        const rw = ctx.measureText(it.r).width;
        const leftParts = wrap(ctx, it.l, W - PAD * 2 - it.indent - rw - 14);
        leftParts.forEach((part, i) => {
          lines.push({ kind: 'lr', l: part, r: i === leftParts.length - 1 ? it.r : '', size: it.size, bold: it.bold, indent: it.indent, y });
          y += lh;
        });
      }
    }
    const height = Math.ceil(y + PAD);
    canvas.width = W;
    canvas.height = height;
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, W, height);
    ctx.fillStyle = '#000';
    ctx.strokeStyle = '#000';
    ctx.textBaseline = 'top';
    for (const ln of lines) {
      if (ln.kind === 'rule') {
        ctx.lineWidth = 2;
        ctx.setLineDash([8, 6]);
        ctx.beginPath(); ctx.moveTo(PAD, ln.y); ctx.lineTo(W - PAD, ln.y); ctx.stroke();
        ctx.setLineDash([]);
        continue;
      }
      if (ln.kind === 'sign') {
        ctx.lineWidth = 2;
        ctx.beginPath(); ctx.moveTo(PAD + 70, ln.y); ctx.lineTo(W - PAD, ln.y); ctx.stroke();
        continue;
      }
      ctx.font = font(ln.size, ln.bold);
      if (ln.kind === 'text') {
        const x = ln.align === 'center' ? (W - ctx.measureText(ln.t).width) / 2 : PAD + ln.indent;
        ctx.fillText(ln.t, x, ln.y);
      } else {
        ctx.fillText(ln.l, PAD + ln.indent, ln.y);
        if (ln.r) ctx.fillText(ln.r, W - PAD - ctx.measureText(ln.r).width, ln.y);
      }
    }
    const dataUrl = canvas.toDataURL('image/png');
    return { dataUrl, base64: dataUrl.split(',')[1], height };
  }

  root.Receipt = { render, WIDTH: W };
})(window);
