/* 跟 Android 外殼溝通（window.Native 由 MainActivity 提供）。
 * 在一般瀏覽器開啟時（開發、測試）改用替身：資料存在瀏覽器、網路用 fetch、列印只產生圖片。
 */
(function (root) {
  'use strict';
  const waiting = {};
  let seq = 0;
  // 原生端完成非同步工作後呼叫：__native(id, ok, status, body)
  root.__native = function (id, ok, status, body) {
    const w = waiting[id];
    if (!w) return;
    delete waiting[id];
    if (ok) w.resolve({ status, body }); else w.reject(new Error(body || '失敗'));
  };
  function callAsync(fn) {
    return new Promise((resolve, reject) => {
      const id = 'n' + (++seq);
      waiting[id] = { resolve, reject };
      try { fn(id); } catch (e) { delete waiting[id]; reject(e); }
    });
  }

  const N = root.Native;
  const NativeBridge = N ? {
    isDevice: true,
    loadState: () => N.loadState(),
    saveState: (json) => { const r = N.saveState(json); if (r !== 'ok') throw new Error(r); },
    http: (method, url, headers, body) => callAsync((id) => N.request(id, method, url, JSON.stringify(headers || {}), body || '')),
    print: (pngBase64, printer) => callAsync((id) => N.print(id, pngBase64, printer || 'auto')),
    printerInfo: () => { try { return JSON.parse(N.printerInfo()); } catch (e) { return { sunmi: false, bluetooth: [], error: String(e) }; } },
    deviceName: () => N.deviceName(),
    appVersion: () => (N.appVersion ? N.appVersion() : ''),
  } : {
    isDevice: false,
    loadState: () => { try { return localStorage.getItem('hh-state') || ''; } catch (e) { return ''; } },
    saveState: (json) => { try { localStorage.setItem('hh-state', json); } catch (e) { /* 測試環境 */ } },
    http: async (method, url, headers, body) => {
      if (root.__offline) throw new Error('offline');
      const r = await fetch(url, { method, headers, body: method === 'GET' ? undefined : body });
      return { status: r.status, body: await r.text() };
    },
    print: async (pngBase64) => { root.__lastPrint = pngBase64; return { status: 0, body: '（瀏覽器測試：未實際列印）' }; },
    printerInfo: () => ({ sunmi: false, bluetooth: [{ name: 'InnerPrinter', address: '00:11:22:33:44:55' }], error: '' }),
    deviceName: () => 'SUNMI V3',
    appVersion: () => 'web',
  };
  root.NativeBridge = NativeBridge;
})(window);
