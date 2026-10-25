use anyhow::{anyhow, bail, Context, Result};
use chrono::{Datelike, Local, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

use crate::models::*;
use crate::util::{compute_totals, line_amount, parse_qty};

const SCHEMA_VERSION: i64 = 4;

/// 第 1 版資料表（最早的版本）。獨立出來，測試才能建出真正的舊版資料庫來驗證升級。
const MIGRATION_V1: &str = r#"
BEGIN;
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE customers (
  id INTEGER PRIMARY KEY,
  code TEXT UNIQUE,
  name TEXT NOT NULL,
  tax_id TEXT NOT NULL DEFAULT '',
  phone TEXT NOT NULL DEFAULT '',
  contact TEXT NOT NULL DEFAULT '',
  address TEXT NOT NULL DEFAULT '',
  closing_day INTEGER NOT NULL DEFAULT 0,
  tax_mode TEXT NOT NULL DEFAULT 'exclusive',
  note TEXT NOT NULL DEFAULT '',
  active INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL
);
CREATE TABLE products (
  id INTEGER PRIMARY KEY,
  code TEXT UNIQUE,
  name TEXT NOT NULL,
  unit TEXT NOT NULL DEFAULT '',
  price REAL,
  weighed INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL
);
CREATE TABLE customer_prices (
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  product_id INTEGER NOT NULL REFERENCES products(id),
  price REAL NOT NULL,
  PRIMARY KEY (customer_id, product_id)
);
CREATE TABLE statements (
  id INTEGER PRIMARY KEY,
  stmt_no TEXT NOT NULL UNIQUE,
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  cust_name TEXT NOT NULL,
  period_start TEXT NOT NULL,
  period_end TEXT NOT NULL,
  prev_balance INTEGER NOT NULL,
  period_amount INTEGER NOT NULL,
  paid_in_period INTEGER NOT NULL,
  total_due INTEGER NOT NULL,
  detail_mode TEXT NOT NULL DEFAULT 'summary',
  maker TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE TABLE documents (
  id INTEGER PRIMARY KEY,
  doc_no TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL DEFAULT 'shipment',
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  doc_date TEXT NOT NULL,
  maker TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'draft',
  statement_id INTEGER REFERENCES statements(id),
  cust_name TEXT NOT NULL,
  cust_phone TEXT NOT NULL DEFAULT '',
  cust_address TEXT NOT NULL DEFAULT '',
  tax_mode TEXT NOT NULL DEFAULT 'exclusive',
  subtotal INTEGER NOT NULL DEFAULT 0,
  tax INTEGER NOT NULL DEFAULT 0,
  total INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_documents_customer_date ON documents(customer_id, doc_date);
CREATE TABLE document_lines (
  id INTEGER PRIMARY KEY,
  document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  line_no INTEGER NOT NULL,
  product_id INTEGER REFERENCES products(id),
  name TEXT NOT NULL,
  unit TEXT NOT NULL DEFAULT '',
  qty_text TEXT NOT NULL DEFAULT '',
  qty REAL NOT NULL,
  price REAL NOT NULL,
  amount INTEGER NOT NULL,
  note TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_lines_document ON document_lines(document_id);
CREATE INDEX idx_lines_product ON document_lines(product_id);
CREATE TABLE payments (
  id INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  pay_date TEXT NOT NULL,
  amount INTEGER NOT NULL,
  method TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE INDEX idx_payments_customer_date ON payments(customer_id, pay_date);
PRAGMA user_version = 1;
COMMIT;
"#;

pub struct Db {
    pub conn: Connection,
}

/// 儲存單據時的額外選項（同步用）
#[derive(Default)]
pub(crate) struct SaveOpts {
    /// 新單指定單號（手機開的單）
    pub doc_no: Option<String>,
    /// 新單指定 uid（手機產生的）
    pub uid: Option<String>,
    /// 來源裝置字頭，空白＝電腦
    pub origin: String,
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub(crate) fn now() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub(crate) fn check_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| anyhow!("日期格式不正確：{}", s))
}

fn opt_text(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path).with_context(|| format!("無法開啟資料庫 {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000;")?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let v: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v > SCHEMA_VERSION {
            bail!("資料庫版本比程式新，請改用新版程式開啟");
        }
        if v < 1 {
            self.conn.execute_batch(MIGRATION_V1)?;
        }
        if v < 2 {
            // 第 2 版：手開對帳單（獨立作業，不連品項、不影響未收金額）
            self.conn.execute_batch(
                r#"
BEGIN;
CREATE TABLE manual_statements (
  id INTEGER PRIMARY KEY,
  stmt_no TEXT NOT NULL UNIQUE,
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  period TEXT NOT NULL,
  doc_date TEXT NOT NULL,
  maker TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'draft',
  layout TEXT NOT NULL DEFAULT 'double',
  cust_name TEXT NOT NULL,
  cust_phone TEXT NOT NULL DEFAULT '',
  cust_address TEXT NOT NULL DEFAULT '',
  tax_mode TEXT NOT NULL DEFAULT 'exclusive',
  subtotal INTEGER NOT NULL DEFAULT 0,
  tax INTEGER NOT NULL DEFAULT 0,
  total INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_manual_customer_period ON manual_statements(customer_id, period);
CREATE TABLE manual_statement_lines (
  id INTEGER PRIMARY KEY,
  statement_id INTEGER NOT NULL REFERENCES manual_statements(id) ON DELETE CASCADE,
  line_no INTEGER NOT NULL,
  line_date TEXT NOT NULL,
  amount INTEGER NOT NULL,
  note TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_manual_lines_statement ON manual_statement_lines(statement_id);
PRAGMA user_version = 2;
COMMIT;
"#,
            )?;
        }
        if v < 3 {
            // 第 3 版：手機同步
            //  - uid：跨裝置不變的編號；rev：每次新增或修改都遞增的變更序號；origin：哪一台開的（空白＝電腦）
            //  - 觸發器自動維護 uid、rev，刪除時留下墓碑，讓手機知道要移除
            let mut sql = String::from("BEGIN;\n");
            for t in ["customers", "products", "documents"] {
                sql.push_str(&format!(
                    "ALTER TABLE {t} ADD COLUMN uid TEXT;
ALTER TABLE {t} ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
ALTER TABLE {t} ADD COLUMN origin TEXT NOT NULL DEFAULT '';
UPDATE {t} SET uid = lower(hex(randomblob(16))) WHERE uid IS NULL;
CREATE UNIQUE INDEX idx_{t}_uid ON {t}(uid);
CREATE INDEX idx_{t}_rev ON {t}(rev);
"
                ));
            }
            sql.push_str(
                "ALTER TABLE customers ADD COLUMN needs_review INTEGER NOT NULL DEFAULT 0;
CREATE TABLE sync_counter (v INTEGER NOT NULL);
INSERT INTO sync_counter(v) VALUES (0);
CREATE TABLE tombstones (id INTEGER PRIMARY KEY, tbl TEXT NOT NULL, uid TEXT NOT NULL, rev INTEGER NOT NULL);
CREATE INDEX idx_tombstones_rev ON tombstones(rev);
CREATE TABLE devices (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  prefix TEXT NOT NULL UNIQUE,
  token_hash TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_sync_at TEXT,
  revoked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE pairing_codes (
  code TEXT PRIMARY KEY,
  prefix TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE merges (from_uid TEXT PRIMARY KEY, to_uid TEXT NOT NULL, merged_at TEXT NOT NULL);
CREATE TABLE sync_conflicts (
  id INTEGER PRIMARY KEY,
  doc_uid TEXT NOT NULL,
  doc_no TEXT NOT NULL DEFAULT '',
  device_prefix TEXT NOT NULL DEFAULT '',
  kind TEXT NOT NULL,
  message TEXT NOT NULL DEFAULT '',
  payload TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  resolved INTEGER NOT NULL DEFAULT 0
);
",
            );
            for t in ["customers", "products", "documents"] {
                sql.push_str(&format!(
                    "CREATE TRIGGER {t}_sync_ins AFTER INSERT ON {t} BEGIN
  UPDATE sync_counter SET v = v + 1;
  UPDATE {t} SET uid = COALESCE(uid, lower(hex(randomblob(16)))), rev = (SELECT v FROM sync_counter) WHERE id = NEW.id;
END;
CREATE TRIGGER {t}_sync_upd AFTER UPDATE ON {t} WHEN NEW.rev = OLD.rev BEGIN
  UPDATE sync_counter SET v = v + 1;
  UPDATE {t} SET rev = (SELECT v FROM sync_counter) WHERE id = NEW.id;
END;
CREATE TRIGGER {t}_sync_del AFTER DELETE ON {t} BEGIN
  UPDATE sync_counter SET v = v + 1;
  INSERT INTO tombstones(tbl, uid, rev) VALUES ('{t}', OLD.uid, (SELECT v FROM sync_counter));
END;
"
                ));
            }
            // 既有資料都給一個 rev，手機第一次同步就會全部收到
            sql.push_str(
                "UPDATE sync_counter SET v = 1;
UPDATE customers SET rev = 1; UPDATE products SET rev = 1; UPDATE documents SET rev = 1;
PRAGMA user_version = 3;
COMMIT;",
            );
            self.conn.execute_batch(&sql)?;
        }
        if v < 4 {
            // 第 4 版：出貨單記錄「手機列印給客戶（出貨）」的時間
            self.conn.execute_batch(
                "BEGIN;
ALTER TABLE documents ADD COLUMN shipped_at TEXT NOT NULL DEFAULT '';
PRAGMA user_version = 4;
COMMIT;",
            )?;
        }
        // 之後的結構變更：if v < 5 { ... PRAGMA user_version = 5; }
        Ok(())
    }

    // ---------------- 設定 ----------------

    pub fn get_settings(&self) -> Result<AppSettings> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key='app'", [], |r| r.get(0))
            .optional()?;
        Ok(match v {
            Some(s) => serde_json::from_str(&s).unwrap_or_default(),
            None => AppSettings::default(),
        })
    }

    pub fn save_settings(&self, s: &AppSettings) -> Result<()> {
        let json = serde_json::to_string(s)?;
        self.conn.execute(
            "INSERT INTO settings(key,value) VALUES('app',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![json],
        )?;
        Ok(())
    }

    // ---------------- 客戶 ----------------

    fn row_customer(r: &rusqlite::Row) -> rusqlite::Result<Customer> {
        Ok(Customer {
            id: Some(r.get(0)?),
            code: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            name: r.get(2)?,
            tax_id: r.get(3)?,
            phone: r.get(4)?,
            contact: r.get(5)?,
            address: r.get(6)?,
            closing_day: r.get(7)?,
            tax_mode: r.get(8)?,
            note: r.get(9)?,
            active: r.get::<_, i64>(10)? != 0,
            uid: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
            origin: r.get(12)?,
            needs_review: r.get::<_, i64>(13)? != 0,
        })
    }

    const CUSTOMER_COLS: &'static str =
        "id,code,name,tax_id,phone,contact,address,closing_day,tax_mode,note,active,uid,origin,needs_review";

    pub fn list_customers(&self, include_inactive: bool) -> Result<Vec<Customer>> {
        let sql = format!(
            "SELECT {} FROM customers {} ORDER BY active DESC, COALESCE(code, 'zzzz'), name",
            Self::CUSTOMER_COLS,
            if include_inactive { "" } else { "WHERE active=1" }
        );
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map([], Self::row_customer)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_customer(&self, id: i64) -> Result<Customer> {
        let sql = format!("SELECT {} FROM customers WHERE id=?1", Self::CUSTOMER_COLS);
        self.conn
            .query_row(&sql, params![id], Self::row_customer)
            .optional()?
            .ok_or_else(|| anyhow!("找不到客戶"))
    }

    pub fn save_customer(&self, c: &Customer) -> Result<i64> {
        if c.name.trim().is_empty() {
            bail!("客戶名稱不可空白");
        }
        if !(0..=28).contains(&c.closing_day) {
            bail!("結帳日只能是月底或 1–28 日");
        }
        let tax_mode = match c.tax_mode.as_str() {
            "inclusive" | "none" => c.tax_mode.as_str(),
            _ => "exclusive",
        };
        let code = opt_text(&c.code);
        let res = match c.id {
            Some(id) => self.conn.execute(
                "UPDATE customers SET code=?1,name=?2,tax_id=?3,phone=?4,contact=?5,address=?6,closing_day=?7,tax_mode=?8,note=?9,active=?10,needs_review=0 WHERE id=?11",
                params![code, c.name.trim(), c.tax_id.trim(), c.phone.trim(), c.contact.trim(), c.address.trim(), c.closing_day, tax_mode, c.note, c.active as i64, id],
            ).map(|_| id),
            None => self.conn.execute(
                "INSERT INTO customers(code,name,tax_id,phone,contact,address,closing_day,tax_mode,note,active,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![code, c.name.trim(), c.tax_id.trim(), c.phone.trim(), c.contact.trim(), c.address.trim(), c.closing_day, tax_mode, c.note, c.active as i64, now()],
            ).map(|_| self.conn.last_insert_rowid()),
        };
        res.map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                anyhow!("客戶代號「{}」已經有人使用", c.code.trim())
            }
            other => other.into(),
        })
    }

    // ---------------- 品項 ----------------

    fn row_product(r: &rusqlite::Row) -> rusqlite::Result<Product> {
        Ok(Product {
            id: Some(r.get(0)?),
            code: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            name: r.get(2)?,
            unit: r.get(3)?,
            price: r.get(4)?,
            weighed: r.get::<_, i64>(5)? != 0,
            active: r.get::<_, i64>(6)? != 0,
            uid: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
            origin: r.get(8)?,
        })
    }

    pub fn list_products(&self, include_inactive: bool) -> Result<Vec<Product>> {
        let sql = format!(
            "SELECT id,code,name,unit,price,weighed,active,uid,origin FROM products {} ORDER BY active DESC, COALESCE(code,'zzzz'), name",
            if include_inactive { "" } else { "WHERE active=1" }
        );
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map([], Self::row_product)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn save_product(&self, p: &Product) -> Result<i64> {
        if p.name.trim().is_empty() {
            bail!("品名不可空白");
        }
        if let Some(pr) = p.price {
            if pr < 0.0 {
                bail!("單價不可為負數");
            }
        }
        let code = opt_text(&p.code);
        let res = match p.id {
            Some(id) => self.conn.execute(
                "UPDATE products SET code=?1,name=?2,unit=?3,price=?4,weighed=?5,active=?6 WHERE id=?7",
                params![code, p.name.trim(), p.unit.trim(), p.price, p.weighed as i64, p.active as i64, id],
            ).map(|_| id),
            None => self.conn.execute(
                "INSERT INTO products(code,name,unit,price,weighed,active,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![code, p.name.trim(), p.unit.trim(), p.price, p.weighed as i64, p.active as i64, now()],
            ).map(|_| self.conn.last_insert_rowid()),
        };
        res.map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                anyhow!("品項代號「{}」已經有人使用", p.code.trim())
            }
            other => other.into(),
        })
    }

    pub fn list_customer_prices(&self, product_id: i64) -> Result<Vec<CustomerPrice>> {
        let mut st = self.conn.prepare(
            "SELECT cp.customer_id, c.name, cp.product_id, cp.price FROM customer_prices cp JOIN customers c ON c.id=cp.customer_id WHERE cp.product_id=?1 ORDER BY c.name",
        )?;
        let rows = st.query_map(params![product_id], |r| {
            Ok(CustomerPrice { customer_id: r.get(0)?, customer_name: r.get(1)?, product_id: r.get(2)?, price: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_customer_price(&self, customer_id: i64, product_id: i64, price: Option<f64>) -> Result<()> {
        match price {
            Some(p) => {
                if p < 0.0 {
                    bail!("單價不可為負數");
                }
                self.conn.execute(
                    "INSERT INTO customer_prices(customer_id,product_id,price) VALUES(?1,?2,?3) ON CONFLICT(customer_id,product_id) DO UPDATE SET price=excluded.price",
                    params![customer_id, product_id, p],
                )?;
            }
            None => {
                self.conn.execute(
                    "DELETE FROM customer_prices WHERE customer_id=?1 AND product_id=?2",
                    params![customer_id, product_id],
                )?;
            }
        }
        Ok(())
    }

    /// 單價建議：客戶專屬價 > 此客戶上次成交價 > 品項預設價。
    pub fn suggest_price(&self, customer_id: i64, product_id: i64) -> Result<PriceSuggestion> {
        let special: Option<f64> = self
            .conn
            .query_row(
                "SELECT price FROM customer_prices WHERE customer_id=?1 AND product_id=?2",
                params![customer_id, product_id],
                |r| r.get(0),
            )
            .optional()?;
        let last: Option<f64> = self
            .conn
            .query_row(
                "SELECT l.price FROM document_lines l JOIN documents d ON d.id=l.document_id
                 WHERE d.customer_id=?1 AND l.product_id=?2 AND d.kind='shipment'
                 ORDER BY d.doc_date DESC, d.id DESC LIMIT 1",
                params![customer_id, product_id],
                |r| r.get(0),
            )
            .optional()?;
        let default: Option<f64> = self
            .conn
            .query_row("SELECT price FROM products WHERE id=?1", params![product_id], |r| r.get(0))
            .optional()?
            .flatten();
        let (price, source) = if let Some(p) = special {
            (Some(p), "customer")
        } else if let Some(p) = last {
            (Some(p), "last")
        } else if let Some(p) = default {
            (Some(p), "default")
        } else {
            (None, "none")
        };
        Ok(PriceSuggestion { price, source: source.into(), last_price: last })
    }

    // ---------------- 單據 ----------------

    fn next_number(&self, table: &str, col: &str, prefix: char, date: &str) -> Result<String> {
        let d = check_date(date)?;
        let head = format!("{}{:04}{:02}-", prefix, d.year(), d.month());
        let sql = format!(
            "SELECT MAX(CAST(substr({col}, {start}) AS INTEGER)) FROM {table} WHERE {col} LIKE ?1",
            col = col,
            table = table,
            start = head.chars().count() + 1
        );
        let max: Option<i64> = self.conn.query_row(&sql, params![format!("{}%", head)], |r| r.get(0))?;
        Ok(format!("{}{:04}", head, max.unwrap_or(0) + 1))
    }

    pub fn list_documents(&self, f: &DocFilter) -> Result<Vec<DocSummary>> {
        let mut sql = String::from(
            "SELECT d.id,d.doc_no,d.kind,d.customer_id,d.cust_name,d.doc_date,d.status,d.statement_id,
             (SELECT COUNT(*) FROM document_lines l WHERE l.document_id=d.id), d.total, d.origin, d.shipped_at
             FROM documents d WHERE 1=1",
        );
        let mut args: Vec<rusqlite::types::Value> = vec![];
        if let Some(c) = f.customer_id {
            sql.push_str(" AND d.customer_id=?");
            args.push(c.into());
        }
        if let Some(from) = &f.from {
            sql.push_str(" AND d.doc_date>=?");
            args.push(from.clone().into());
        }
        if let Some(to) = &f.to {
            sql.push_str(" AND d.doc_date<=?");
            args.push(to.clone().into());
        }
        sql.push_str(" ORDER BY d.doc_date DESC, d.id DESC");
        if let Some(l) = f.limit {
            sql.push_str(&format!(" LIMIT {}", l.max(1)));
        }
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(args), |r| {
            Ok(DocSummary {
                id: r.get(0)?,
                doc_no: r.get(1)?,
                kind: r.get(2)?,
                customer_id: r.get(3)?,
                cust_name: r.get(4)?,
                doc_date: r.get(5)?,
                status: r.get(6)?,
                statement_id: r.get(7)?,
                item_count: r.get(8)?,
                total: r.get(9)?,
                origin: r.get(10)?,
                shipped_at: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_document(&self, id: i64) -> Result<Document> {
        let mut doc = self
            .conn
            .query_row(
                "SELECT d.id,d.doc_no,d.kind,d.customer_id,d.doc_date,d.maker,d.note,d.status,d.statement_id,s.stmt_no,
                        d.cust_name,d.cust_phone,d.cust_address,d.tax_mode,d.subtotal,d.tax,d.total,d.uid,d.origin,d.shipped_at
                 FROM documents d LEFT JOIN statements s ON s.id=d.statement_id WHERE d.id=?1",
                params![id],
                |r| {
                    Ok(Document {
                        id: Some(r.get(0)?),
                        doc_no: r.get(1)?,
                        kind: r.get(2)?,
                        customer_id: r.get(3)?,
                        doc_date: r.get(4)?,
                        maker: r.get(5)?,
                        note: r.get(6)?,
                        status: r.get(7)?,
                        statement_id: r.get(8)?,
                        statement_no: r.get(9)?,
                        cust_name: r.get(10)?,
                        cust_phone: r.get(11)?,
                        cust_address: r.get(12)?,
                        tax_mode: r.get(13)?,
                        subtotal: r.get(14)?,
                        tax: r.get(15)?,
                        total: r.get(16)?,
                        uid: r.get::<_, Option<String>>(17)?.unwrap_or_default(),
                        origin: r.get(18)?,
                        shipped_at: r.get(19)?,
                        lines: vec![],
                    })
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("找不到單據"))?;
        let mut st = self.conn.prepare(
            "SELECT id,product_id,name,unit,qty_text,qty,price,amount,note FROM document_lines WHERE document_id=?1 ORDER BY line_no",
        )?;
        let rows = st.query_map(params![id], |r| {
            Ok(DocLine {
                id: Some(r.get(0)?),
                product_id: r.get(1)?,
                name: r.get(2)?,
                unit: r.get(3)?,
                qty_text: r.get(4)?,
                qty: r.get(5)?,
                price: r.get(6)?,
                amount: r.get(7)?,
                note: r.get(8)?,
            })
        })?;
        doc.lines = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(doc)
    }

    fn ensure_unlocked(&self, id: i64) -> Result<()> {
        let stmt: Option<String> = self
            .conn
            .query_row(
                "SELECT s.stmt_no FROM documents d JOIN statements s ON s.id=d.statement_id WHERE d.id=?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(no) = stmt {
            bail!("這張單已列入對帳單 {}，不能修改。要更正請開退貨／折讓單，或先作廢該對帳單。", no);
        }
        Ok(())
    }

    /// 儲存出貨單或退貨折讓單。會重新計算每列金額與合計，並快照客戶資料。
    pub fn save_document(&mut self, input: &Document) -> Result<Document> {
        self.save_document_with(input, &SaveOpts::default())
    }

    /// 同步用：可以指定新單的單號、uid、來源裝置。
    pub(crate) fn save_document_with(&mut self, input: &Document, opts: &SaveOpts) -> Result<Document> {
        check_date(&input.doc_date)?;
        let kind = if input.kind == "adjustment" { "adjustment" } else { "shipment" };
        let status = if input.status == "draft" { "draft" } else { "issued" };
        let cust = self.get_customer(input.customer_id)?;

        let mut lines: Vec<DocLine> = Vec::new();
        for (i, l) in input.lines.iter().enumerate() {
            if l.name.trim().is_empty() && l.qty_text.trim().is_empty() {
                continue; // 空白列略過
            }
            if l.name.trim().is_empty() {
                bail!("第 {} 列沒有品名", i + 1);
            }
            let qty = parse_qty(&l.qty_text).map_err(|e| anyhow!("第 {} 列：{}", i + 1, e))?;
            if l.price < 0.0 {
                bail!("第 {} 列單價不可為負數", i + 1);
            }
            let mut nl = l.clone();
            nl.name = l.name.trim().to_string();
            nl.qty = qty;
            nl.amount = line_amount(qty, l.price);
            lines.push(nl);
        }
        if status == "issued" && lines.is_empty() {
            bail!("至少要有一項品項才能存檔");
        }
        let sign = if kind == "adjustment" { -1 } else { 1 };
        let (subtotal, tax, total) = compute_totals(&lines, &cust.tax_mode, sign);

        if let Some(id) = input.id {
            self.ensure_unlocked(id)?;
        }
        let tx = self.conn.transaction()?;
        let (id, doc_no) = match input.id {
            Some(id) => {
                tx.execute(
                    "UPDATE documents SET kind=?1,customer_id=?2,doc_date=?3,maker=?4,note=?5,status=?6,cust_name=?7,cust_phone=?8,cust_address=?9,tax_mode=?10,subtotal=?11,tax=?12,total=?13,updated_at=?14 WHERE id=?15",
                    params![kind, cust.id, input.doc_date, input.maker.trim(), input.note, status, cust.name, cust.phone, cust.address, cust.tax_mode, subtotal, tax, total, now(), id],
                )?;
                tx.execute("DELETE FROM document_lines WHERE document_id=?1", params![id])?;
                let no: String = tx.query_row("SELECT doc_no FROM documents WHERE id=?1", params![id], |r| r.get(0))?;
                (id, no)
            }
            None => {
                let no = match &opts.doc_no {
                    // 手機開的單：用手機給的單號（字頭 V1-，不會跟電腦撞號）
                    Some(no) => {
                        let used: i64 = tx.query_row("SELECT COUNT(*) FROM documents WHERE doc_no=?1", params![no], |r| r.get(0))?;
                        if used > 0 {
                            bail!("單號 {} 已經被使用", no);
                        }
                        no.clone()
                    }
                    None => {
                        let prefix = if kind == "adjustment" { 'A' } else { 'S' };
                        // next_number 需要 &self，交易中用同一連線查詢
                        let d = check_date(&input.doc_date)?;
                        let head = format!("{}{:04}{:02}-", prefix, d.year(), d.month());
                        let max: Option<i64> = tx.query_row(
                            &format!("SELECT MAX(CAST(substr(doc_no, {}) AS INTEGER)) FROM documents WHERE doc_no LIKE ?1", head.len() + 1),
                            params![format!("{}%", head)],
                            |r| r.get(0),
                        )?;
                        format!("{}{:04}", head, max.unwrap_or(0) + 1)
                    }
                };
                tx.execute(
                    "INSERT INTO documents(doc_no,kind,customer_id,doc_date,maker,note,status,cust_name,cust_phone,cust_address,tax_mode,subtotal,tax,total,created_at,updated_at,uid,origin) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15,?16,?17)",
                    params![no, kind, cust.id, input.doc_date, input.maker.trim(), input.note, status, cust.name, cust.phone, cust.address, cust.tax_mode, subtotal, tax, total, now(), opts.uid, opts.origin],
                )?;
                (tx.last_insert_rowid(), no)
            }
        };
        // 開單時直接輸入的新品名：同名品項已存在就連過去，否則自動加入品項清單
        // （單位、單價一起存；代號留空，之後到「品項」補代號或改售價）
        for l in lines.iter_mut() {
            if l.product_id.is_some() {
                continue;
            }
            let existing: Option<i64> = tx
                .query_row("SELECT id FROM products WHERE name=?1 ORDER BY active DESC, id LIMIT 1", params![l.name], |r| r.get(0))
                .optional()?;
            l.product_id = Some(match existing {
                Some(pid) => pid,
                None => {
                    let weighed = l.unit.contains('斤') || l.unit.contains('兩') || l.qty_text.contains('斤') || l.qty_text.contains('兩');
                    let price = if l.price > 0.0 { Some(l.price) } else { None };
                    tx.execute(
                        "INSERT INTO products(code,name,unit,price,weighed,active,created_at,origin) VALUES(NULL,?1,?2,?3,?4,1,?5,?6)",
                        params![l.name, l.unit.trim(), price, weighed as i64, now(), opts.origin],
                    )?;
                    tx.last_insert_rowid()
                }
            });
        }
        for (i, l) in lines.iter().enumerate() {
            tx.execute(
                "INSERT INTO document_lines(document_id,line_no,product_id,name,unit,qty_text,qty,price,amount,note) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![id, i as i64 + 1, l.product_id, l.name, l.unit.trim(), l.qty_text.trim(), l.qty, l.price, l.amount, l.note.trim()],
            )?;
        }
        tx.commit()?;
        let _ = doc_no;
        self.get_document(id)
    }

    /// 刪除客戶：沒有任何單據、收款、對帳單的直接刪除；用過的改為停用（舊單據不受影響）
    /// 回傳 "deleted" 或 "deactivated"
    pub fn delete_customer(&self, id: i64) -> Result<String> {
        let used: i64 = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM documents WHERE customer_id=?1)
                  + (SELECT COUNT(*) FROM payments WHERE customer_id=?1)
                  + (SELECT COUNT(*) FROM statements WHERE customer_id=?1)
                  + (SELECT COUNT(*) FROM manual_statements WHERE customer_id=?1)",
            params![id],
            |r| r.get(0),
        )?;
        if used > 0 {
            self.conn.execute("UPDATE customers SET active=0, needs_review=0 WHERE id=?1", params![id])?;
            return Ok("deactivated".into());
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM customer_prices WHERE customer_id=?1", params![id])?;
        tx.execute("DELETE FROM customers WHERE id=?1", params![id])?;
        tx.commit()?;
        Ok("deleted".into())
    }

    /// 刪除品項：出貨單用過的改為停用，沒用過的直接刪除
    pub fn delete_product(&self, id: i64) -> Result<String> {
        let used: i64 = self.conn.query_row("SELECT COUNT(*) FROM document_lines WHERE product_id=?1", params![id], |r| r.get(0))?;
        if used > 0 {
            self.conn.execute("UPDATE products SET active=0 WHERE id=?1", params![id])?;
            return Ok("deactivated".into());
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM customer_prices WHERE product_id=?1", params![id])?;
        tx.execute("DELETE FROM products WHERE id=?1", params![id])?;
        tx.commit()?;
        Ok("deleted".into())
    }

    pub fn delete_document(&self, id: i64) -> Result<()> {
        self.ensure_unlocked(id)?;
        self.conn.execute("DELETE FROM document_lines WHERE document_id=?1", params![id])?;
        self.conn.execute("DELETE FROM documents WHERE id=?1", params![id])?;
        Ok(())
    }

    /// 列印後把草稿轉為正式單據。
    pub fn mark_issued(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE documents SET status='issued' WHERE id=?1 AND status='draft'", params![id])?;
        Ok(())
    }

    // ---------------- 收款 ----------------

    pub fn add_payment(&self, p: &Payment) -> Result<i64> {
        check_date(&p.pay_date)?;
        if p.amount == 0 {
            bail!("收款金額不可為 0");
        }
        self.get_customer(p.customer_id)?;
        self.conn.execute(
            "INSERT INTO payments(customer_id,pay_date,amount,method,note,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![p.customer_id, p.pay_date, p.amount, p.method.trim(), p.note.trim(), now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_payments(&self, customer_id: i64, from: Option<&str>, to: Option<&str>) -> Result<Vec<Payment>> {
        let mut st = self.conn.prepare(
            "SELECT id,customer_id,pay_date,amount,method,note FROM payments
             WHERE customer_id=?1 AND pay_date>=?2 AND pay_date<=?3 ORDER BY pay_date DESC, id DESC",
        )?;
        let rows = st.query_map(params![customer_id, from.unwrap_or("0000-00-00"), to.unwrap_or("9999-12-31")], |r| {
            Ok(Payment {
                id: Some(r.get(0)?),
                customer_id: r.get(1)?,
                pay_date: r.get(2)?,
                amount: r.get(3)?,
                method: r.get(4)?,
                note: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_payment(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM payments WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn customer_balances(&self) -> Result<Vec<CustomerBalance>> {
        let mut st = self.conn.prepare(
            "SELECT c.id,
               COALESCE((SELECT SUM(total) FROM documents d WHERE d.customer_id=c.id AND d.status='issued'),0)
             - COALESCE((SELECT SUM(amount) FROM payments p WHERE p.customer_id=c.id),0)
             FROM customers c",
        )?;
        let rows = st.query_map([], |r| Ok(CustomerBalance { customer_id: r.get(0)?, balance: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------------- 對帳單 ----------------

    /// 依客戶結帳日算出某月份的對帳期間。
    pub fn statement_period(&self, customer_id: i64, year: i32, month: u32) -> Result<(String, String)> {
        let c = self.get_customer(customer_id)?;
        period_for(c.closing_day, year, month)
    }

    pub fn preview_statement(&self, customer_id: i64, start: &str, end: &str) -> Result<StatementPreview> {
        check_date(start)?;
        check_date(end)?;
        if start > end {
            bail!("對帳期間的開始日不能晚於結束日");
        }
        let c = self.get_customer(customer_id)?;
        let before_docs: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(total),0) FROM documents WHERE customer_id=?1 AND status='issued' AND doc_date<?2",
            params![customer_id, start],
            |r| r.get(0),
        )?;
        let before_pay: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(amount),0) FROM payments WHERE customer_id=?1 AND pay_date<?2",
            params![customer_id, start],
            |r| r.get(0),
        )?;
        let all = self.list_documents(&DocFilter {
            customer_id: Some(customer_id),
            from: Some(start.into()),
            to: Some(end.into()),
            limit: None,
        })?;
        let drafts_excluded = all.iter().filter(|d| d.status == "draft").count() as i64;
        let already_locked = all.iter().filter(|d| d.status == "issued" && d.statement_id.is_some()).count() as i64;
        let mut docs: Vec<DocSummary> = all.into_iter().filter(|d| d.status == "issued").collect();
        docs.sort_by(|a, b| a.doc_date.cmp(&b.doc_date).then(a.id.cmp(&b.id)));
        let period_amount: i64 = docs.iter().map(|d| d.total).sum();
        let payments = self.list_payments(customer_id, Some(start), Some(end))?;
        let paid_in_period: i64 = payments.iter().map(|p| p.amount).sum();
        let prev_balance = before_docs - before_pay;
        Ok(StatementPreview {
            customer_id,
            cust_name: c.name,
            period_start: start.into(),
            period_end: end.into(),
            prev_balance,
            docs,
            period_amount,
            payments,
            paid_in_period,
            total_due: prev_balance + period_amount - paid_in_period,
            drafts_excluded,
            already_locked,
        })
    }

    pub fn create_statement(&mut self, customer_id: i64, start: &str, end: &str, detail_mode: &str, maker: &str) -> Result<Statement> {
        let p = self.preview_statement(customer_id, start, end)?;
        if p.already_locked > 0 {
            bail!("這段期間有 {} 張單已經列入其他對帳單，請先作廢那張對帳單或調整期間", p.already_locked);
        }
        let mode = if detail_mode == "detail" { "detail" } else { "summary" };
        let no = self.next_number("statements", "stmt_no", 'T', end)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO statements(stmt_no,customer_id,cust_name,period_start,period_end,prev_balance,period_amount,paid_in_period,total_due,detail_mode,maker,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![no, customer_id, p.cust_name, start, end, p.prev_balance, p.period_amount, p.paid_in_period, p.total_due, mode, maker.trim(), now()],
        )?;
        let sid = tx.last_insert_rowid();
        for d in &p.docs {
            tx.execute("UPDATE documents SET statement_id=?1 WHERE id=?2", params![sid, d.id])?;
        }
        tx.commit()?;
        self.get_statement(sid)
    }

    fn row_statement(r: &rusqlite::Row) -> rusqlite::Result<Statement> {
        Ok(Statement {
            id: r.get(0)?,
            stmt_no: r.get(1)?,
            customer_id: r.get(2)?,
            cust_name: r.get(3)?,
            period_start: r.get(4)?,
            period_end: r.get(5)?,
            prev_balance: r.get(6)?,
            period_amount: r.get(7)?,
            paid_in_period: r.get(8)?,
            total_due: r.get(9)?,
            detail_mode: r.get(10)?,
            maker: r.get(11)?,
            created_at: r.get(12)?,
        })
    }

    const STMT_COLS: &'static str = "id,stmt_no,customer_id,cust_name,period_start,period_end,prev_balance,period_amount,paid_in_period,total_due,detail_mode,maker,created_at";

    pub fn get_statement(&self, id: i64) -> Result<Statement> {
        let sql = format!("SELECT {} FROM statements WHERE id=?1", Self::STMT_COLS);
        self.conn
            .query_row(&sql, params![id], Self::row_statement)
            .optional()?
            .ok_or_else(|| anyhow!("找不到對帳單"))
    }

    pub fn list_statements(&self, customer_id: Option<i64>) -> Result<Vec<Statement>> {
        let sql = format!(
            "SELECT {} FROM statements WHERE (?1 IS NULL OR customer_id=?1) ORDER BY period_end DESC, id DESC LIMIT 200",
            Self::STMT_COLS
        );
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(params![customer_id], Self::row_statement)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn statement_documents(&self, statement_id: i64) -> Result<Vec<Document>> {
        let mut st = self
            .conn
            .prepare("SELECT id FROM documents WHERE statement_id=?1 ORDER BY doc_date, id")?;
        let ids = st
            .query_map(params![statement_id], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.get_document(id)).collect()
    }

    /// 作廢對帳單：刪除對帳單並解除單據鎖定。
    pub fn void_statement(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE documents SET statement_id=NULL WHERE statement_id=?1", params![id])?;
        tx.execute("DELETE FROM statements WHERE id=?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    // ---------------- 手開對帳單 ----------------

    pub fn list_manual(&self, customer_id: Option<i64>, period: Option<&str>, limit: Option<i64>) -> Result<Vec<ManualSummary>> {
        let mut sql = String::from(
            "SELECT m.id,m.stmt_no,m.customer_id,m.cust_name,m.period,m.doc_date,m.status,
             (SELECT COUNT(*) FROM manual_statement_lines l WHERE l.statement_id=m.id), m.total
             FROM manual_statements m WHERE (?1 IS NULL OR m.customer_id=?1) AND (?2 IS NULL OR m.period=?2)
             ORDER BY m.period DESC, m.id DESC",
        );
        if let Some(l) = limit {
            sql.push_str(&format!(" LIMIT {}", l.max(1)));
        }
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(params![customer_id, period], |r| {
            Ok(ManualSummary {
                id: r.get(0)?,
                stmt_no: r.get(1)?,
                customer_id: r.get(2)?,
                cust_name: r.get(3)?,
                period: r.get(4)?,
                doc_date: r.get(5)?,
                status: r.get(6)?,
                line_count: r.get(7)?,
                total: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_manual(&self, id: i64) -> Result<ManualStatement> {
        let mut m = self
            .conn
            .query_row(
                "SELECT id,stmt_no,customer_id,period,doc_date,maker,note,status,layout,cust_name,cust_phone,cust_address,tax_mode,subtotal,tax,total
                 FROM manual_statements WHERE id=?1",
                params![id],
                |r| {
                    Ok(ManualStatement {
                        id: Some(r.get(0)?),
                        stmt_no: r.get(1)?,
                        customer_id: r.get(2)?,
                        period: r.get(3)?,
                        doc_date: r.get(4)?,
                        maker: r.get(5)?,
                        note: r.get(6)?,
                        status: r.get(7)?,
                        layout: r.get(8)?,
                        cust_name: r.get(9)?,
                        cust_phone: r.get(10)?,
                        cust_address: r.get(11)?,
                        tax_mode: r.get(12)?,
                        subtotal: r.get(13)?,
                        tax: r.get(14)?,
                        total: r.get(15)?,
                        lines: vec![],
                    })
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("找不到對帳單"))?;
        let mut st = self.conn.prepare(
            "SELECT id,line_date,amount,note FROM manual_statement_lines WHERE statement_id=?1 ORDER BY line_no",
        )?;
        let rows = st.query_map(params![id], |r| {
            Ok(ManualLine { id: Some(r.get(0)?), line_date: r.get(1)?, amount: r.get(2)?, note: r.get(3)? })
        })?;
        m.lines = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(m)
    }

    /// 儲存手開對帳單：檢查日期、依日期排序、重算合計並快照客戶資料。
    pub fn save_manual(&mut self, input: &ManualStatement) -> Result<ManualStatement> {
        let period = input.period.trim();
        check_date(&format!("{}-01", period)).map_err(|_| anyhow!("對帳月份格式不正確：{}", period))?;
        check_date(&input.doc_date)?;
        let status = if input.status == "draft" { "draft" } else { "issued" };
        let layout = if input.layout == "single" { "single" } else { "double" };
        let cust = self.get_customer(input.customer_id)?;

        let mut lines: Vec<ManualLine> = Vec::new();
        for (i, l) in input.lines.iter().enumerate() {
            let d = l.line_date.trim();
            if d.is_empty() && l.amount == 0 && l.note.trim().is_empty() {
                continue; // 空白列略過
            }
            if d.is_empty() {
                bail!("第 {} 列沒有日期", i + 1);
            }
            check_date(d).map_err(|_| anyhow!("第 {} 列日期不正確：{}", i + 1, d))?;
            lines.push(ManualLine { id: None, line_date: d.to_string(), amount: l.amount, note: l.note.trim().to_string() });
        }
        if status == "issued" && lines.is_empty() {
            bail!("至少要有一筆日期才能存檔");
        }
        // 依日期排序（同一天維持輸入順序）
        lines.sort_by(|a, b| a.line_date.cmp(&b.line_date));
        let sum: i64 = lines.iter().map(|l| l.amount).sum();
        let (subtotal, tax, total) = crate::util::totals_from_sum(sum, &cust.tax_mode);

        let tx = self.conn.transaction()?;
        let id = match input.id {
            Some(id) => {
                tx.execute(
                    "UPDATE manual_statements SET customer_id=?1,period=?2,doc_date=?3,maker=?4,note=?5,status=?6,layout=?7,cust_name=?8,cust_phone=?9,cust_address=?10,tax_mode=?11,subtotal=?12,tax=?13,total=?14,updated_at=?15 WHERE id=?16",
                    params![cust.id, period, input.doc_date, input.maker.trim(), input.note, status, layout, cust.name, cust.phone, cust.address, cust.tax_mode, subtotal, tax, total, now(), id],
                )?;
                tx.execute("DELETE FROM manual_statement_lines WHERE statement_id=?1", params![id])?;
                id
            }
            None => {
                let head = format!("M{}-", period.replace('-', ""));
                let max: Option<i64> = tx.query_row(
                    &format!("SELECT MAX(CAST(substr(stmt_no, {}) AS INTEGER)) FROM manual_statements WHERE stmt_no LIKE ?1", head.len() + 1),
                    params![format!("{}%", head)],
                    |r| r.get(0),
                )?;
                let no = format!("{}{:04}", head, max.unwrap_or(0) + 1);
                tx.execute(
                    "INSERT INTO manual_statements(stmt_no,customer_id,period,doc_date,maker,note,status,layout,cust_name,cust_phone,cust_address,tax_mode,subtotal,tax,total,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?16)",
                    params![no, cust.id, period, input.doc_date, input.maker.trim(), input.note, status, layout, cust.name, cust.phone, cust.address, cust.tax_mode, subtotal, tax, total, now()],
                )?;
                tx.last_insert_rowid()
            }
        };
        for (i, l) in lines.iter().enumerate() {
            tx.execute(
                "INSERT INTO manual_statement_lines(statement_id,line_no,line_date,amount,note) VALUES(?1,?2,?3,?4,?5)",
                params![id, i as i64 + 1, l.line_date, l.amount, l.note],
            )?;
        }
        tx.commit()?;
        self.get_manual(id)
    }

    pub fn delete_manual(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM manual_statement_lines WHERE statement_id=?1", params![id])?;
        self.conn.execute("DELETE FROM manual_statements WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn mark_manual_issued(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE manual_statements SET status='issued' WHERE id=?1 AND status='draft'", params![id])?;
        Ok(())
    }

    // ---------------- 備份 ----------------

    /// 用 VACUUM INTO 產生一致的備份檔（資料庫開著也安全）。
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        self.conn.execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
        Ok(())
    }
}

pub fn period_for(closing_day: i64, year: i32, month: u32) -> Result<(String, String)> {
    let first = NaiveDate::from_ymd_opt(year, month, 1).ok_or_else(|| anyhow!("月份不正確"))?;
    let next_first = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
    };
    if closing_day <= 0 {
        let last = next_first.pred_opt().unwrap();
        return Ok((first.format("%Y-%m-%d").to_string(), last.format("%Y-%m-%d").to_string()));
    }
    let d = closing_day as u32;
    let end = NaiveDate::from_ymd_opt(year, month, d).ok_or_else(|| anyhow!("結帳日不正確"))?;
    let (py, pm) = if month == 1 { (year - 1, 12) } else { (year, month - 1) };
    let start = NaiveDate::from_ymd_opt(py, pm, d).unwrap().succ_opt().unwrap();
    Ok((start.format("%Y-%m-%d").to_string(), end.format("%Y-%m-%d").to_string()))
}

pub fn today_string() -> String {
    today()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Db, i64, i64) {
        let db = Db::open_in_memory().unwrap();
        let cid = db
            .save_customer(&Customer { name: "時光樹影吧檯".into(), tax_mode: "exclusive".into(), active: true, ..Default::default() })
            .unwrap();
        let pid = db
            .save_product(&Product { name: "透抽".into(), unit: "斤".into(), price: Some(300.0), weighed: true, active: true, ..Default::default() })
            .unwrap();
        (db, cid, pid)
    }

    fn line(pid: i64, q: &str, price: f64) -> DocLine {
        DocLine { product_id: Some(pid), name: "透抽".into(), unit: "斤".into(), qty_text: q.into(), price, ..Default::default() }
    }

    #[test]
    fn document_flow() {
        let (mut db, cid, pid) = setup();
        let doc = db
            .save_document(&Document {
                customer_id: cid,
                doc_date: "2026-01-15".into(),
                status: "issued".into(),
                lines: vec![line(pid, "2斤5兩", 300.0), DocLine::default()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(doc.doc_no, "S202601-0001");
        assert_eq!(doc.lines.len(), 1);
        assert_eq!(doc.subtotal, 694);
        assert_eq!(doc.tax, 35);
        assert_eq!(doc.total, 729);

        let doc2 = db
            .save_document(&Document { customer_id: cid, doc_date: "2026-01-20".into(), status: "issued".into(), lines: vec![line(pid, "1", 320.0)], ..Default::default() })
            .unwrap();
        assert_eq!(doc2.doc_no, "S202601-0002");

        // 上次成交價
        let s = db.suggest_price(cid, pid).unwrap();
        assert_eq!(s.source, "last");
        assert_eq!(s.price, Some(320.0));
        db.set_customer_price(cid, pid, Some(280.0)).unwrap();
        assert_eq!(db.suggest_price(cid, pid).unwrap().source, "customer");

        // 退貨
        let adj = db
            .save_document(&Document { kind: "adjustment".into(), customer_id: cid, doc_date: "2026-01-21".into(), status: "issued".into(), lines: vec![line(pid, "1", 300.0)], ..Default::default() })
            .unwrap();
        assert_eq!(adj.doc_no, "A202601-0001");
        assert_eq!(adj.total, -315);

        db.add_payment(&Payment { customer_id: cid, pay_date: "2025-12-31".into(), amount: 100, ..Default::default() }).unwrap();
        let (a, b) = db.statement_period(cid, 2026, 1).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("2026-01-01", "2026-01-31"));
        let st = db.create_statement(cid, &a, &b, "summary", "").unwrap();
        assert_eq!(st.stmt_no, "T202601-0001");
        assert_eq!(st.prev_balance, -100);
        assert_eq!(st.period_amount, 729 + 336 - 315);
        assert_eq!(st.total_due, -100 + 750);

        // 已鎖定
        let mut locked = db.get_document(doc.id.unwrap()).unwrap();
        locked.note = "x".into();
        assert!(db.save_document(&locked).is_err());
        assert!(db.create_statement(cid, &a, &b, "summary", "").is_err());
        db.void_statement(st.id).unwrap();
        assert!(db.save_document(&locked).is_ok());
    }

    #[test]
    fn new_products_from_shipment() {
        let (mut db, cid, pid) = setup();
        let free = |n: &str, u: &str, q: &str, p: f64| DocLine { product_id: None, name: n.into(), unit: u.into(), qty_text: q.into(), price: p, ..Default::default() };
        let doc = db
            .save_document(&Document {
                customer_id: cid,
                doc_date: "2026-10-25".into(),
                status: "draft".into(),
                lines: vec![
                    free("海鮮煎餅", "盒", "2", 50.0),
                    free("透抽", "斤", "1", 320.0),     // 已有同名品項：連過去，不重複新增
                    free("沙公", "隻", "1", 0.0),       // 還沒定價：存成時價
                    free("海鮮煎餅", "盒", "1", 50.0),  // 同一張單出現兩次：只新增一次
                ],
                ..Default::default()
            })
            .unwrap();
        let products = db.list_products(true).unwrap();
        assert_eq!(products.len(), 3, "{:?}", products.iter().map(|p| &p.name).collect::<Vec<_>>());
        let cake = products.iter().find(|p| p.name == "海鮮煎餅").unwrap();
        assert_eq!((cake.unit.as_str(), cake.price, cake.weighed, cake.code.as_str()), ("盒", Some(50.0), false, ""));
        let crab = products.iter().find(|p| p.name == "沙公").unwrap();
        assert_eq!(crab.price, None);
        // 每一列都連到品項
        assert!(doc.lines.iter().all(|l| l.product_id.is_some()));
        assert_eq!(doc.lines[1].product_id, Some(pid));
        assert_eq!(doc.lines[0].product_id, doc.lines[3].product_id);
        // 再存一次不會重複新增
        db.save_document(&doc).unwrap();
        assert_eq!(db.list_products(true).unwrap().len(), 3);
        // 秤重單位自動勾選秤重
        db.save_document(&Document { customer_id: cid, doc_date: "2026-10-25".into(), status: "draft".into(), lines: vec![free("白蝦", "", "2斤5兩", 380.0)], ..Default::default() }).unwrap();
        assert!(db.list_products(true).unwrap().iter().find(|p| p.name == "白蝦").unwrap().weighed);
    }

    #[test]
    fn manual_statement_flow() {
        let (mut db, cid, _pid) = setup();
        let ml = |d: &str, a: i64, n: &str| ManualLine { id: None, line_date: d.into(), amount: a, note: n.into() };
        let m = db
            .save_manual(&ManualStatement {
                customer_id: cid,
                period: "2026-10".into(),
                doc_date: "2026-10-31".into(),
                status: "issued".into(),
                layout: "single".into(),
                lines: vec![ml("2026-10-05", 3960, ""), ml("2026-10-01", 9065, "含冷凍品"), ManualLine::default(), ml("2026-10-03", -630, "退貨")],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(m.stmt_no, "M202610-0001");
        assert_eq!(m.layout, "single");
        // 空白列略過、依日期排序
        let dates: Vec<&str> = m.lines.iter().map(|l| l.line_date.as_str()).collect();
        assert_eq!(dates, ["2026-10-01", "2026-10-03", "2026-10-05"]);
        assert_eq!(m.subtotal, 12395);
        assert_eq!(m.tax, 620);
        assert_eq!(m.total, 13015);
        assert_eq!(m.cust_name, "時光樹影吧檯");

        // 第二張流水號、修改
        let m2 = db.save_manual(&ManualStatement { customer_id: cid, period: "2026-10".into(), doc_date: "2026-10-31".into(), status: "draft".into(), ..Default::default() }).unwrap();
        assert_eq!(m2.stmt_no, "M202610-0002");
        assert_eq!(m2.layout, "double");
        let mut edit = m.clone();
        edit.lines.push(ml("2026-10-02", 1000, ""));
        let edited = db.save_manual(&edit).unwrap();
        assert_eq!(edited.stmt_no, "M202610-0001");
        assert_eq!(edited.lines.len(), 4);

        // 不影響客戶未收金額
        let bal = db.customer_balances().unwrap();
        assert_eq!(bal.iter().find(|b| b.customer_id == cid).unwrap().balance, 0);

        // 日期錯誤
        let bad = ManualStatement { customer_id: cid, period: "2026-10".into(), doc_date: "2026-10-31".into(), status: "issued".into(), lines: vec![ml("2026-13-01", 1, "")], ..Default::default() };
        assert!(db.save_manual(&bad).is_err());

        assert_eq!(db.list_manual(Some(cid), Some("2026-10"), None).unwrap().len(), 2);
        db.delete_manual(m2.id.unwrap()).unwrap();
        assert_eq!(db.list_manual(None, None, None).unwrap().len(), 1);
    }

    #[test]
    fn upgrade_from_v1_keeps_data() {
        let dir = std::env::temp_dir().join(format!("hs-upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shipping.db");
        let _ = std::fs::remove_file(&path);
        {
            // 建一個真正的第 1 版資料庫，放進客戶、品項、出貨單
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(
                "INSERT INTO customers(id,code,name,created_at) VALUES(1,'C001','舊客戶','2026-01-01');
                 INSERT INTO products(id,code,name,unit,price,created_at) VALUES(1,'SH01','白蝦','斤',380,'2026-01-01');
                 INSERT INTO documents(id,doc_no,customer_id,doc_date,status,cust_name,created_at,updated_at,total) VALUES(1,'S202601-0001',1,'2026-01-05','issued','舊客戶','x','x',399);
                 INSERT INTO document_lines(document_id,line_no,product_id,name,qty,price,amount) VALUES(1,1,1,'白蝦',1,380,380);",
            )
            .unwrap();
            let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
            assert_eq!(v, 1);
        }
        let db = Db::open(&path).unwrap();
        let v: i64 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let c = db.list_customers(true).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].uid.len(), 32, "舊資料要補上 uid");
        assert!(!c[0].needs_review);
        assert_eq!(db.list_products(true).unwrap()[0].uid.len(), 32);
        let d = db.get_document(1).unwrap();
        assert_eq!((d.doc_no.as_str(), d.total, d.lines.len()), ("S202601-0001", 399, 1));
        assert_eq!(d.uid.len(), 32);
        assert!(db.list_manual(None, None, None).unwrap().is_empty());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn periods() {
        assert_eq!(period_for(25, 2026, 1).unwrap(), ("2025-12-26".into(), "2026-01-25".into()));
        assert_eq!(period_for(0, 2024, 2).unwrap(), ("2024-02-01".into(), "2024-02-29".into()));
    }
}
