use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Customer {
    pub id: Option<i64>,
    pub code: String,
    pub name: String,
    pub tax_id: String,
    pub phone: String,
    pub contact: String,
    pub address: String,
    /// 0 = 月底結帳；1..=28 = 每月該日結帳
    pub closing_day: i64,
    /// "exclusive"（外加 5%）、"inclusive"（內含）、"none"（免稅）
    pub tax_mode: String,
    pub note: String,
    pub active: bool,
    /// 跨裝置不變的編號（同步用）
    pub uid: String,
    /// 哪一台建立的：空白＝電腦，"V1"＝第一台手機
    pub origin: String,
    /// 手機新增、電腦還沒確認過
    pub needs_review: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Product {
    pub id: Option<i64>,
    pub code: String,
    pub name: String,
    pub unit: String,
    /// None = 時價（沒有預設價）
    pub price: Option<f64>,
    /// 秤重品項：數量可輸入斤兩
    pub weighed: bool,
    pub active: bool,
    pub uid: String,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CustomerPrice {
    pub customer_id: i64,
    pub customer_name: String,
    pub product_id: i64,
    pub price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PriceSuggestion {
    pub price: Option<f64>,
    /// "customer"（客戶專屬價）、"last"（上次成交）、"default"（預設價）、"none"
    pub source: String,
    pub last_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DocLine {
    pub id: Option<i64>,
    pub product_id: Option<i64>,
    pub name: String,
    pub unit: String,
    /// 使用者輸入的原始數量文字，例如「2斤5兩」
    pub qty_text: String,
    pub qty: f64,
    pub price: f64,
    pub amount: i64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Document {
    pub id: Option<i64>,
    pub doc_no: String,
    /// "shipment"（出貨單）或 "adjustment"（退貨／折讓）
    pub kind: String,
    pub customer_id: i64,
    pub doc_date: String,
    pub maker: String,
    pub note: String,
    /// "draft" 或 "issued"
    pub status: String,
    pub statement_id: Option<i64>,
    pub statement_no: Option<String>,
    pub cust_name: String,
    pub cust_phone: String,
    pub cust_address: String,
    pub tax_mode: String,
    pub subtotal: i64,
    pub tax: i64,
    pub total: i64,
    pub uid: String,
    /// 哪一台開的：空白＝電腦，"V1"＝第一台手機
    pub origin: String,
    /// 手機列印給客戶（出貨）的時間；空白＝還沒出貨
    pub shipped_at: String,
    pub lines: Vec<DocLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DocSummary {
    pub id: i64,
    pub doc_no: String,
    pub kind: String,
    pub customer_id: i64,
    pub cust_name: String,
    pub doc_date: String,
    pub status: String,
    pub statement_id: Option<i64>,
    pub item_count: i64,
    pub total: i64,
    /// 哪一台開的：空白＝電腦
    pub origin: String,
    pub shipped_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DocFilter {
    pub customer_id: Option<i64>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Payment {
    pub id: Option<i64>,
    pub customer_id: i64,
    pub pay_date: String,
    pub amount: i64,
    pub method: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatementPreview {
    pub customer_id: i64,
    pub cust_name: String,
    pub period_start: String,
    pub period_end: String,
    pub prev_balance: i64,
    pub docs: Vec<DocSummary>,
    pub period_amount: i64,
    pub payments: Vec<Payment>,
    pub paid_in_period: i64,
    pub total_due: i64,
    /// 期間內還是草稿、不會列入的出貨單張數
    pub drafts_excluded: i64,
    /// 期間內已經被其他對帳單鎖定的張數
    pub already_locked: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Statement {
    pub id: i64,
    pub stmt_no: String,
    pub customer_id: i64,
    pub cust_name: String,
    pub period_start: String,
    pub period_end: String,
    pub prev_balance: i64,
    pub period_amount: i64,
    pub paid_in_period: i64,
    pub total_due: i64,
    /// "summary"（每張出貨單一列）或 "detail"（展開品項）
    pub detail_mode: String,
    pub maker: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CustomerBalance {
    pub customer_id: i64,
    pub balance: i64,
}

/// 手開對帳單的一列：日期＋金額＋備註
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ManualLine {
    pub id: Option<i64>,
    /// YYYY-MM-DD
    pub line_date: String,
    pub amount: i64,
    pub note: String,
}

/// 手開對帳單：獨立作業，不連品項、不算進客戶未收金額
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ManualStatement {
    pub id: Option<i64>,
    pub stmt_no: String,
    pub customer_id: i64,
    /// 對帳月份 YYYY-MM，印成「10月份對帳單」
    pub period: String,
    pub doc_date: String,
    pub maker: String,
    pub note: String,
    /// "draft" 或 "issued"
    pub status: String,
    /// "single"（單欄）或 "double"（左右兩欄）
    pub layout: String,
    pub cust_name: String,
    pub cust_phone: String,
    pub cust_address: String,
    pub tax_mode: String,
    pub subtotal: i64,
    pub tax: i64,
    pub total: i64,
    pub lines: Vec<ManualLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ManualSummary {
    pub id: i64,
    pub stmt_no: String,
    pub customer_id: i64,
    pub cust_name: String,
    pub period: String,
    pub doc_date: String,
    pub status: String,
    pub line_count: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintConfig {
    /// "escp"（文字模式直送印表機）或 "driver"（經 Windows 驅動的圖形列印）
    pub mode: String,
    pub printer_name: String,
    /// 每頁總行數（含上下留白），9½×11 紙以 1/6 吋行距為 66 行
    pub page_lines: usize,
    /// 行距，單位 1/180 吋（30 = 1/6 吋）
    pub line_spacing_180: u8,
    pub top_lines: usize,
    pub bottom_lines: usize,
    /// 左邊界（字元數）
    pub left_margin_cols: usize,
    /// 每行可用寬度（半形字元數，中文算 2）
    pub total_cols: usize,
    /// 10 / 12 / 15
    pub cpi: u8,
    /// 是否送出 FS & 進入中文模式
    pub chinese_cmd: bool,
    /// 紙張方式：
    /// "half"＝一律用半張（中一刀），筆數多就接續印下一張半張；
    /// "auto"＝放得下半張就印半張，否則印整張；
    /// "full"＝一律整張。半張行數 = 每頁行數 ÷ 2
    pub paper_mode: String,
    /// 驅動程式模式用的半張紙尺寸（公分），要和驅動程式裡的自訂紙張一致
    pub driver_paper_w_cm: f64,
    pub driver_paper_h_cm: f64,
    /// 連續報表：續頁直接接著印明細，不重印欄位名稱，也不印每頁頁碼
    pub continuous: bool,
}

impl Default for PrintConfig {
    fn default() -> Self {
        PrintConfig {
            mode: "escp".into(),
            printer_name: String::new(),
            page_lines: 66,
            line_spacing_180: 30,
            top_lines: 2,
            bottom_lines: 2,
            left_margin_cols: 2,
            total_cols: 79,
            cpi: 10,
            chinese_cmd: true,
            paper_mode: "half".into(),
            driver_paper_w_cm: 21.0,
            driver_paper_h_cm: 13.97, // 中一刀：5.5 吋
            continuous: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub company_name: String,
    pub default_maker: String,
    pub signatures: Vec<String>,
    pub extra_backup_dir: String,
    /// 新開手開對帳單時預設的列印版面："single" 或 "double"
    pub manual_layout: String,
    /// 手機同步服務
    pub sync_enabled: bool,
    pub sync_port: u16,
    pub print: PrintConfig,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            company_name: "泓昇海鮮".into(),
            default_maker: String::new(),
            signatures: vec!["會計".into(), "倉管".into(), "經辦".into(), "簽章".into()],
            extra_backup_dir: String::new(),
            manual_layout: "double".into(),
            sync_enabled: true,
            sync_port: 47110,
            print: PrintConfig::default(),
        }
    }
}
