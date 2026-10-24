//! 列印版面：把單據排成「一頁一頁、一行一行」的文字，
//! 同一份結果同時給 ESC/P 文字列印與畫面預覽／驅動程式列印使用，
//! 所以預覽看到的分頁就是實際列印的分頁。

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::models::{AppSettings, Document, ManualStatement, PrintConfig, Statement};
use crate::util::{center, fmt_int, fmt_num, pad_left, pad_right, str_width};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Line {
    pub text: String,
    pub bold: bool,
}

impl Line {
    fn plain(t: impl Into<String>) -> Line {
        Line { text: t.into(), bold: false }
    }
    fn bold(t: impl Into<String>) -> Line {
        Line { text: t.into(), bold: true }
    }
    fn blank() -> Line {
        Line::plain("")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedDoc {
    pub pages: Vec<Vec<Line>>,
    pub config: PrintConfig,
    /// 這份單據實際使用的每頁行數（整張＝設定的每頁行數，半張＝一半）
    #[serde(default)]
    pub page_lines: usize,
    #[serde(default)]
    pub half: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Right,
}

struct Col {
    title: &'static str,
    width: usize, // 0 = 吃掉剩下的寬度
    align: Align,
}

/// 一份要列印的單據（出貨單或對帳單）的抽象內容。
struct PrintDoc {
    title: String,
    subtitle: String,
    /// 客戶資料區，兩欄並排：(左, 右)
    fields: Vec<(String, String)>,
    cols: Vec<Col>,
    rows: Vec<Vec<String>>,
    totals: Vec<(String, String)>,
    /// 合計數字對齊到哪一欄（欄位索引）
    totals_col: usize,
    signatures: Vec<String>,
    footer_id: String,
    /// 左右兩欄：明細分成左右兩半並排（左欄先排完再排右欄），中間用「 | 」分隔
    pair: bool,
}

/// 兩欄中間的分隔
const PAIR_GAP: &str = " | ";

fn resolve_widths(cols: &[Col], total: usize) -> Result<Vec<usize>> {
    let gaps = cols.len().saturating_sub(1);
    let fixed: usize = cols.iter().map(|c| c.width).sum::<usize>() + gaps;
    if fixed + 8 > total {
        bail!("每行寬度 {} 太窄，放不下所有欄位（至少需要 {}）", total, fixed + 8);
    }
    let flex = total - fixed;
    Ok(cols.iter().map(|c| if c.width == 0 { flex } else { c.width }).collect())
}

fn row_text(cells: &[String], cols: &[Col], widths: &[usize]) -> String {
    let parts: Vec<String> = cells
        .iter()
        .zip(cols.iter().zip(widths))
        .map(|(v, (c, w))| match c.align {
            Align::Left => pad_right(v, *w),
            Align::Right => pad_left(v, *w),
        })
        .collect();
    parts.join(" ").trim_end().to_string()
}

/// 依紙張方式排版：
/// half：每一頁都是半張，多的接續下一張半張；
/// auto：只有一頁放得下半張時印半張，否則整張；
/// full：一律整張。
fn layout(doc: &PrintDoc, cfg: &PrintConfig) -> Result<(Vec<Vec<Line>>, usize, bool)> {
    let half = cfg.page_lines / 2;
    match cfg.paper_mode.as_str() {
        "full" => Ok((paginate(doc, cfg, cfg.page_lines)?, cfg.page_lines, false)),
        "auto" => {
            if let Ok(pages) = paginate(doc, cfg, half) {
                if pages.len() == 1 {
                    return Ok((pages, half, true));
                }
            }
            Ok((paginate(doc, cfg, cfg.page_lines)?, cfg.page_lines, false))
        }
        _ => Ok((paginate(doc, cfg, half)?, half, true)),
    }
}

fn rendered(pd: &PrintDoc, cfg: &PrintConfig) -> Result<RenderedDoc> {
    let (pages, page_lines, half) = layout(pd, cfg)?;
    Ok(RenderedDoc { pages, config: cfg.clone(), page_lines, half })
}

fn paginate(doc: &PrintDoc, cfg: &PrintConfig, page_lines: usize) -> Result<Vec<Vec<Line>>> {
    let w = cfg.total_cols;
    // 兩欄時每一欄的寬度
    let col_w = if doc.pair { (w - PAIR_GAP.len()) / 2 } else { w };
    let widths = resolve_widths(&doc.cols, col_w)?;
    let usable = page_lines.saturating_sub(cfg.top_lines + cfg.bottom_lines);
    let pair_text = |left: &str, right: Option<&str>| -> String {
        match right {
            Some(r) => format!("{}{}{}", pad_right(left, col_w), PAIR_GAP, r).trim_end().to_string(),
            None => left.trim_end().to_string(),
        }
    };

    let header_titles: Vec<String> = doc.cols.iter().map(|c| c.title.to_string()).collect();
    let one_header = row_text(&header_titles, &doc.cols, &widths);
    let header_text = if doc.pair { pair_text(&one_header, Some(&one_header)) } else { one_header };
    let col_header = vec![Line::bold(header_text), Line::plain("-".repeat(w))];

    let mut first_header = vec![Line::bold(center(&doc.title, w)), Line::plain(center(&doc.subtitle, w)), Line::blank()];
    let half = (w - 2) / 2;
    for (l, r) in &doc.fields {
        first_header.push(Line::plain(format!("{}  {}", pad_right(l, half), r).trim_end().to_string()));
    }
    first_header.push(Line::blank());
    first_header.extend(col_header.clone());

    // 合計區：橫線、各項合計、空行、簽名列
    // 合計數字對齊明細的金額欄；兩欄時對齊右欄的金額
    let amount_end: usize = widths[..=doc.totals_col].iter().sum::<usize>()
        + doc.totals_col
        + if doc.pair { col_w + PAIR_GAP.len() } else { 0 };
    let value_w = widths[doc.totals_col];
    // 標籤（例如「未稅金額：」）放不下時，數字整組往右移，不截斷標籤
    let label_w = doc.totals.iter().map(|(l, _)| str_width(l)).max().unwrap_or(0);
    let amount_end = amount_end.max(label_w + 1 + value_w).min(w);
    let mut totals_block = vec![Line::plain("-".repeat(w))];
    for (label, value) in &doc.totals {
        let lw = amount_end.saturating_sub(value_w + 1);
        totals_block.push(Line::plain(format!("{} {}", pad_left(label, lw), pad_left(value, value_w))));
    }
    totals_block.push(Line::blank());
    if !doc.signatures.is_empty() {
        let seg = w / doc.signatures.len();
        let sig: String = doc
            .signatures
            .iter()
            .map(|s| {
                let label = format!("{}：", s);
                let fill = seg.saturating_sub(str_width(&label) + 2);
                pad_right(&format!("{}{}", label, "_".repeat(fill)), seg)
            })
            .collect();
        totals_block.push(Line::plain(sig.trim_end().to_string()));
    }

    // 連續報表：續頁不重印欄位名稱、不印頁碼，明細直接接下去
    let next_header: Vec<Line> = if cfg.continuous { vec![] } else { col_header.clone() };
    let footer_lines = if cfg.continuous { 0usize } else { 1 };
    let cap_first = usable as i64 - first_header.len() as i64 - footer_lines as i64;
    let cap_next = usable as i64 - next_header.len() as i64 - footer_lines as i64;
    let t = totals_block.len() as i64;
    if cap_first < t || cap_next < t + 1 {
        bail!("每頁可用行數（{}）太少，放不下表頭與合計區，請在設定調整頁長或上下留白", usable);
    }

    // 分頁：決定每一頁放哪些筆，以及合計放在哪一頁。兩欄時一行放兩筆。
    let pair = doc.pair;
    let lines_for = |m: usize| if pair { (m + 1) / 2 } else { m };
    let items_for = |cap: usize| if pair { cap * 2 } else { cap };
    let n = doc.rows.len();
    let mut plan: Vec<(usize, usize, bool)> = vec![]; // (start, end, has_totals)
    let mut i = 0usize;
    loop {
        let cap_lines = if plan.is_empty() { cap_first } else { cap_next } as usize;
        let remaining = n - i;
        if lines_for(remaining) + t as usize <= cap_lines {
            plan.push((i, n, true));
            break;
        }
        let cap = items_for(cap_lines);
        if remaining <= cap {
            // 明細放得下但合計放不下：留至少一列到下一頁，合計不單獨成頁
            let take = if remaining > 1 { remaining - 1 } else { remaining };
            plan.push((i, i + take, false));
            i += take;
            if i == n {
                plan.push((n, n, true));
                break;
            }
            continue;
        }
        plan.push((i, i + cap, false));
        i += cap;
    }

    let total_pages = plan.len();
    let mut pages = vec![];
    for (pi, (s, e, has_totals)) in plan.iter().enumerate() {
        let mut lines: Vec<Line> = if pi == 0 { first_header.clone() } else { next_header.clone() };
        let texts: Vec<String> = doc.rows[*s..*e].iter().map(|r| row_text(r, &doc.cols, &widths)).collect();
        if pair {
            // 這一頁的筆數左欄先排一半，右欄排另一半
            let k = (texts.len() + 1) / 2;
            for j in 0..k {
                let right = texts.get(j + k).map(|t| t.as_str());
                // 這頁只有一筆時不印分隔線；其餘列保持分隔線連續
                let right = if k > 1 { Some(right.unwrap_or("")) } else { right };
                lines.push(Line::plain(pair_text(&texts[j], right)));
            }
        } else {
            for t in texts {
                lines.push(Line::plain(t));
            }
        }
        if *has_totals {
            lines.extend(totals_block.clone());
        }
        while lines.len() < usable - footer_lines {
            lines.push(Line::blank());
        }
        if cfg.continuous {
            pages.push(lines);
            continue;
        }
        let right = format!(
            "第 {} / {} 頁{}",
            pi + 1,
            total_pages,
            if pi + 1 == total_pages { "" } else { "（接下頁）" }
        );
        let left_w = w.saturating_sub(str_width(&right) + 1);
        lines.push(Line::plain(format!("{} {}", pad_right(&doc.footer_id, left_w), right)));
        pages.push(lines);
    }
    Ok(pages)
}

fn tax_label(mode: &str) -> &'static str {
    match mode {
        "inclusive" => "稅金（內含）：",
        "none" => "稅金（免稅）：",
        _ => "稅金金額：",
    }
}

fn date_mmdd(d: &str) -> String {
    if d.len() >= 10 {
        format!("{}/{}", &d[5..7], &d[8..10])
    } else {
        d.to_string()
    }
}

fn date_slash(d: &str) -> String {
    d.replace('-', "/")
}

pub fn render_document(doc: &Document, settings: &AppSettings) -> Result<RenderedDoc> {
    let cfg = &settings.print;
    let subtitle = if doc.kind == "adjustment" { "退貨／折讓單" } else { "出貨單" };
    let rows = doc
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            vec![
                (i + 1).to_string(),
                l.name.clone(),
                if l.qty_text.trim().is_empty() { fmt_num(l.qty) } else { l.qty_text.clone() },
                l.unit.clone(),
                fmt_num(l.price),
                fmt_int(l.amount),
                l.note.clone(),
            ]
        })
        .collect();
    let pd = PrintDoc {
        title: settings.company_name.clone(),
        subtitle: subtitle.into(),
        fields: vec![
            (format!("公司名稱：{}", doc.cust_name), format!("單據日期：{}", date_slash(&doc.doc_date))),
            (format!("電　　話：{}", doc.cust_phone), format!("單據編號：{}", doc.doc_no)),
            (format!("聯絡地址：{}", doc.cust_address), format!("製表人員：{}", doc.maker)),
        ],
        cols: vec![
            Col { title: "#", width: 3, align: Align::Right },
            Col { title: "品名", width: 0, align: Align::Left },
            Col { title: "數量", width: 8, align: Align::Right },
            Col { title: "單位", width: 4, align: Align::Left },
            Col { title: "單價", width: 8, align: Align::Right },
            Col { title: "金額", width: 9, align: Align::Right },
            Col { title: "訂貨備註", width: 12, align: Align::Left },
        ],
        rows,
        totals: vec![
            ("未稅金額：".into(), fmt_int(doc.subtotal)),
            (tax_label(&doc.tax_mode).into(), fmt_int(doc.tax)),
            ("合計金額：".into(), fmt_int(doc.total)),
        ],
        totals_col: 5,
        signatures: settings.signatures.clone(),
        footer_id: format!("{}　{}", doc.doc_no, doc.cust_name),
        pair: false,
    };
    rendered(&pd, cfg)
}

