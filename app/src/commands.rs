use serde::Serialize;
use shipping_core::db::Db;
use shipping_core::models::*;
use shipping_core::print::{self, RenderedDoc};
use shipping_core::server::{SharedDb, SyncServer};
use shipping_core::sync::{Device, PairingCode, SyncConflict};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

use crate::{printer, storage};

pub struct AppState {
    /// 畫面與同步服務共用同一個資料庫連線
    pub db: SharedDb,
    pub data_dir: PathBuf,
    pub init_error: Option<String>,
    pub sync_server: Mutex<Option<SyncServer>>,
    /// 同步服務狀態：None＝運作中或未啟用；Some＝啟動失敗的原因
    pub sync_error: Mutex<Option<String>>,
}

/// 依設定啟動（或重新啟動）手機同步服務
pub fn start_sync_server(state: &AppState) {
    let settings = state.db.lock().ok().and_then(|g| g.as_ref().and_then(|d| d.get_settings().ok()));
    let mut slot = state.sync_server.lock().unwrap();
    if let Some(mut old) = slot.take() {
        old.stop();
    }
    let mut err = state.sync_error.lock().unwrap();
    *err = None;
    let Some(settings) = settings else { return };
    if !settings.sync_enabled {
        return;
    }
    match SyncServer::start(state.db.clone(), settings.sync_port) {
        Ok(srv) => *slot = Some(srv),
        Err(e) => *err = Some(format!("{:#}", e)),
    }
}

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{:#}", e)
}

fn with_db<T>(state: &State<'_, AppState>, f: impl FnOnce(&mut Db) -> anyhow::Result<T>) -> R<T> {
    let mut guard = state.db.lock().map_err(|_| "資料庫鎖定失敗，請重新開啟程式".to_string())?;
    let db = guard
        .as_mut()
        .ok_or_else(|| state.init_error.clone().unwrap_or_else(|| "資料庫未開啟".into()))?;
    f(db).map_err(err)
}

#[derive(Serialize)]
pub struct AppInfo {
    data_dir: String,
    init_error: Option<String>,
    version: String,
    last_backup: Option<String>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        data_dir: state.data_dir.display().to_string(),
        init_error: state.init_error.clone(),
        version: env!("CARGO_PKG_VERSION").into(),
        last_backup: storage::last_backup(&state.data_dir),
    }
}

// ---------- 設定 ----------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> R<AppSettings> {
    with_db(&state, |db| db.get_settings())
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> R<()> {
    let p = &settings.print;
    if p.page_lines == 0 || p.page_lines > 127 {
        return Err("每頁行數要在 1–127 之間".into());
    }
    if p.top_lines + p.bottom_lines + 20 > p.page_lines {
        return Err("上下留白太多，每頁至少要留 20 行可列印".into());
    }
    if p.total_cols < 60 || p.total_cols > 136 {
        return Err("每行字數要在 60–136 之間".into());
    }
    if settings.sync_port < 1024 {
        return Err("同步連接埠要在 1024–65535 之間".into());
    }
    with_db(&state, |db| db.save_settings(&settings))
}

#[tauri::command]
pub fn list_printers() -> R<serde_json::Value> {
    let list = printer::list_printers()?;
    Ok(serde_json::json!({ "printers": list, "default": printer::default_printer() }))
}

#[tauri::command]
pub fn check_text(text: String) -> Vec<String> {
    print::non_big5_chars(&text).into_iter().map(|c| c.to_string()).collect()
}

// ---------- 客戶 ----------

#[tauri::command]
pub fn list_customers(state: State<'_, AppState>, include_inactive: bool) -> R<Vec<Customer>> {
    with_db(&state, |db| db.list_customers(include_inactive))
}

#[tauri::command]
pub fn save_customer(state: State<'_, AppState>, customer: Customer) -> R<i64> {
    with_db(&state, |db| db.save_customer(&customer))
}

