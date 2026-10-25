//! 手機同步
//!
//! 電腦是唯一的正式資料庫。手機每次同步呼叫一次 `sync()`：
//! 先把手機的變更送上來（新客戶、新品項、出貨單），再把電腦上 `since_rev` 之後的變更送回去。
//!
//! 規則：
//! - 客戶、品項、客戶專屬價、上次成交價：電腦 → 手機（手機只能「新增」客戶與品項）
//! - 出貨單：雙向。草稿全部同步；已開立的只送最近 `DOC_WINDOW_DAYS` 天（補印、看上次價格用）
//! - 手機新增的客戶標記「待確認」，電腦上確認或合併
//! - 同一張單兩邊都改過：手機已開立（客戶手上已有單）→ 手機為準，電腦版另存；兩邊都是草稿 → 電腦版為準，手機版另存，讓你選
//! - 已列入對帳單的單：手機的修改不套用，記一筆提醒

use anyhow::{anyhow, bail, Result};
use chrono::{Duration, Local};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

use crate::db::{now, Db, SaveOpts};
use crate::models::{DocLine, Document};

/// 已開立的出貨單只同步最近幾天
pub const DOC_WINDOW_DAYS: i64 = 60;
/// 配對碼有效時間（分鐘）
const PAIRING_MINUTES: i64 = 10;
/// 配對碼輸錯幾次就作廢
const PAIRING_MAX_ATTEMPTS: i64 = 5;