pub fn render_statement(
    st: &Statement,
    docs: &[Document],
    phone: &str,
    address: &str,
    settings: &AppSettings,
) -> Result<RenderedDoc> {
    let cfg = &settings.print;
    let month: u32 = st.period_end.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(0);
    let (cols, rows) = if st.detail_mode == "detail" {
        let cols = vec![
            Col { title: "日期", width: 5, align: Align::Left },
            Col { title: "品名", width: 0, align: Align::Left },
            Col { title: "數量", width: 8, align: Align::Right },
            Col { title: "單位", width: 4, align: Align::Left },
            Col { title: "單價", width: 8, align: Align::Right },
            Col { title: "金額", width: 10, align: Align::Right },
            Col { title: "單號", width: 12, align: Align::Left },
        ];
        let mut rows = vec![];
        for d in docs {
            let sign = if d.kind == "adjustment" { -1 } else { 1 };
            for (i, l) in d.lines.iter().enumerate() {
                let name = if d.kind == "adjustment" { format!("（退）{}", l.name) } else { l.name.clone() };
                rows.push(vec![
                    if i == 0 { date_mmdd(&d.doc_date) } else { String::new() },
                    name,
                    if l.qty_text.trim().is_empty() { fmt_num(l.qty) } else { l.qty_text.clone() },
                    l.unit.clone(),
                    fmt_num(l.price),
                    fmt_int(l.amount * sign),
                    if i == 0 { d.doc_no.clone() } else { String::new() },
                ]);
            }
            if d.tax != 0 {
                rows.push(vec![String::new(), "　稅金".into(), String::new(), String::new(), String::new(), fmt_int(d.tax), String::new()]);
            }
        }
        (cols, rows)
    } else {
        let cols = vec![
            Col { title: "日期", width: 5, align: Align::Left },
            Col { title: "單號", width: 13, align: Align::Left },
            Col { title: "說明", width: 0, align: Align::Left },
            Col { title: "品項", width: 4, align: Align::Right },
            Col { title: "未稅", width: 10, align: Align::Right },
            Col { title: "稅金", width: 8, align: Align::Right },
            Col { title: "金額", width: 10, align: Align::Right },
        ];
        let rows = docs
            .iter()
            .map(|d| {
                vec![
                    date_mmdd(&d.doc_date),
                    d.doc_no.clone(),
                    if d.kind == "adjustment" { "退貨／折讓".into() } else { "出貨".into() },
                    d.lines.len().to_string(),
                    fmt_int(d.subtotal),
                    fmt_int(d.tax),
                    fmt_int(d.total),
                ]
            })
            .collect();
        (cols, rows)
    };
    let mut totals = vec![("前期未收：".to_string(), fmt_int(st.prev_balance)), ("本期金額：".to_string(), fmt_int(st.period_amount))];
    if st.paid_in_period != 0 {
        totals.push(("本期收款：".into(), fmt_int(-st.paid_in_period)));
    }
    totals.push(("本期應收：".into(), fmt_int(st.total_due)));
    let pd = PrintDoc {
        title: settings.company_name.clone(),
        subtitle: if month > 0 { format!("{}月份對帳單", month) } else { "對帳單".into() },
        fields: vec![
            (format!("公司名稱：{}", st.cust_name), format!("對帳期間：{} – {}", date_slash(&st.period_start), date_slash(&st.period_end))),
            (format!("電　　話：{}", phone), format!("單據編號：{}", st.stmt_no)),
            (format!("聯絡地址：{}", address), format!("製表人員：{}", st.maker)),
        ],
        cols,
        rows,
        totals,
        totals_col: 6,
        signatures: settings.signatures.clone(),
        footer_id: format!("{}　{}", st.stmt_no, st.cust_name),
        pair: false,
    };
    rendered(&pd, cfg)
}