#[tauri::command]
pub fn delete_customer(state: State<'_, AppState>, id: i64) -> R<String> {
    with_db(&state, |db| db.delete_customer(id))
}

#[tauri::command]
pub fn delete_product(state: State<'_, AppState>, id: i64) -> R<String> {
    with_db(&state, |db| db.delete_product(id))
}

#[tauri::command]
pub fn customer_balances(state: State<'_, AppState>) -> R<Vec<CustomerBalance>> {
    with_db(&state, |db| db.customer_balances())
}

// ---------- 品項 ----------

#[tauri::command]
pub fn list_products(state: State<'_, AppState>, include_inactive: bool) -> R<Vec<Product>> {
    with_db(&state, |db| db.list_products(include_inactive))
}

#[tauri::command]
pub fn save_product(state: State<'_, AppState>, product: Product) -> R<i64> {
    with_db(&state, |db| db.save_product(&product))
}

#[tauri::command]
pub fn list_customer_prices(state: State<'_, AppState>, product_id: i64) -> R<Vec<CustomerPrice>> {
    with_db(&state, |db| db.list_customer_prices(product_id))
}

#[tauri::command]
pub fn set_customer_price(state: State<'_, AppState>, customer_id: i64, product_id: i64, price: Option<f64>) -> R<()> {
    with_db(&state, |db| db.set_customer_price(customer_id, product_id, price))
}

#[tauri::command]
pub fn suggest_price(state: State<'_, AppState>, customer_id: i64, product_id: i64) -> R<PriceSuggestion> {
    with_db(&state, |db| db.suggest_price(customer_id, product_id))
}

// ---------- 單據 ----------

#[tauri::command]
pub fn list_documents(state: State<'_, AppState>, filter: DocFilter) -> R<Vec<DocSummary>> {
    with_db(&state, |db| db.list_documents(&filter))
}

#[tauri::command]
pub fn get_document(state: State<'_, AppState>, id: i64) -> R<Document> {
    with_db(&state, |db| db.get_document(id))
}

#[tauri::command]
pub fn save_document(state: State<'_, AppState>, doc: Document) -> R<Document> {
    with_db(&state, |db| db.save_document(&doc))
}

#[tauri::command]
pub fn delete_document(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.delete_document(id))
}

#[tauri::command]
pub fn mark_issued(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.mark_issued(id))
}

// ---------- 收款 ----------

#[tauri::command]
pub fn add_payment(state: State<'_, AppState>, payment: Payment) -> R<i64> {
    with_db(&state, |db| db.add_payment(&payment))
}

#[tauri::command]
pub fn list_payments(state: State<'_, AppState>, customer_id: i64) -> R<Vec<Payment>> {
    with_db(&state, |db| db.list_payments(customer_id, None, None))
}

#[tauri::command]
pub fn delete_payment(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.delete_payment(id))
}

// ---------- 對帳單 ----------

#[tauri::command]
pub fn statement_period(state: State<'_, AppState>, customer_id: i64, year: i32, month: u32) -> R<(String, String)> {
    with_db(&state, |db| db.statement_period(customer_id, year, month))
}

#[tauri::command]
pub fn preview_statement(state: State<'_, AppState>, customer_id: i64, start: String, end: String) -> R<StatementPreview> {
    with_db(&state, |db| db.preview_statement(customer_id, &start, &end))
}

#[tauri::command]
pub fn create_statement(
    state: State<'_, AppState>,
    customer_id: i64,
    start: String,
    end: String,
    detail_mode: String,
    maker: String,
) -> R<Statement> {
    with_db(&state, |db| db.create_statement(customer_id, &start, &end, &detail_mode, &maker))
}

#[tauri::command]
pub fn list_statements(state: State<'_, AppState>, customer_id: Option<i64>) -> R<Vec<Statement>> {
    with_db(&state, |db| db.list_statements(customer_id))
}

#[tauri::command]
pub fn void_statement(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.void_statement(id))
}