// ---------------- 傳輸格式（電腦 ⇄ 手機） ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct WireCustomer {
    pub uid: String,
    pub code: String,
    pub name: String,
    pub tax_id: String,
    pub phone: String,
    pub contact: String,
    pub address: String,
    pub closing_day: i64,
    pub tax_mode: String,
    pub note: String,
    pub active: bool,
    pub origin: String,
    pub needs_review: bool,
    pub rev: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct WireProduct {
    pub uid: String,
    pub code: String,
    pub name: String,
    pub unit: String,
    pub price: Option<f64>,
    pub weighed: bool,
    pub active: bool,
    pub origin: String,
    pub rev: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct WireLine {
    pub product_uid: Option<String>,
    pub name: String,
    pub unit: String,
    pub qty_text: String,
    pub qty: f64,
    pub price: f64,
    pub amount: i64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct WireDoc {
    pub uid: String,
    pub doc_no: String,
    pub kind: String,
    pub customer_uid: String,
    pub doc_date: String,
    pub maker: String,
    pub note: String,
    pub status: String,
    /// 已列入對帳單（手機只能看、補印）
    pub locked: bool,
    pub statement_no: Option<String>,
    pub cust_name: String,
    pub cust_phone: String,
    pub cust_address: String,
    pub tax_mode: String,
    pub subtotal: i64,
    pub tax: i64,
    pub total: i64,
    pub origin: String,
    /// 手機列印給客戶（出貨）的時間；空白＝還沒出貨
    pub shipped_at: String,
    pub rev: i64,
    pub lines: Vec<WireLine>,
}

/// 手機送上來的一張單
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PushDoc {
    pub doc: WireDoc,
    /// 手機修改時看到的電腦版本 rev；手機新開的單為 None
    pub base_rev: Option<i64>,
    /// 手機刪除了這張草稿
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SyncRequest {
    /// 上次同步拿到的 server_rev；0＝第一次，全部重送
    pub since_rev: i64,
    pub customers: Vec<WireCustomer>,
    pub products: Vec<WireProduct>,
    pub documents: Vec<PushDoc>,
    /// 手機已列印給客戶的單（只記出貨時間，不動單據內容）
    pub shipped: Vec<ShippedMark>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct ShippedMark {
    pub uid: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct PushResult {
    pub uid: String,
    pub doc_no: String,
    /// "applied"：已套用；"conflict"：沒套用，電腦上會提醒；"rejected"：資料有誤；"ignored"：不需處理
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct PricePair {
    pub customer_uid: String,
    pub product_uid: String,
    pub price: f64,
    pub doc_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Deleted {
    pub tbl: String,
    pub uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SyncResponse {
    /// 下次同步帶這個值當 since_rev
    pub server_rev: i64,
    /// since_rev 為 0 時送的是全部資料，手機應以此取代本機資料
    pub full: bool,
    pub customers: Vec<WireCustomer>,
    pub products: Vec<WireProduct>,
    /// 客戶專屬價（每次全部送）
    pub customer_prices: Vec<PricePair>,
    /// 每位客戶每個品項的上次成交價（每次全部送）
    pub last_prices: Vec<PricePair>,
    pub documents: Vec<WireDoc>,
    pub deleted: Vec<Deleted>,
    /// 客戶合併：舊 uid → 新 uid（手機上還沒上傳的單要改指向新客戶）
    pub customer_merges: BTreeMap<String, String>,
    /// 品項對應：手機新增的品項，電腦上已有同名品項時，手機 uid → 電腦 uid
    pub product_remaps: BTreeMap<String, String>,
    pub results: Vec<PushResult>,
    pub device_prefix: String,
    pub device_name: String,
    /// 這台手機每個月已用到的最大流水號（年月 → 號碼），手機重裝後編號才不會重複
    pub prefix_last_seq: BTreeMap<String, i64>,
    pub company_name: String,
    pub server_time: String,
}

// ---------------- 裝置與配對 ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Device {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub last_sync_at: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PairingCode {
    pub code: String,
    pub prefix: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PairResult {
    pub token: String,
    pub prefix: String,
    pub device_name: String,
    pub company_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SyncConflict {
    pub id: i64,
    pub doc_uid: String,
    pub doc_no: String,
    pub device_prefix: String,
    /// both_edited｜locked｜pc_overwritten｜deleted_on_pc｜delete_rejected｜late_period
    pub kind: String,
    pub message: String,
    /// 另一個版本（WireDoc 的 JSON）；可以選擇改用這個版本的種類才有
    pub payload: String,
    pub created_at: String,
    pub resolved: bool,
    /// 可以「改用手機版本」
    pub can_use_handheld: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DuplicateHint {
    pub customer_id: i64,
    pub candidates: Vec<(i64, String)>,
}

fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

fn random_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn random_code() -> String {
    let n = u128::from_le_bytes(*uuid::Uuid::new_v4().as_bytes());
    format!("{:06}", n % 1_000_000)
}

fn later(minutes: i64) -> String {
    (Local::now() + Duration::minutes(minutes)).format("%Y-%m-%d %H:%M:%S").to_string()
}

impl Db {
    // ---------------- 配對與裝置 ----------------

    /// 產生一組配對碼，並預留下一個手機字頭（V1、V2…，用過的字頭不再重複使用）
    pub fn create_pairing_code(&self) -> Result<PairingCode> {
        self.conn.execute("DELETE FROM pairing_codes WHERE expires_at < ?1", params![now()])?;
        let mut used: Vec<String> = self
            .conn
            .prepare("SELECT prefix FROM devices UNION SELECT prefix FROM pairing_codes")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        used.sort();
        let prefix = (1..100).map(|i| format!("V{}", i)).find(|p| !used.contains(p)).ok_or_else(|| anyhow!("手機數量已達上限"))?;
        let code = random_code();
        let expires_at = later(PAIRING_MINUTES);
        self.conn.execute("INSERT INTO pairing_codes(code,prefix,expires_at) VALUES(?1,?2,?3)", params![code, prefix, expires_at])?;
        Ok(PairingCode { code, prefix, expires_at })
    }

    /// 手機輸入配對碼。成功回傳之後同步要用的 token（只會出現這一次）。
    pub fn pair_device(&self, code: &str, device_name: &str) -> Result<PairResult> {
        self.conn.execute("DELETE FROM pairing_codes WHERE expires_at < ?1", params![now()])?;
        let found: Option<String> = self
            .conn
            .query_row("SELECT prefix FROM pairing_codes WHERE code=?1", params![code.trim()], |r| r.get(0))
            .optional()?;
        let Some(prefix) = found else {
            // 輸錯：所有有效的配對碼都記一次，錯太多次全部作廢，避免被猜中
            self.conn.execute("UPDATE pairing_codes SET attempts = attempts + 1", [])?;
            self.conn.execute("DELETE FROM pairing_codes WHERE attempts >= ?1", params![PAIRING_MAX_ATTEMPTS])?;
            bail!("配對碼錯誤或已過期，請在電腦上重新產生");
        };
        let token = random_token();
        let name = if device_name.trim().is_empty() { format!("手機 {}", prefix) } else { device_name.trim().to_string() };
        self.conn.execute(
            "INSERT INTO devices(name,prefix,token_hash,created_at) VALUES(?1,?2,?3,?4)",
            params![name, prefix, hash_token(&token), now()],
        )?;
        self.conn.execute("DELETE FROM pairing_codes WHERE code=?1", params![code.trim()])?;
        Ok(PairResult { token, prefix, device_name: name, company_name: self.get_settings()?.company_name })
    }

    /// 用 token 找出是哪一台手機；停用的手機不能同步
    pub fn device_by_token(&self, token: &str) -> Result<Device> {
        self.conn
            .query_row(
                "SELECT id,name,prefix,created_at,last_sync_at,revoked FROM devices WHERE token_hash=?1",
                params![hash_token(token.trim())],
                Self::row_device,
            )
            .optional()?
            .filter(|d| !d.revoked)
            .ok_or_else(|| anyhow!("這台手機沒有配對，或已在電腦上停用"))
    }

    fn row_device(r: &rusqlite::Row) -> rusqlite::Result<Device> {
        Ok(Device {
            id: r.get(0)?,
            name: r.get(1)?,
            prefix: r.get(2)?,
            created_at: r.get(3)?,
            last_sync_at: r.get(4)?,
            revoked: r.get::<_, i64>(5)? != 0,
        })
    }

    pub fn list_devices(&self) -> Result<Vec<Device>> {
        let mut st = self.conn.prepare("SELECT id,name,prefix,created_at,last_sync_at,revoked FROM devices ORDER BY prefix")?;
        let rows = st.query_map([], Self::row_device)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn revoke_device(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE devices SET revoked=1 WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn rename_device(&self, id: i64, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            bail!("名稱不可空白");
        }
        self.conn.execute("UPDATE devices SET name=?1 WHERE id=?2", params![name.trim(), id])?;
        Ok(())
    }

    // ---------------- 同步 ----------------

    /// 手機的一次同步：先處理上傳，再回傳電腦上的變更
    pub fn sync(&mut self, device: &Device, req: &SyncRequest) -> Result<SyncResponse> {
        let mut results = Vec::new();
        let mut product_remaps = BTreeMap::new();

        for c in &req.customers {
            results.push(self.push_customer(device, c)?);
        }
        for p in &req.products {
            let r = self.push_product(device, p)?;
            if r.status == "remapped" {
                product_remaps.insert(p.uid.clone(), r.message.clone());
            }
            results.push(r);
        }
        for d in &req.documents {
            let r = match self.push_document(device, d, &product_remaps) {
                Ok(r) => r,
                Err(e) => PushResult { uid: d.doc.uid.clone(), doc_no: d.doc.doc_no.clone(), status: "rejected".into(), message: format!("{:#}", e) },
            };
            results.push(r);
        }
        for m in &req.shipped {
            let at = if m.at.trim().is_empty() { now() } else { m.at.trim().to_string() };
            self.conn.execute("UPDATE documents SET shipped_at=?2 WHERE uid=?1 AND shipped_at=''", params![m.uid, at])?;
        }
        self.conn.execute("UPDATE devices SET last_sync_at=?1 WHERE id=?2", params![now(), device.id])?;

        let mut resp = self.pull(req.since_rev, device)?;
        // 這次上傳時對應過的品項也要讓手機知道（pull 之後才有新的 rev）
        for (k, v) in product_remaps {
            resp.product_remaps.insert(k, v);
        }
        // 這次上傳的單（不論套用或衝突），一律把電腦上的最終版本送回去，
        // 手機才能換成正確版本（即使日期超過同步範圍、或 rev 不在 since 之後）
        for d in &req.documents {
            if resp.documents.iter().any(|x| x.uid == d.doc.uid) {
                continue;
            }
            let id: Option<i64> = self.conn.query_row("SELECT id FROM documents WHERE uid=?1", params![d.doc.uid], |r| r.get(0)).optional()?;
            if let Some(id) = id {
                resp.documents.push(self.wire_doc(id)?);
            }
        }
        resp.results = results;
        Ok(resp)
    }

    fn current_rev(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT v FROM sync_counter", [], |r| r.get(0))?)
    }

    /// 取出 since_rev 之後的所有變更（since_rev = 0：全部）
    pub fn pull(&self, since_rev: i64, device: &Device) -> Result<SyncResponse> {
        let server_rev = self.current_rev()?;
        let full = since_rev <= 0;
        let since = if full { -1 } else { since_rev };

        let customers = {
            let mut st = self.conn.prepare(
                "SELECT uid,COALESCE(code,''),name,tax_id,phone,contact,address,closing_day,tax_mode,note,active,origin,needs_review,rev
                 FROM customers WHERE rev > ?1 ORDER BY rev",
            )?;
            let rows = st.query_map(params![since], |r| {
                Ok(WireCustomer {
                    uid: r.get(0)?,
                    code: r.get(1)?,
                    name: r.get(2)?,
                    tax_id: r.get(3)?,
                    phone: r.get(4)?,
                    contact: r.get(5)?,
                    address: r.get(6)?,
                    closing_day: r.get(7)?,
                    tax_mode: r.get(8)?,
                    note: r.get(9)?,
                    active: r.get::<_, i64>(10)? != 0,
                    origin: r.get(11)?,
                    needs_review: r.get::<_, i64>(12)? != 0,
                    rev: r.get(13)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let products = {
            let mut st = self.conn.prepare(
                "SELECT uid,COALESCE(code,''),name,unit,price,weighed,active,origin,rev FROM products WHERE rev > ?1 ORDER BY rev",
            )?;
            let rows = st.query_map(params![since], |r| {
                Ok(WireProduct {
                    uid: r.get(0)?,
                    code: r.get(1)?,
                    name: r.get(2)?,
                    unit: r.get(3)?,
                    price: r.get(4)?,
                    weighed: r.get::<_, i64>(5)? != 0,
                    active: r.get::<_, i64>(6)? != 0,
                    origin: r.get(7)?,
                    rev: r.get(8)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let customer_prices = {
            let mut st = self.conn.prepare(
                "SELECT c.uid, p.uid, cp.price FROM customer_prices cp JOIN customers c ON c.id=cp.customer_id JOIN products p ON p.id=cp.product_id",
            )?;
            let rows = st.query_map([], |r| Ok(PricePair { customer_uid: r.get(0)?, product_uid: r.get(1)?, price: r.get(2)?, doc_date: String::new() }))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let last_prices = {
            let mut st = self.conn.prepare(
                "SELECT c.uid, p.uid, l.price, d.doc_date FROM document_lines l
                 JOIN documents d ON d.id=l.document_id JOIN customers c ON c.id=d.customer_id JOIN products p ON p.id=l.product_id
                 WHERE d.kind='shipment' AND d.status='issued'
                 ORDER BY d.doc_date DESC, d.id DESC",
            )?;
            let rows = st.query_map([], |r| Ok(PricePair { customer_uid: r.get(0)?, product_uid: r.get(1)?, price: r.get(2)?, doc_date: r.get(3)? }))?;
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for p in rows {
                let p = p?;
                if seen.insert((p.customer_uid.clone(), p.product_uid.clone())) {
                    out.push(p);
                }
            }
            out
        };

        // 出貨單：草稿全部；已開立的只送最近 DOC_WINDOW_DAYS 天
        let window_start = (Local::now() - Duration::days(DOC_WINDOW_DAYS)).format("%Y-%m-%d").to_string();
        let doc_ids: Vec<i64> = self
            .conn
            .prepare("SELECT id FROM documents WHERE rev > ?1 AND kind='shipment' AND (status='draft' OR doc_date >= ?2) ORDER BY rev")?
            .query_map(params![since, window_start], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut documents = Vec::new();
        for id in doc_ids {
            documents.push(self.wire_doc(id)?);
        }

        let deleted = if full {
            vec![]
        } else {
            let mut st = self.conn.prepare("SELECT tbl, uid FROM tombstones WHERE rev > ?1 ORDER BY rev")?;
            let rows = st.query_map(params![since], |r| Ok(Deleted { tbl: r.get(0)?, uid: r.get(1)? }))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let customer_merges = {
            let mut st = self.conn.prepare("SELECT from_uid, to_uid FROM merges")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?
        };

        let prefix_last_seq = {
            let head = format!("{}-", device.prefix);
            let mut st = self.conn.prepare(&format!(
                "SELECT substr(doc_no, {s}, 6), MAX(CAST(substr(doc_no, {n}) AS INTEGER)) FROM documents WHERE doc_no LIKE ?1 GROUP BY substr(doc_no, {s}, 6)",
                s = head.len() + 1,
                n = head.len() + 8
            ))?;
            let rows = st.query_map(params![format!("{}%", head)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?
        };

        Ok(SyncResponse {
            server_rev,
            full,
            customers,
            products,
            customer_prices,
            last_prices,
            documents,
            deleted,
            customer_merges,
            product_remaps: BTreeMap::new(),
            results: vec![],
            device_prefix: device.prefix.clone(),
            device_name: device.name.clone(),
            prefix_last_seq,
            company_name: self.get_settings()?.company_name,
            server_time: now(),
        })
    }

    /// 單據轉成傳輸格式
    pub fn wire_doc(&self, id: i64) -> Result<WireDoc> {
        let d = self.get_document(id)?;
        let (customer_uid, rev): (String, i64) = self.conn.query_row(
            "SELECT c.uid, d.rev FROM documents d JOIN customers c ON c.id=d.customer_id WHERE d.id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut product_uids: HashMap<i64, String> = HashMap::new();
        for l in &d.lines {
            if let Some(pid) = l.product_id {
                if !product_uids.contains_key(&pid) {
                    let u: Option<String> = self.conn.query_row("SELECT uid FROM products WHERE id=?1", params![pid], |r| r.get(0)).optional()?;
                    if let Some(u) = u {
                        product_uids.insert(pid, u);
                    }
                }
            }
        }
        Ok(WireDoc {
            uid: d.uid.clone(),
            doc_no: d.doc_no,
            kind: d.kind,
            customer_uid,
            doc_date: d.doc_date,
            maker: d.maker,
            note: d.note,
            status: d.status,
            locked: d.statement_id.is_some(),
            statement_no: d.statement_no,
            cust_name: d.cust_name,
            cust_phone: d.cust_phone,
            cust_address: d.cust_address,
            tax_mode: d.tax_mode,
            subtotal: d.subtotal,
            tax: d.tax,
            total: d.total,
            origin: d.origin,
            shipped_at: d.shipped_at,
            rev,
            lines: d
                .lines
                .iter()
                .map(|l| WireLine {
                    product_uid: l.product_id.and_then(|p| product_uids.get(&p).cloned()),
                    name: l.name.clone(),
                    unit: l.unit.clone(),
                    qty_text: l.qty_text.clone(),
                    qty: l.qty,
                    price: l.price,
                    amount: l.amount,
                    note: l.note.clone(),
                })
                .collect(),
        })
    }

    /// 客戶 uid 依合併紀錄找到最後的客戶
    fn resolve_customer_uid(&self, uid: &str) -> Result<String> {
        let mut cur = uid.to_string();
        for _ in 0..20 {
            let next: Option<String> = self.conn.query_row("SELECT to_uid FROM merges WHERE from_uid=?1", params![cur], |r| r.get(0)).optional()?;
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        Ok(cur)
    }

    fn push_customer(&self, device: &Device, c: &WireCustomer) -> Result<PushResult> {
        let res = |status: &str, msg: &str| PushResult { uid: c.uid.clone(), doc_no: String::new(), status: status.into(), message: msg.into() };
        if c.uid.trim().is_empty() || c.name.trim().is_empty() {
            return Ok(res("rejected", "客戶缺少 uid 或名稱"));
        }
        let exists: i64 = self.conn.query_row("SELECT COUNT(*) FROM customers WHERE uid=?1", params![c.uid], |r| r.get(0))?;
        let merged: i64 = self.conn.query_row("SELECT COUNT(*) FROM merges WHERE from_uid=?1", params![c.uid], |r| r.get(0))?;
        let tax_mode = match c.tax_mode.as_str() {
            "inclusive" | "none" => c.tax_mode.as_str(),
            _ => "exclusive",
        };
        if merged > 0 {
            return Ok(res("ignored", "已合併到其他客戶"));
        }
        if exists > 0 {
            // rev 0＝手機新增的客戶重送（已經收過）
            if c.rev <= 0 {
                return Ok(res("ignored", "已存在"));
            }
            // 手機修改既有客戶：電腦上這段時間沒改過才套用，否則以電腦為準
            let (id, rev): (i64, i64) = self.conn.query_row("SELECT id, rev FROM customers WHERE uid=?1", params![c.uid], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if rev != c.rev {
                return Ok(res("conflict", "電腦上也修改過這個客戶，以電腦的資料為準"));
            }
            self.conn.execute(
                "UPDATE customers SET name=?1,tax_id=?2,phone=?3,contact=?4,address=?5,tax_mode=?6,note=?7 WHERE id=?8",
                params![c.name.trim(), c.tax_id.trim(), c.phone.trim(), c.contact.trim(), c.address.trim(), tax_mode, c.note, id],
            )?;
            return Ok(res("applied", "已更新客戶資料"));
        }
        self.conn.execute(
            "INSERT INTO customers(code,name,tax_id,phone,contact,address,closing_day,tax_mode,note,active,created_at,uid,origin,needs_review)
             VALUES(NULL,?1,?2,?3,?4,?5,0,?6,?7,1,?8,?9,?10,1)",
            params![c.name.trim(), c.tax_id.trim(), c.phone.trim(), c.contact.trim(), c.address.trim(), tax_mode, c.note, now(), c.uid, device.prefix],
        )?;
        Ok(res("applied", "新客戶，待電腦確認"))
    }

    fn push_product(&self, device: &Device, p: &WireProduct) -> Result<PushResult> {
        let res = |status: &str, msg: &str| PushResult { uid: p.uid.clone(), doc_no: String::new(), status: status.into(), message: msg.into() };
        if p.uid.trim().is_empty() || p.name.trim().is_empty() {
            return Ok(res("rejected", "品項缺少 uid 或名稱"));
        }
        let exists: i64 = self.conn.query_row("SELECT COUNT(*) FROM products WHERE uid=?1", params![p.uid], |r| r.get(0))?;
        if exists > 0 {
            return Ok(res("ignored", "已存在"));
        }
        // 電腦上已有同名品項：不新增，請手機改用電腦的品項
        let same: Option<String> = self
            .conn
            .query_row("SELECT uid FROM products WHERE name=?1 ORDER BY active DESC, id LIMIT 1", params![p.name.trim()], |r| r.get(0))
            .optional()?;
        if let Some(u) = same {
            return Ok(res("remapped", &u));
        }
        let price = p.price.filter(|x| *x > 0.0);
        self.conn.execute(
            "INSERT INTO products(code,name,unit,price,weighed,active,created_at,uid,origin) VALUES(NULL,?1,?2,?3,?4,1,?5,?6,?7)",
            params![p.name.trim(), p.unit.trim(), price, p.weighed as i64, now(), p.uid, device.prefix],
        )?;
        Ok(res("applied", "新品項"))
    }

    /// WireDoc 轉成可以存檔的 Document（客戶、品項由 uid 換成電腦上的編號）
    fn doc_from_wire(&self, w: &WireDoc, id: Option<i64>, remaps: &BTreeMap<String, String>) -> Result<Document> {
        let cust_uid = self.resolve_customer_uid(&w.customer_uid)?;
        let customer_id: i64 = self
            .conn
            .query_row("SELECT id FROM customers WHERE uid=?1", params![cust_uid], |r| r.get(0))
            .optional()?
            .ok_or_else(|| anyhow!("找不到這張單的客戶"))?;
        let mut lines = Vec::new();
        for l in &w.lines {
            let product_id = match &l.product_uid {
                Some(u) => {
                    let u = remaps.get(u).unwrap_or(u);
                    self.conn.query_row("SELECT id FROM products WHERE uid=?1", params![u], |r| r.get(0)).optional()?
                }
                None => None,
            };
            lines.push(DocLine {
                id: None,
                product_id,
                name: l.name.clone(),
                unit: l.unit.clone(),
                qty_text: l.qty_text.clone(),
                qty: l.qty,
                price: l.price,
                amount: 0,
                note: l.note.clone(),
            });
        }
        Ok(Document {
            id,
            kind: if w.kind == "adjustment" { "adjustment".into() } else { "shipment".into() },
            customer_id,
            doc_date: w.doc_date.clone(),
            maker: w.maker.clone(),
            note: w.note.clone(),
            status: if w.status == "draft" { "draft".into() } else { "issued".into() },
            lines,
            ..Default::default()
        })
    }

    fn add_conflict(&self, w: &WireDoc, device: &Device, kind: &str, message: &str, payload: Option<&WireDoc>) -> Result<()> {
        let json = match payload {
            Some(p) => serde_json::to_string(p)?,
            None => String::new(),
        };
        self.conn.execute(
            "INSERT INTO sync_conflicts(doc_uid,doc_no,device_prefix,kind,message,payload,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![w.uid, w.doc_no, device.prefix, kind, message, json, now()],
        )?;
        Ok(())
    }

    /// 已開立的單日期落在已建立的對帳單期間內、卻沒有列進去：提醒
    fn check_late_period(&self, doc_id: i64, device: &Device) -> Result<()> {
        let (customer_id, doc_date, status, stmt, uid, doc_no): (i64, String, String, Option<i64>, String, String) = self.conn.query_row(
            "SELECT customer_id, doc_date, status, statement_id, uid, doc_no FROM documents WHERE id=?1",
            params![doc_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        if status != "issued" || stmt.is_some() {
            return Ok(());
        }
        let covering: Option<String> = self
            .conn
            .query_row(
                "SELECT stmt_no FROM statements WHERE customer_id=?1 AND period_start<=?2 AND period_end>=?2 ORDER BY id DESC LIMIT 1",
                params![customer_id, doc_date],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(no) = covering {
            let w = WireDoc { uid, doc_no, ..Default::default() };
            self.add_conflict(
                &w,
                device,
                "late_period",
                &format!("這張單的日期 {} 落在已建立的對帳單 {} 期間內，但沒有列進去。可以作廢該對帳單重建，或留到下一期。", doc_date.replace('-', "/"), no),
                None,
            )?;
        }
        Ok(())
    }

    fn push_document(&mut self, device: &Device, p: &PushDoc, remaps: &BTreeMap<String, String>) -> Result<PushResult> {
        let w = &p.doc;
        let res = |status: &str, msg: String| PushResult { uid: w.uid.clone(), doc_no: w.doc_no.clone(), status: status.into(), message: msg };
        if w.uid.trim().is_empty() {
            return Ok(res("rejected", "單據缺少 uid".into()));
        }
        let existing: Option<(i64, i64, Option<i64>)> = self
            .conn
            .query_row("SELECT id, rev, statement_id FROM documents WHERE uid=?1", params![w.uid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let handheld_issued = w.status != "draft";

        // ---- 手機刪除草稿 ----
        if p.deleted {
            return match existing {
                None => Ok(res("ignored", "電腦上已沒有這張單".into())),
                Some((id, rev, stmt)) => {
                    if stmt.is_none() && Some(rev) == p.base_rev {
                        self.delete_document(id)?;
                        Ok(res("applied", "已刪除".into()))
                    } else {
                        self.add_conflict(w, device, "delete_rejected", "手機刪除了這張草稿，但電腦上已經修改過，所以沒有刪除。", None)?;
                        Ok(res("conflict", "電腦上已修改，沒有刪除".into()))
                    }
                }
            };
        }

        match existing {
            // ---- 電腦上沒有這張單 ----
            None => {
                let tomb: i64 = self.conn.query_row("SELECT COUNT(*) FROM tombstones WHERE tbl='documents' AND uid=?1", params![w.uid], |r| r.get(0))?;
                if tomb > 0 && !handheld_issued {
                    self.add_conflict(w, device, "deleted_on_pc", "電腦上已經刪除這張草稿，但手機又修改過。", Some(w))?;
                    return Ok(res("conflict", "電腦上已刪除，手機版本另存待確認".into()));
                }
                let doc = self.doc_from_wire(w, None, remaps)?;
                let no = if w.doc_no.trim().is_empty() { None } else { Some(w.doc_no.trim().to_string()) };
                let saved = self.save_document_with(&doc, &SaveOpts { doc_no: no, uid: Some(w.uid.clone()), origin: device.prefix.clone() })?;
                if tomb > 0 {
                    self.add_conflict(w, device, "deleted_on_pc", "電腦上刪除過這張單，但手機已經開立並交給客戶，所以已恢復。", None)?;
                }
                self.check_late_period(saved.id.unwrap(), device)?;
                Ok(res("applied", "新單".into()))
            }
            // ---- 電腦上有這張單 ----
            Some((id, rev, stmt)) => {
                if stmt.is_some() {
                    self.add_conflict(w, device, "locked", "這張單已列入對帳單，手機的修改沒有套用。要套用請先作廢該對帳單。", Some(w))?;
                    return Ok(res("conflict", "已列入對帳單".into()));
                }
                let pc_changed = Some(rev) != p.base_rev;
                if pc_changed && !handheld_issued {
                    // 兩邊都改了草稿：電腦版為準，手機版另存讓你選
                    self.add_conflict(w, device, "both_edited", "電腦和手機都修改了這張草稿。目前以電腦版為準，可以改用手機版本。", Some(w))?;
                    return Ok(res("conflict", "兩邊都修改過，待電腦確認".into()));
                }
                if pc_changed && handheld_issued {
                    // 手機已開立並列印給客戶：手機為準，電腦版另存備查
                    let pc_version = self.wire_doc(id)?;
                    self.add_conflict(w, device, "pc_overwritten", "手機已開立並交給客戶，以手機版本為準；電腦上原本的修改另存在這裡。", Some(&pc_version))?;
                }
                let doc = self.doc_from_wire(w, Some(id), remaps)?;
                self.save_document_with(&doc, &SaveOpts { doc_no: None, uid: None, origin: String::new() })?;
                self.check_late_period(id, device)?;
                Ok(res("applied", if pc_changed { "已套用（電腦版另存）".into() } else { "已更新".into() }))
            }
        }
    }

    // ---------------- 同步提醒（電腦端） ----------------

    pub fn list_sync_conflicts(&self, include_resolved: bool) -> Result<Vec<SyncConflict>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT id,doc_uid,doc_no,device_prefix,kind,message,payload,created_at,resolved FROM sync_conflicts {} ORDER BY id DESC LIMIT 200",
            if include_resolved { "" } else { "WHERE resolved=0" }
        ))?;
        let rows = st.query_map([], |r| {
            let kind: String = r.get(4)?;
            let payload: String = r.get(6)?;
            Ok(SyncConflict {
                id: r.get(0)?,
                doc_uid: r.get(1)?,
                doc_no: r.get(2)?,
                device_prefix: r.get(3)?,
                can_use_handheld: !payload.is_empty() && matches!(kind.as_str(), "both_edited" | "locked" | "deleted_on_pc"),
                kind,
                message: r.get(5)?,
                payload,
                created_at: r.get(7)?,
                resolved: r.get::<_, i64>(8)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// action："use_handheld"＝改用手機版本；"dismiss"＝保留電腦版本／知道了
    pub fn resolve_sync_conflict(&mut self, id: i64, action: &str) -> Result<()> {
        let (kind, payload): (String, String) = self
            .conn
            .query_row("SELECT kind, payload FROM sync_conflicts WHERE id=?1", params![id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .ok_or_else(|| anyhow!("找不到這筆提醒"))?;
        if action == "use_handheld" {
            if payload.is_empty() || !matches!(kind.as_str(), "both_edited" | "locked" | "deleted_on_pc") {
                bail!("這筆提醒沒有可以套用的手機版本");
            }
            let w: WireDoc = serde_json::from_str(&payload)?;
            let existing: Option<i64> = self.conn.query_row("SELECT id FROM documents WHERE uid=?1", params![w.uid], |r| r.get(0)).optional()?;
            let doc = self.doc_from_wire(&w, existing, &BTreeMap::new())?;
            match existing {
                // 已列入對帳單的話，save_document 會擋下並說明要先作廢對帳單
                Some(_) => {
                    self.save_document(&doc)?;
                }
                None => {
                    let no = if w.doc_no.trim().is_empty() { None } else { Some(w.doc_no.clone()) };
                    self.save_document_with(&doc, &SaveOpts { doc_no: no, uid: Some(w.uid.clone()), origin: w.origin.clone() })?;
                }
            }
        }
        self.conn.execute("UPDATE sync_conflicts SET resolved=1 WHERE id=?1", params![id])?;
        Ok(())
    }

    // ---------------- 手機新增的客戶：確認與合併 ----------------

    /// 可能重複的客戶：名稱相同，或電話相同（電話不是空白）
    pub fn possible_duplicates(&self, customer_id: i64) -> Result<Vec<(i64, String)>> {
        let c = self.get_customer(customer_id)?;
        let mut st = self.conn.prepare(
            "SELECT id, name FROM customers WHERE id<>?1 AND (trim(name)=trim(?2) OR (trim(?3)<>'' AND replace(replace(phone,'-',''),' ','')=replace(replace(?3,'-',''),' ','')))",
        )?;
        let rows = st.query_map(params![customer_id, c.name, c.phone], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn confirm_customer(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE customers SET needs_review=0 WHERE id=?1", params![id])?;
        Ok(())
    }

    /// 把 from 客戶合併到 to 客戶：出貨單、收款、對帳單、手開對帳單、專屬價全部移過去，然後刪除 from
    pub fn merge_customers(&mut self, from_id: i64, to_id: i64) -> Result<()> {
        if from_id == to_id {
            bail!("不能合併到同一位客戶");
        }
        let from = self.get_customer(from_id)?;
        let to = self.get_customer(to_id)?;
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE documents SET customer_id=?1 WHERE customer_id=?2 AND status<>'draft'", params![to_id, from_id])?;
        // 草稿的客戶資料快照一起換成新客戶
        tx.execute(
            "UPDATE documents SET customer_id=?1, cust_name=?2, cust_phone=?3, cust_address=?4 WHERE customer_id=?5 AND status='draft'",
            params![to_id, to.name, to.phone, to.address, from_id],
        )?;
        tx.execute("UPDATE payments SET customer_id=?1 WHERE customer_id=?2", params![to_id, from_id])?;
        tx.execute("UPDATE statements SET customer_id=?1 WHERE customer_id=?2", params![to_id, from_id])?;
        tx.execute("UPDATE manual_statements SET customer_id=?1 WHERE customer_id=?2", params![to_id, from_id])?;
        tx.execute(
            "INSERT OR IGNORE INTO customer_prices(customer_id,product_id,price) SELECT ?1, product_id, price FROM customer_prices WHERE customer_id=?2",
            params![to_id, from_id],
        )?;
        tx.execute("DELETE FROM customer_prices WHERE customer_id=?1", params![from_id])?;
        tx.execute("UPDATE merges SET to_uid=?1 WHERE to_uid=?2", params![to.uid, from.uid])?;
        tx.execute("INSERT OR REPLACE INTO merges(from_uid,to_uid,merged_at) VALUES(?1,?2,?3)", params![from.uid, to.uid, now()])?;
        tx.execute("DELETE FROM customers WHERE id=?1", params![from_id])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Customer, Product};

    /// 模擬手機：只記得 token、since_rev，以及電腦送來的資料
    struct Handheld {
        device: Device,
        since: i64,
    }

    fn setup() -> (Db, Handheld, i64, i64) {
        let db = Db::open_in_memory().unwrap();
        let cid = db.save_customer(&Customer { code: "C001".into(), name: "時光樹影吧檯".into(), phone: "02-2711-0000".into(), tax_mode: "exclusive".into(), active: true, ..Default::default() }).unwrap();
        let pid = db.save_product(&Product { code: "SH01".into(), name: "白蝦（大）".into(), unit: "斤".into(), price: Some(380.0), weighed: true, active: true, ..Default::default() }).unwrap();
        let code = db.create_pairing_code().unwrap();
        assert_eq!(code.prefix, "V1");
        let pr = db.pair_device(&code.code, "V3 測試機").unwrap();
        let device = db.device_by_token(&pr.token).unwrap();
        (db, Handheld { device, since: 0 }, cid, pid)
    }

    fn sync(db: &mut Db, h: &mut Handheld, req: SyncRequest) -> SyncResponse {
        let req = SyncRequest { since_rev: h.since, ..req };
        let r = db.sync(&h.device, &req).unwrap();
        h.since = r.server_rev;
        r
    }

    fn line(product_uid: Option<&str>, name: &str, qty: &str, price: f64) -> WireLine {
        WireLine { product_uid: product_uid.map(|s| s.to_string()), name: name.into(), unit: "斤".into(), qty_text: qty.into(), price, ..Default::default() }
    }

    #[test]
    fn pairing_rules() {
        let (db, _h, _, _) = setup();
        // 錯的配對碼
        let code = db.create_pairing_code().unwrap();
        assert_eq!(code.prefix, "V2");
        assert!(db.pair_device("000000x", "x").is_err());
        // 錯太多次，配對碼作廢
        for _ in 0..PAIRING_MAX_ATTEMPTS {
            let _ = db.pair_device("bad", "x");
        }
        assert!(db.pair_device(&code.code, "x").is_err());
        // 作廢的配對碼從來沒有手機用過，字頭可以再用
        let c2 = db.create_pairing_code().unwrap();
        let pr = db.pair_device(&c2.code, "備援機").unwrap();
        assert_eq!(pr.prefix, "V2");
        // 停用的手機不能同步，而且它的字頭不再重複使用（它開過的單號才不會撞到）
        let d = db.device_by_token(&pr.token).unwrap();
        db.revoke_device(d.id).unwrap();
        assert!(db.device_by_token(&pr.token).is_err());
        assert!(db.device_by_token("not-a-token").is_err());
        assert_eq!(db.create_pairing_code().unwrap().prefix, "V3");
    }

    #[test]
    fn first_sync_sends_everything_then_only_changes() {
        let (mut db, mut h, cid, _pid) = setup();
        // 電腦上先開一張草稿（價格空白）和一張已開立的單
        let draft = db.save_document(&Document { customer_id: cid, doc_date: Local::now().format("%Y-%m-%d").to_string(), status: "draft".into(),
            lines: vec![DocLine { name: "白蝦（大）".into(), unit: "斤".into(), qty_text: "3".into(), price: 0.0, ..Default::default() }], ..Default::default() }).unwrap();
        let r = sync(&mut db, &mut h, SyncRequest::default());
        assert!(r.full);
        assert_eq!(r.customers.len(), 1);
        assert_eq!(r.products.len(), 1);
        assert_eq!(r.documents.len(), 1);
        assert_eq!(r.documents[0].uid, draft.uid);
        assert_eq!(r.device_prefix, "V1");
        // 沒有任何變更：什麼都不送
        let r2 = sync(&mut db, &mut h, SyncRequest::default());
        assert!(!r2.full);
        assert!(r2.customers.is_empty() && r2.products.is_empty() && r2.documents.is_empty() && r2.deleted.is_empty());
        // 電腦上改了品項價格 → 只送那個品項
        let mut p = db.list_products(true).unwrap().remove(0);
        p.price = Some(420.0);
        db.save_product(&p).unwrap();
        let r3 = sync(&mut db, &mut h, SyncRequest::default());
        assert_eq!(r3.products.len(), 1);
        assert_eq!(r3.products[0].price, Some(420.0));
        assert!(r3.customers.is_empty());
        // 電腦上刪除草稿 → 送刪除通知
        db.delete_document(draft.id.unwrap()).unwrap();
        let r4 = sync(&mut db, &mut h, SyncRequest::default());
        assert_eq!(r4.deleted, vec![Deleted { tbl: "documents".into(), uid: draft.uid.clone() }]);
    }

    #[test]
    fn handheld_new_customer_and_document_offline() {
        let (mut db, mut h, _cid, _pid) = setup();
        let r = sync(&mut db, &mut h, SyncRequest::default());
        let shrimp = r.products[0].uid.clone();
        // 離線時：新增客戶、開一張單（含一個新品項），已開立並列印
        let new_cust = WireCustomer { uid: "hh-cust-1".into(), name: "海港小館".into(), phone: "02-2345-1111".into(), tax_mode: "inclusive".into(), ..Default::default() };
        let new_prod = WireProduct { uid: "hh-prod-1".into(), name: "海鮮煎餅".into(), unit: "盒".into(), price: Some(50.0), ..Default::default() };
        let doc = WireDoc {
            uid: "hh-doc-1".into(),
            doc_no: "V1-202610-0001".into(),
            customer_uid: "hh-cust-1".into(),
            doc_date: "2026-10-25".into(),
            status: "issued".into(),
            lines: vec![line(Some(&shrimp), "白蝦（大）", "2斤5兩", 380.0), line(Some("hh-prod-1"), "海鮮煎餅", "2", 50.0)],
            ..Default::default()
        };
        let req = SyncRequest { customers: vec![new_cust.clone()], products: vec![new_prod.clone()], documents: vec![PushDoc { doc: doc.clone(), base_rev: None, deleted: false }], ..Default::default() };
        let r = sync(&mut db, &mut h, req.clone());
        assert!(r.results.iter().all(|x| x.status == "applied"), "{:?}", r.results);
        // 電腦上：客戶待確認、單號照手機、金額由電腦重算（內含稅）
        let c = db.list_customers(true).unwrap().into_iter().find(|c| c.name == "海港小館").unwrap();
        assert!(c.needs_review);
        assert_eq!(c.origin, "V1");
        let docs = db.list_documents(&Default::default()).unwrap();
        let d = db.get_document(docs.iter().find(|d| d.doc_no == "V1-202610-0001").unwrap().id).unwrap();
        assert_eq!(d.origin, "V1");
        assert_eq!(d.lines.len(), 2);
        assert_eq!(d.total, 879 + 100);
        assert_eq!(d.tax_mode, "inclusive");
        // 同一批再送一次（網路斷掉重送）：不會重複
        let mut h2 = Handheld { device: h.device.clone(), since: h.since };
        let again = sync(&mut db, &mut h2, req);
        assert!(again.results.iter().all(|x| x.status == "ignored" || x.status == "applied"), "{:?}", again.results);
        assert_eq!(db.list_customers(true).unwrap().len(), 2);
        assert_eq!(db.list_products(true).unwrap().len(), 2);
        assert_eq!(db.list_documents(&Default::default()).unwrap().len(), 1);
        // 單號已用到 0001，手機重裝後也知道
        assert_eq!(again.prefix_last_seq.get("202610"), Some(&1));
    }

    #[test]
    fn product_with_same_name_is_remapped() {
        let (mut db, mut h, cid, pid) = setup();
        let r0 = sync(&mut db, &mut h, SyncRequest::default());
        let cust_uid = r0.customers[0].uid.clone();
        let pc_uid = db.list_products(true).unwrap().into_iter().find(|p| p.id == Some(pid)).unwrap().uid;
        let dup = WireProduct { uid: "hh-dup".into(), name: "白蝦（大）".into(), unit: "斤".into(), ..Default::default() };
        let doc = WireDoc { uid: "d1".into(), doc_no: "V1-202610-0002".into(), customer_uid: cust_uid, doc_date: "2026-10-25".into(), status: "issued".into(),
            lines: vec![line(Some("hh-dup"), "白蝦（大）", "1", 380.0)], ..Default::default() };
        let r = sync(&mut db, &mut h, SyncRequest { products: vec![dup], documents: vec![PushDoc { doc, base_rev: None, deleted: false }], ..Default::default() });
        assert_eq!(r.product_remaps.get("hh-dup"), Some(&pc_uid));
        assert_eq!(db.list_products(true).unwrap().len(), 1, "不重複新增");
        let d = db.list_documents(&Default::default()).unwrap();
        assert_eq!(db.get_document(d[0].id).unwrap().lines[0].product_id, Some(pid));
        let _ = cid;
    }

    #[test]
    fn draft_from_pc_priced_on_handheld() {
        let (mut db, mut h, cid, _pid) = setup();
        // 電腦先開草稿：品項、數量，價格空白（時價）
        let draft = db.save_document(&Document { customer_id: cid, doc_date: "2026-10-25".into(), status: "draft".into(),
            lines: vec![DocLine { name: "白蝦（大）".into(), unit: "斤".into(), qty_text: "3".into(), price: 0.0, ..Default::default() }], ..Default::default() }).unwrap();
        let r = sync(&mut db, &mut h, SyncRequest::default());
        let mut w = r.documents.iter().find(|d| d.uid == draft.uid).unwrap().clone();
        let base = w.rev;
        // 手機補上價格、開立
        w.lines[0].price = 400.0;
        w.status = "issued".into();
        let r2 = sync(&mut db, &mut h, SyncRequest { documents: vec![PushDoc { doc: w, base_rev: Some(base), deleted: false }], ..Default::default() });
        assert_eq!(r2.results[0].status, "applied", "{:?}", r2.results);
        let d = db.get_document(draft.id.unwrap()).unwrap();
        assert_eq!((d.status.as_str(), d.doc_no.as_str(), d.subtotal), ("issued", draft.doc_no.as_str(), 1200), "沿用電腦的 S 單號");
        assert!(db.list_sync_conflicts(false).unwrap().is_empty());
        // 手機收到更新後的版本
        assert!(r2.documents.iter().any(|x| x.uid == draft.uid && x.status == "issued"));
    }

    #[test]
    fn conflicts() {
        let (mut db, mut h, cid, _pid) = setup();
        let mk = |db: &mut Db| db.save_document(&Document { customer_id: cid, doc_date: "2026-10-25".into(), status: "draft".into(),
            lines: vec![DocLine { name: "白蝦（大）".into(), unit: "斤".into(), qty_text: "3".into(), price: 0.0, ..Default::default() }], ..Default::default() }).unwrap();
        let a = mk(&mut db);
        let b = mk(&mut db);
        let r = sync(&mut db, &mut h, SyncRequest::default());
        let wa = r.documents.iter().find(|d| d.uid == a.uid).unwrap().clone();
        let wb = r.documents.iter().find(|d| d.uid == b.uid).unwrap().clone();
        // 電腦上兩張都再改過
        for d in [&a, &b] {
            let mut x = db.get_document(d.id.unwrap()).unwrap();
            x.lines[0].qty_text = "4".into();
            db.save_document(&x).unwrap();
        }
        // A：手機也只是改草稿 → 電腦版為準，手機版另存
        let mut ha = wa.clone();
        ha.lines[0].price = 390.0;
        // B：手機已開立並列印 → 手機為準，電腦版另存
        let mut hb = wb.clone();
        hb.lines[0].price = 400.0;
        hb.status = "issued".into();
        let r2 = sync(&mut db, &mut h, SyncRequest { documents: vec![
            PushDoc { doc: ha, base_rev: Some(wa.rev), deleted: false },
            PushDoc { doc: hb, base_rev: Some(wb.rev), deleted: false },
        ], ..Default::default() });
        assert_eq!(r2.results[0].status, "conflict");
        assert_eq!(r2.results[1].status, "applied");
        // 兩張單的電腦最終版本都有送回手機
        assert!(r2.documents.iter().any(|d| d.uid == a.uid && d.lines[0].qty_text == "4"));
        assert!(r2.documents.iter().any(|d| d.uid == b.uid && d.status == "issued"));
        let da = db.get_document(a.id.unwrap()).unwrap();
        assert_eq!((da.lines[0].qty_text.as_str(), da.lines[0].price), ("4", 0.0), "A 維持電腦版");
        let dbb = db.get_document(b.id.unwrap()).unwrap();
        assert_eq!((dbb.status.as_str(), dbb.lines[0].qty_text.as_str(), dbb.lines[0].price), ("issued", "3", 400.0), "B 用手機版");
        let cs = db.list_sync_conflicts(false).unwrap();
        let kinds: Vec<&str> = cs.iter().map(|c| c.kind.as_str()).collect();
        assert!(kinds.contains(&"both_edited") && kinds.contains(&"pc_overwritten"), "{:?}", kinds);
        // 在電腦上選「改用手機版本」
        let ca = cs.iter().find(|c| c.kind == "both_edited").unwrap();
        assert!(ca.can_use_handheld);
        db.resolve_sync_conflict(ca.id, "use_handheld").unwrap();
        let da = db.get_document(a.id.unwrap()).unwrap();
        assert_eq!((da.lines[0].qty_text.as_str(), da.lines[0].price), ("3", 390.0));
        let cb = cs.iter().find(|c| c.kind == "pc_overwritten").unwrap();
        assert!(!cb.can_use_handheld);
        db.resolve_sync_conflict(cb.id, "dismiss").unwrap();
        assert!(db.list_sync_conflicts(false).unwrap().is_empty());
    }

    #[test]
    fn locked_and_late_period() {
        let (mut db, mut h, cid, _pid) = setup();
        let issued = db.save_document(&Document { customer_id: cid, doc_date: "2026-09-10".into(), status: "issued".into(),
            lines: vec![DocLine { name: "白蝦（大）".into(), unit: "斤".into(), qty_text: "1".into(), price: 380.0, ..Default::default() }], ..Default::default() }).unwrap();
        let r = sync(&mut db, &mut h, SyncRequest::default());
        let cust_uid = r.customers[0].uid.clone();
        let w = db.wire_doc(issued.id.unwrap()).unwrap();
        // 電腦上建 9 月對帳單 → 這張單鎖定
        db.create_statement(cid, "2026-09-01", "2026-09-30", "summary", "").unwrap();
        // 手機改這張單 → 不套用
        let mut hw = w.clone();
        hw.lines[0].price = 1.0;
        let r2 = sync(&mut db, &mut h, SyncRequest { documents: vec![PushDoc { doc: hw, base_rev: Some(w.rev), deleted: false }], ..Default::default() });
        assert_eq!(r2.results[0].status, "conflict");
        assert_eq!(db.get_document(issued.id.unwrap()).unwrap().lines[0].price, 380.0);
        // 手機離線很久，現在才上傳一張 9 月的單 → 收下，但提醒沒列入 9 月對帳單
        let late = WireDoc { uid: "late".into(), doc_no: "V1-202609-0001".into(), customer_uid: cust_uid, doc_date: "2026-09-20".into(), status: "issued".into(),
            lines: vec![line(None, "透抽", "1", 300.0)], ..Default::default() };
        let r3 = sync(&mut db, &mut h, SyncRequest { documents: vec![PushDoc { doc: late, base_rev: None, deleted: false }], ..Default::default() });
        assert_eq!(r3.results[0].status, "applied");
        let kinds: Vec<String> = db.list_sync_conflicts(false).unwrap().into_iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&"locked".to_string()) && kinds.contains(&"late_period".to_string()), "{:?}", kinds);
        // 新品名「透抽」自動加入品項清單，來源標記 V1
        let p = db.list_products(true).unwrap().into_iter().find(|p| p.name == "透抽").unwrap();
        assert_eq!(p.origin, "V1");
    }

    #[test]
    fn handheld_delete_draft() {
        let (mut db, mut h, cid, _pid) = setup();
        let mk = |db: &mut Db| db.save_document(&Document { customer_id: cid, doc_date: "2026-10-25".into(), status: "draft".into(),
            lines: vec![DocLine { name: "白蝦（大）".into(), qty_text: "1".into(), ..Default::default() }], ..Default::default() }).unwrap();
        let a = mk(&mut db);
        let b = mk(&mut db);
        let r = sync(&mut db, &mut h, SyncRequest::default());
        let wa = r.documents.iter().find(|d| d.uid == a.uid).unwrap().clone();
        let wb = r.documents.iter().find(|d| d.uid == b.uid).unwrap().clone();
        let mut x = db.get_document(b.id.unwrap()).unwrap();
        x.note = "電腦改過".into();
        db.save_document(&x).unwrap();
        let r2 = sync(&mut db, &mut h, SyncRequest { documents: vec![
            PushDoc { base_rev: Some(wa.rev), doc: wa, deleted: true },
            PushDoc { base_rev: Some(wb.rev), doc: wb, deleted: true },
        ], ..Default::default() });
        assert_eq!(r2.results[0].status, "applied");
        assert_eq!(r2.results[1].status, "conflict", "電腦改過就不刪");
        assert!(db.get_document(a.id.unwrap()).is_err());
        assert!(db.get_document(b.id.unwrap()).is_ok());
    }

    #[test]
    fn merge_handheld_customer() {
        let (mut db, mut h, cid, _pid) = setup();
        sync(&mut db, &mut h, SyncRequest::default());
        // 手機又新增了一次「時光樹影吧檯」（電話一樣）
        let dup = WireCustomer { uid: "hh-dup-cust".into(), name: "時光樹影".into(), phone: "0227110000".into(), ..Default::default() };
        let d1 = WireDoc { uid: "m1".into(), doc_no: "V1-202610-0003".into(), customer_uid: "hh-dup-cust".into(), doc_date: "2026-10-25".into(), status: "issued".into(),
            lines: vec![line(None, "白蝦（大）", "1", 380.0)], ..Default::default() };
        sync(&mut db, &mut h, SyncRequest { customers: vec![dup], documents: vec![PushDoc { doc: d1, base_rev: None, deleted: false }], ..Default::default() });
        let dup_id = db.list_customers(true).unwrap().into_iter().find(|c| c.uid == "hh-dup-cust").unwrap().id.unwrap();
        // 電話相同 → 提示可能重複
        let cands = db.possible_duplicates(dup_id).unwrap();
        assert_eq!(cands, vec![(cid, "時光樹影吧檯".to_string())]);
        db.merge_customers(dup_id, cid).unwrap();
        assert_eq!(db.list_customers(true).unwrap().len(), 1);
        let docs = db.list_documents(&Default::default()).unwrap();
        assert!(docs.iter().all(|d| d.customer_id == cid));
        // 手機收到：重複客戶刪除、合併對照、單據改指向原客戶
        let r = sync(&mut db, &mut h, SyncRequest::default());
        assert!(r.deleted.contains(&Deleted { tbl: "customers".into(), uid: "hh-dup-cust".into() }));
        let cust_uid = db.get_customer(cid).unwrap().uid;
        assert_eq!(r.customer_merges.get("hh-dup-cust"), Some(&cust_uid));
        // 手機合併前還離線開了一張指向舊客戶的單 → 自動歸到原客戶
        let d2 = WireDoc { uid: "m2".into(), doc_no: "V1-202610-0004".into(), customer_uid: "hh-dup-cust".into(), doc_date: "2026-10-25".into(), status: "issued".into(),
            lines: vec![line(None, "白蝦（大）", "1", 380.0)], ..Default::default() };
        let r2 = sync(&mut db, &mut h, SyncRequest { documents: vec![PushDoc { doc: d2, base_rev: None, deleted: false }], ..Default::default() });
        assert_eq!(r2.results[0].status, "applied");
        assert!(db.list_documents(&Default::default()).unwrap().iter().all(|d| d.customer_id == cid));
    }

    #[test]
    fn pc_edit_confirms_customer() {
        let (mut db, mut h, _cid, _pid) = setup();
        sync(&mut db, &mut h, SyncRequest { customers: vec![WireCustomer { uid: "n1".into(), name: "新客".into(), ..Default::default() }], ..Default::default() });
        let mut c = db.list_customers(true).unwrap().into_iter().find(|c| c.uid == "n1").unwrap();
        assert!(c.needs_review);
        c.code = "C009".into();
        db.save_customer(&c).unwrap();
        assert!(!db.get_customer(c.id.unwrap()).unwrap().needs_review, "電腦上編輯存檔就算確認過");
    }
    #[test]
    fn phone_prints_pc_issued_doc_and_edits_customer() {
        let (mut db, mut h, cid, _pid) = setup();
        let today = Local::now().format("%Y-%m-%d").to_string();
        // 電腦上在家開好、已開立的單 → 手機看得到，還沒出貨
        let issued = db.save_document(&Document { customer_id: cid, doc_date: today.clone(), status: "issued".into(),
            lines: vec![DocLine { name: "白蝦（大）".into(), unit: "斤".into(), qty_text: "2".into(), price: 380.0, ..Default::default() }], ..Default::default() }).unwrap();
        let r = sync(&mut db, &mut h, SyncRequest::default());
        let w = r.documents.iter().find(|d| d.uid == issued.uid).expect("已開立的單也送到手機");
        assert_eq!(w.shipped_at, "");
        // 手機列印給客戶 → 記錄出貨時間，電腦看得到，單據內容不變
        let r2 = sync(&mut db, &mut h, SyncRequest { shipped: vec![ShippedMark { uid: issued.uid.clone(), at: "2026-10-26 09:12:00".into() }], ..Default::default() });
        let back = r2.documents.iter().find(|d| d.uid == issued.uid).expect("出貨時間改了要送回手機");
        assert_eq!(back.shipped_at, "2026-10-26 09:12:00");
        let pc = db.get_document(issued.id.unwrap()).unwrap();
        assert_eq!(pc.shipped_at, "2026-10-26 09:12:00");
        assert_eq!(pc.total, issued.total);
        // 再送一次不會覆蓋第一次的出貨時間
        sync(&mut db, &mut h, SyncRequest { shipped: vec![ShippedMark { uid: issued.uid.clone(), at: "2026-10-27 10:00:00".into() }], ..Default::default() });
        assert_eq!(db.get_document(issued.id.unwrap()).unwrap().shipped_at, "2026-10-26 09:12:00");
        // 手機核對時改了數量（電腦沒動過）→ 套用
        let mut mine = back.clone();
        mine.lines[0].qty_text = "2斤8兩".into();
        let base = mine.rev;
        let r3 = sync(&mut db, &mut h, SyncRequest { documents: vec![PushDoc { doc: mine, base_rev: Some(base), deleted: false }], ..Default::default() });
        assert_eq!(r3.results[0].status, "applied");
        assert_eq!(db.get_document(issued.id.unwrap()).unwrap().lines[0].qty_text, "2斤8兩");

        // 手機修改客戶資料
        let cust = r.customers[0].clone();
        let mut edited = cust.clone();
        edited.phone = "0912-345-678".into();
        edited.tax_id = "12345678".into();
        let r4 = sync(&mut db, &mut h, SyncRequest { customers: vec![edited.clone()], ..Default::default() });
        assert_eq!(r4.results[0].status, "applied");
        let c = db.get_customer(cid).unwrap();
        assert_eq!((c.phone.as_str(), c.tax_id.as_str()), ("0912-345-678", "12345678"));
        assert_eq!(c.code, "C001", "代號、結帳日只能在電腦改");
        // 拿舊的版本再改一次（電腦已經是新版）→ 以電腦為準
        edited.phone = "舊版本".into();
        let r5 = sync(&mut db, &mut h, SyncRequest { customers: vec![edited], ..Default::default() });
        assert_eq!(r5.results[0].status, "conflict");
        assert_eq!(db.get_customer(cid).unwrap().phone, "0912-345-678");
    }

    #[test]
    fn delete_customer_and_product() {
        let (mut db, mut h, cid, pid) = setup();
        sync(&mut db, &mut h, SyncRequest::default());
        // 沒用過的直接刪除，手機收到刪除通知
        let c2 = db.save_customer(&Customer { name: "打錯的客戶".into(), tax_mode: "exclusive".into(), active: true, ..Default::default() }).unwrap();
        let c2uid = db.get_customer(c2).unwrap().uid;
        let p2 = db.save_product(&Product { name: "打錯的品項".into(), active: true, ..Default::default() }).unwrap();
        db.set_customer_price(c2, pid, Some(300.0)).unwrap();
        sync(&mut db, &mut h, SyncRequest::default());
        assert_eq!(db.delete_customer(c2).unwrap(), "deleted");
        assert_eq!(db.delete_product(p2).unwrap(), "deleted");
        let r = sync(&mut db, &mut h, SyncRequest::default());
        assert!(r.deleted.iter().any(|d| d.tbl == "customers" && d.uid == c2uid));
        assert!(r.deleted.iter().any(|d| d.tbl == "products"));
        // 出貨單用過的改成停用，舊單據不受影響
        let doc = db.save_document(&Document { customer_id: cid, doc_date: "2026-10-25".into(), status: "issued".into(),
            lines: vec![DocLine { product_id: Some(pid), name: "白蝦（大）".into(), unit: "斤".into(), qty_text: "1".into(), price: 380.0, ..Default::default() }], ..Default::default() }).unwrap();
        assert_eq!(db.delete_customer(cid).unwrap(), "deactivated");
        assert_eq!(db.delete_product(pid).unwrap(), "deactivated");
        assert!(!db.get_customer(cid).unwrap().active);
        assert_eq!(db.get_document(doc.id.unwrap()).unwrap().cust_name, "時光樹影吧檯");
        let r2 = sync(&mut db, &mut h, SyncRequest::default());
        assert!(r2.customers.iter().any(|c| c.uid == db.get_customer(cid).unwrap().uid && !c.active));
    }
}