/// 手開對帳單：明細只有日期、金額、備註，可選單欄或左右兩欄。
pub fn render_manual(m: &ManualStatement, settings: &AppSettings) -> Result<RenderedDoc> {
    let cfg = &settings.print;
    let month: u32 = m.period.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(0);
    let pair = m.layout != "single";
    let cols = if pair {
        vec![
            Col { title: "#", width: 3, align: Align::Right },
            Col { title: "日期", width: 5, align: Align::Left },
            Col { title: "金額", width: 11, align: Align::Right },
            Col { title: "備註", width: 0, align: Align::Left },
        ]
    } else {
        vec![
            Col { title: "#", width: 3, align: Align::Right },
            Col { title: "日期", width: 6, align: Align::Left },
            Col { title: "金額", width: 12, align: Align::Right },
            Col { title: "備註", width: 0, align: Align::Left },
        ]
    };
    let rows = m
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| vec![(i + 1).to_string(), date_mmdd(&l.line_date), fmt_int(l.amount), l.note.clone()])
        .collect();
    let pd = PrintDoc {
        title: settings.company_name.clone(),
        subtitle: if month > 0 { format!("{}月份對帳單", month) } else { "對帳單".into() },
        fields: vec![
            (format!("公司名稱：{}", m.cust_name), format!("單據日期：{}", date_slash(&m.doc_date))),
            (format!("電　　話：{}", m.cust_phone), format!("單據編號：{}", m.stmt_no)),
            (format!("聯絡地址：{}", m.cust_address), format!("製表人員：{}", m.maker)),
        ],
        cols,
        rows,
        totals: vec![
            ("未稅金額：".into(), fmt_int(m.subtotal)),
            (tax_label(&m.tax_mode).into(), fmt_int(m.tax)),
            ("合計金額：".into(), fmt_int(m.total)),
        ],
        totals_col: 2,
        signatures: settings.signatures.clone(),
        footer_id: format!("{}　{}", m.stmt_no, m.cust_name),
        pair,
    };
    rendered(&pd, cfg)
}