// ---------- 手開對帳單 ----------

#[tauri::command]
pub fn list_manual(state: State<'_, AppState>, customer_id: Option<i64>, period: Option<String>, limit: Option<i64>) -> R<Vec<ManualSummary>> {
    with_db(&state, |db| db.list_manual(customer_id, period.as_deref(), limit))
}

#[tauri::command]
pub fn get_manual(state: State<'_, AppState>, id: i64) -> R<ManualStatement> {
    with_db(&state, |db| db.get_manual(id))
}

#[tauri::command]
pub fn save_manual(state: State<'_, AppState>, doc: ManualStatement) -> R<ManualStatement> {
    with_db(&state, |db| db.save_manual(&doc))
}

#[tauri::command]
pub fn delete_manual(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.delete_manual(id))
}

#[tauri::command]
pub fn mark_manual_issued(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.mark_manual_issued(id))
}

fn render_man(db: &Db, id: i64) -> anyhow::Result<(RenderedDoc, String)> {
    let settings = db.get_settings()?;
    let m = db.get_manual(id)?;
    Ok((print::render_manual(&m, &settings)?, m.stmt_no))
}

#[tauri::command]
pub fn render_manual(state: State<'_, AppState>, id: i64) -> R<RenderedDoc> {
    with_db(&state, |db| render_man(db, id).map(|r| r.0))
}

#[tauri::command]
pub fn print_manual(state: State<'_, AppState>, id: i64) -> R<()> {
    let (rendered, no) = with_db(&state, |db| render_man(db, id))?;
    send_raw(&rendered, &no)?;
    with_db(&state, |db| db.mark_manual_issued(id))
}

// ---------- 版面與列印 ----------

fn render_doc(db: &Db, id: i64) -> anyhow::Result<(RenderedDoc, String)> {
    let settings = db.get_settings()?;
    let doc = db.get_document(id)?;
    Ok((print::render_document(&doc, &settings)?, doc.doc_no))
}

fn render_stmt(db: &Db, id: i64) -> anyhow::Result<(RenderedDoc, String)> {
    let settings = db.get_settings()?;
    let st = db.get_statement(id)?;
    let docs = db.statement_documents(id)?;
    let cust = db.get_customer(st.customer_id)?;
    Ok((print::render_statement(&st, &docs, &cust.phone, &cust.address, &settings)?, st.stmt_no))
}

#[tauri::command]
pub fn render_document(state: State<'_, AppState>, id: i64) -> R<RenderedDoc> {
    with_db(&state, |db| render_doc(db, id).map(|r| r.0))
}

#[tauri::command]
pub fn render_statement(state: State<'_, AppState>, id: i64) -> R<RenderedDoc> {
    with_db(&state, |db| render_stmt(db, id).map(|r| r.0))
}

#[tauri::command]
pub fn render_test_page(state: State<'_, AppState>) -> R<RenderedDoc> {
    with_db(&state, |db| Ok(print::render_test_page(&db.get_settings()?)))
}

fn send_raw(rendered: &RenderedDoc, job: &str) -> R<()> {
    let cfg = &rendered.config;
    if cfg.mode != "escp" {
        return Err("目前設定為驅動程式列印，請用預覽視窗的列印按鈕".into());
    }
    let name = if cfg.printer_name.trim().is_empty() {
        printer::default_printer().ok_or("請先到設定選擇印表機")?
    } else {
        cfg.printer_name.clone()
    };
    printer::print_raw(&name, job, &print::render_escp(rendered))
}

#[tauri::command]
pub fn print_document(state: State<'_, AppState>, id: i64) -> R<()> {
    let (rendered, no) = with_db(&state, |db| render_doc(db, id))?;
    send_raw(&rendered, &no)?;
    with_db(&state, |db| db.mark_issued(id))
}