/// 對位測試頁：每行標行號、上方有欄位尺標，用來確認每頁行數與左右位置。
pub fn render_test_page(settings: &AppSettings) -> RenderedDoc {
    let cfg = &settings.print;
    let w = cfg.total_cols;
    let usable = cfg.page_lines.saturating_sub(cfg.top_lines + cfg.bottom_lines);
    let mut lines = vec![];
    let ruler: String = (0..w).map(|i| char::from_digit(((i + 1) % 10) as u32, 10).unwrap()).collect();
    let tens: String = (0..w)
        .map(|i| if (i + 1) % 10 == 0 { char::from_digit((((i + 1) / 10) % 10) as u32, 10).unwrap() } else { ' ' })
        .collect();
    for n in 1..=usable {
        let text = match n {
            1 => tens.clone(),
            2 => ruler.clone(),
            3 => format!("{:02} 中文測試：{}　全形（）數字 1234567890", n, settings.company_name),
            4 => format!("{:02} 每頁行數 {}，上留白 {}，下留白 {}，每行 {} 字", n, cfg.page_lines, cfg.top_lines, cfg.bottom_lines, w),
            _ if n == usable => format!("{:02} ← 最後一行，下一頁應從撕線後第 {} 行開始", n, cfg.top_lines + 1),
            _ => format!("{:02}", n),
        };
        lines.push(Line { text, bold: n == 3 });
    }
    RenderedDoc { pages: vec![lines.clone(), lines], config: cfg.clone(), page_lines: cfg.page_lines, half: false }
}

// ---------------- ESC/P 文字模式 ----------------

const ESC: u8 = 0x1B;
const FS: u8 = 0x1C;

/// 標準 Big5（不含香港增補字集）的編碼。點陣印表機的中文字庫通常只有標準 Big5。
fn big5_char(c: char) -> Option<[u8; 2]> {
    let mut buf = [0u8; 4];
    let (bytes, _, had_errors) = encoding_rs::BIG5.encode(c.encode_utf8(&mut buf));
    if had_errors || bytes.len() != 2 {
        return None;
    }
    let code = u16::from_be_bytes([bytes[0], bytes[1]]);
    let standard = (0xA140..=0xA3BF).contains(&code)
        || (0xA440..=0xC67E).contains(&code)
        || (0xC940..=0xF9D5).contains(&code);
    if standard {
        Some([bytes[0], bytes[1]])
    } else {
        None
    }
}

fn encode_big5(s: &str, out: &mut Vec<u8>) {
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c as u8);
            continue;
        }
        match big5_char(c) {
            Some(b) => out.extend_from_slice(&b),
            // 印表機沒有這個字：用全形問號佔同樣寬度
            None => out.extend_from_slice(&[0xA1, 0x48]),
        }
    }
}

/// 找出 Big5 印不出來的字，用來在輸入品名、客戶名時提醒。
pub fn non_big5_chars(s: &str) -> Vec<char> {
    let mut out = vec![];
    for c in s.chars() {
        if !c.is_ascii() && big5_char(c).is_none() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

pub fn render_escp(doc: &RenderedDoc) -> Vec<u8> {
    let cfg = &doc.config;
    let mut out = vec![ESC, b'@'];
    out.extend_from_slice(&[ESC, b'3', cfg.line_spacing_180]);
    let page_lines = if doc.page_lines > 0 { doc.page_lines } else { cfg.page_lines };
    out.extend_from_slice(&[ESC, b'C', page_lines.clamp(1, 127) as u8]);
    match cfg.cpi {
        12 => out.extend_from_slice(&[ESC, b'M']),
        15 => out.extend_from_slice(&[ESC, b'g']),
        _ => out.extend_from_slice(&[ESC, b'P']),
    }
    out.extend_from_slice(&[ESC, b'l', cfg.left_margin_cols.min(255) as u8]);
    if cfg.chinese_cmd {
        out.extend_from_slice(&[FS, b'&']);
    }
    for page in &doc.pages {
        for _ in 0..cfg.top_lines {
            out.extend_from_slice(b"\r\n");
        }
        for line in page {
            if line.bold {
                out.extend_from_slice(&[ESC, b'E']);
            }
            encode_big5(&line.text, &mut out);
            if line.bold {
                out.extend_from_slice(&[ESC, b'F']);
            }
            out.extend_from_slice(b"\r\n");
        }
        out.push(0x0C); // 換頁：送到下一張的頁首
    }
    if cfg.chinese_cmd {
        out.extend_from_slice(&[FS, b'.']);
    }
    out.extend_from_slice(&[ESC, b'@']);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DocLine, Document};

    fn doc_with(n: usize) -> Document {
        Document {
            doc_no: "S202601-0012".into(),
            cust_name: "時光樹影吧檯".into(),
            doc_date: "2026-01-15".into(),
            tax_mode: "exclusive".into(),
            lines: (0..n)
                .map(|i| DocLine { name: format!("白蝦（大）{}", i), unit: "斤".into(), qty_text: "2斤5兩".into(), qty: 2.3125, price: 380.0, amount: 879, ..Default::default() })
                .collect(),
            ..Default::default()
        }
    }

    fn check(doc: &RenderedDoc) {
        let cfg = &doc.config;
        let usable = doc.page_lines - cfg.top_lines - cfg.bottom_lines;
        for p in &doc.pages {
            assert_eq!(p.len(), usable);
            for l in p {
                assert!(str_width(&l.text) <= cfg.total_cols, "too wide: {}", l.text);
            }
        }
    }

    fn count_rows(doc: &RenderedDoc) -> usize {
        doc.pages.iter().flatten().filter(|l| l.text.contains("白蝦")).count()
    }

    #[test]
    fn paginate_various() {
        for (mode, cont) in [("half", true), ("half", false), ("auto", true), ("auto", false), ("full", true), ("full", false)] {
        let mut s = with_mode(mode);
        s.print.continuous = cont;
        for n in [0, 1, 10, 13, 14, 18, 19, 20, 25, 26, 40, 45, 46, 47, 48, 49, 50, 51, 52, 53, 100, 200] {
            let r = render_document(&doc_with(n), &s).unwrap();
            check(&r);
            assert_eq!(count_rows(&r), n, "n={}", n);
            let last = r.pages.last().unwrap();
            assert!(last.iter().any(|l| l.text.contains("合計金額")), "n={}", n);
            let totals_pages = r.pages.iter().filter(|p| p.iter().any(|l| l.text.contains("合計金額"))).count();
            assert_eq!(totals_pages, 1);
            // 只有第一頁有客戶資料
            let header_pages = r.pages.iter().filter(|p| p.iter().any(|l| l.text.contains("公司名稱"))).count();
            assert_eq!(header_pages, 1);
            // 合計頁一定有明細（除非整張單沒有品項）
            if n > 0 {
                assert!(last.iter().any(|l| l.text.contains("白蝦")), "n={}", n);
            }
        }
        }
    }

    fn with_mode(mode: &str) -> AppSettings {
        let mut s = AppSettings::default();
        s.print.paper_mode = mode.into();
        s
    }

    #[test]
    fn continuous_form() {
        // 預設是連續報表：整份單據只有一次欄位名稱，沒有頁碼
        let s = AppSettings::default();
        assert!(s.print.continuous);
        let r = render_document(&doc_with(40), &s).unwrap();
        assert!(r.pages.len() >= 2);
        check(&r);
        let all: Vec<&Line> = r.pages.iter().flatten().collect();
        assert_eq!(all.iter().filter(|l| l.text.contains("品名")).count(), 1);
        assert!(!all.iter().any(|l| l.text.contains("接下頁")));
        // 第二張第一行就是明細
        assert!(r.pages[1][0].text.contains("白蝦"));
        assert_eq!(count_rows(&r), 40);
        // 第一張：表頭 9 行，其餘 20 行都是明細
        assert_eq!(r.pages[0].iter().filter(|l| l.text.contains("白蝦")).count(), 20);

        // 關閉連續報表：每頁重印欄位名稱並有頁碼
        let mut off = AppSettings::default();
        off.print.continuous = false;
        let r = render_document(&doc_with(40), &off).unwrap();
        let all: Vec<&Line> = r.pages.iter().flatten().collect();
        assert_eq!(all.iter().filter(|l| l.text.contains("品名")).count(), r.pages.len());
        assert!(all.iter().any(|l| l.text.contains("接下頁")));
    }

    fn manual_with(n: usize, layout: &str) -> ManualStatement {
        ManualStatement {
            stmt_no: "M202610-0003".into(),
            cust_name: "時光樹影吧檯".into(),
            period: "2026-10".into(),
            doc_date: "2026-10-31".into(),
            layout: layout.into(),
            tax_mode: "exclusive".into(),
            lines: (0..n)
                .map(|i| crate::models::ManualLine { id: None, line_date: format!("2026-{:02}-{:02}", 10 + i / 31, i % 31 + 1), amount: 9065, note: if i == 4 { "含冷凍品，超長的備註會被截斷".into() } else { String::new() } })
                .collect(),
            ..Default::default()
        }
    }

    fn count_dates(r: &RenderedDoc) -> usize {
        r.pages.iter().flatten().map(|l| l.text.matches("9,065").count()).sum()
    }

    #[test]
    fn manual_layouts() {
        let s = AppSettings::default();
        // 單欄：跟出貨單一樣，一張 14 筆
        for (n, sheets) in [(1, 1), (14, 1), (15, 2), (26, 2), (31, 2), (43, 2), (44, 3)] {
            let r = render_manual(&manual_with(n, "single"), &s).unwrap();
            check(&r);
            assert_eq!(r.pages.len(), sheets, "single n={}", n);
            assert_eq!(count_dates(&r), n);
            assert!(r.pages[0][1].text.contains("10月份對帳單"));
        }
        // 兩欄：一張 28 筆，超過接續下一張
        for (n, sheets) in [(1, 1), (13, 1), (26, 1), (28, 1), (29, 2), (31, 2), (40, 2)] {
            let r = render_manual(&manual_with(n, "double"), &s).unwrap();
            check(&r);
            assert_eq!(r.pages.len(), sheets, "double n={}", n);
            assert_eq!(count_dates(&r), n, "double n={}", n);
            let all: Vec<&Line> = r.pages.iter().flatten().collect();
            assert_eq!(all.iter().filter(|l| l.text.contains("合計金額")).count(), 1);
            // 只有一次欄位名稱列（左右各一組）
            assert_eq!(all.iter().filter(|l| l.text.contains("# 日期")).count(), 1);
        }
        // 合計標籤不可被截斷
        for layout in ["single", "double"] {
            for n in [3, 26, 31] {
                let r = render_manual(&manual_with(n, layout), &s).unwrap();
                let all: Vec<&Line> = r.pages.iter().flatten().collect();
                for lab in ["未稅金額：", "稅金金額：", "合計金額："] {
                    assert!(all.iter().any(|l| l.text.contains(lab)), "{} n={} 缺 {}", layout, n, lab);
                }
                assert!(!all.iter().any(|l| l.text.trim_end().ends_with('|')) || n != 31, "{} n={} 有多餘分隔線", layout, n);
            }
        }
        // 兩欄時左欄 1~13、右欄 14~26
        let r = render_manual(&manual_with(26, "double"), &s).unwrap();
        let first_row = r.pages[0].iter().find(|l| l.text.trim_start().starts_with("1 ")).unwrap();
        assert!(first_row.text.contains(" | ") && first_row.text.contains("14 10/14"), "{}", first_row.text);
    }

    #[test]
    fn paper_modes() {
        // half（預設）：每頁都是 33 行的半張，多頁接續
        let s = AppSettings::default();
        assert_eq!(s.print.paper_mode, "half");
        // 連續報表沒有頁碼行，一張中一刀放得下 14 筆
        for n in 0..=14 {
            let r = render_document(&doc_with(n), &s).unwrap();
            assert!(r.half);
            assert_eq!(r.page_lines, 33);
            assert_eq!(r.pages.len(), 1, "n={}", n);
            check(&r);
        }
        let r = render_document(&doc_with(15), &s).unwrap();
        assert!(r.half);
        assert_eq!(r.pages.len(), 2);
        let r = render_document(&doc_with(40), &s).unwrap();
        assert_eq!(r.page_lines, 33);
        assert!(r.pages.len() >= 2);
        check(&r);
        let bytes = render_escp(&r);
        assert!(bytes.windows(3).any(|w| w == [ESC, b'C', 33]));

        // auto：半張放得下（連續報表 14 筆）就印半張，超過印整張
        let a = with_mode("auto");
        assert!(render_document(&doc_with(14), &a).unwrap().half);
        let r = render_document(&doc_with(15), &a).unwrap();
        assert!(!r.half);
        assert_eq!(r.page_lines, 66);

        // full：一律整張
        let r = render_document(&doc_with(3), &with_mode("full")).unwrap();
        assert!(!r.half);
        assert_eq!(r.page_lines, 66);
    }

    #[test]
    fn print_sample() {
        let s = AppSettings::default();
        let r = render_document(&doc_with(55), &s).unwrap();
        for (i, p) in r.pages.iter().enumerate() {
            println!("===== page {} =====", i + 1);
            for l in p {
                println!("|{}", l.text);
            }
        }
        let bytes = render_escp(&r);
        assert!(bytes.starts_with(&[ESC, b'@']));
        assert_eq!(bytes.iter().filter(|b| **b == 0x0C).count(), r.pages.len());
    }

    #[test]
    fn big5() {
        assert!(non_big5_chars("白蝦透抽鮭魚蛤蜊干貝鱸魚蚵仔（）：　，、").is_empty());
        assert_eq!(non_big5_chars("鱼虾魚"), vec!['鱼', '虾']);
        assert_eq!(non_big5_chars("㗎"), vec!['㗎']);
    }
}