#[tauri::command]
pub fn print_statement(state: State<'_, AppState>, id: i64) -> R<()> {
    let (rendered, no) = with_db(&state, |db| render_stmt(db, id))?;
    send_raw(&rendered, &no)
}

#[tauri::command]
pub fn print_test_page(state: State<'_, AppState>) -> R<()> {
    let rendered = with_db(&state, |db| Ok(print::render_test_page(&db.get_settings()?)))?;
    send_raw(&rendered, "對位測試頁")
}

// ---------- 備份 ----------

#[tauri::command]
pub fn backup_now(state: State<'_, AppState>) -> R<String> {
    let dir = state.data_dir.clone();
    with_db(&state, |db| storage::backup_now(db, &dir).map(|p| p.display().to_string()))
}


// ---------- 手機同步 ----------

#[derive(Serialize)]
pub struct SyncAddress {
    ip: String,
    /// "Tailscale" 或 "區域網路"
    kind: String,
}

#[derive(Serialize)]
pub struct SyncStatus {
    enabled: bool,
    running: bool,
    port: u16,
    error: Option<String>,
    addresses: Vec<SyncAddress>,
}

fn local_addresses() -> Vec<SyncAddress> {
    let mut out: Vec<SyncAddress> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(v4) => {
                let o = v4.octets();
                // Tailscale 的位址在 100.64.0.0/10
                let tailscale = o[0] == 100 && (64..128).contains(&o[1]);
                if v4.is_link_local() {
                    None
                } else {
                    Some(SyncAddress { ip: v4.to_string(), kind: if tailscale { "Tailscale".into() } else { "區域網路".into() } })
                }
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| (a.kind != "Tailscale").cmp(&(b.kind != "Tailscale")).then(a.ip.cmp(&b.ip)));
    out
}

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> R<SyncStatus> {
    let settings = with_db(&state, |db| db.get_settings())?;
    let running = state.sync_server.lock().map(|s| s.is_some()).unwrap_or(false);
    Ok(SyncStatus {
        enabled: settings.sync_enabled,
        running,
        port: settings.sync_port,
        error: state.sync_error.lock().ok().and_then(|e| e.clone()),
        addresses: local_addresses(),
    })
}

#[tauri::command]
pub fn restart_sync_server(state: State<'_, AppState>) -> R<SyncStatus> {
    start_sync_server(&state);
    sync_status(state)
}

#[tauri::command]
pub fn create_pairing_code(state: State<'_, AppState>) -> R<PairingCode> {
    with_db(&state, |db| db.create_pairing_code())
}

#[tauri::command]
pub fn list_devices(state: State<'_, AppState>) -> R<Vec<Device>> {
    with_db(&state, |db| db.list_devices())
}

#[tauri::command]
pub fn revoke_device(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.revoke_device(id))
}

#[tauri::command]
pub fn rename_device(state: State<'_, AppState>, id: i64, name: String) -> R<()> {
    with_db(&state, |db| db.rename_device(id, &name))
}

#[tauri::command]
pub fn list_sync_conflicts(state: State<'_, AppState>) -> R<Vec<SyncConflict>> {
    with_db(&state, |db| db.list_sync_conflicts(false))
}

#[tauri::command]
pub fn resolve_sync_conflict(state: State<'_, AppState>, id: i64, action: String) -> R<()> {
    with_db(&state, |db| db.resolve_sync_conflict(id, &action))
}

#[tauri::command]
pub fn possible_duplicates(state: State<'_, AppState>, customer_id: i64) -> R<Vec<(i64, String)>> {
    with_db(&state, |db| db.possible_duplicates(customer_id))
}

#[tauri::command]
pub fn confirm_customer(state: State<'_, AppState>, id: i64) -> R<()> {
    with_db(&state, |db| db.confirm_customer(id))
}

#[tauri::command]
pub fn merge_customers(state: State<'_, AppState>, from_id: i64, to_id: i64) -> R<()> {
    with_db(&state, |db| db.merge_customers(from_id, to_id))
}
