use crate::models::DocLine;

/// 一個字元在點陣印表機上佔幾個半形寬度：中文與全形符號佔 2。
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x1100 {
        return 1;
    }
    let wide = (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE30..=0xFE4F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x20000..=0x3FFFD).contains(&u)
        || (0x2500..=0x257F).contains(&u);
    if wide {
        2
    } else {
        1
    }
}

pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// 依顯示寬度截斷，不會把中文字切一半。
pub fn truncate_width(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = char_width(c);
        if w + cw > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

pub fn pad_right(s: &str, width: usize) -> String {
    let t = truncate_width(s, width);
    let w = str_width(&t);
    format!("{}{}", t, " ".repeat(width - w))
}

pub fn pad_left(s: &str, width: usize) -> String {
    let t = truncate_width(s, width);
    let w = str_width(&t);
    format!("{}{}", " ".repeat(width - w), t)
}

pub fn center(s: &str, width: usize) -> String {
    let t = truncate_width(s, width);
    let w = str_width(&t);
    let left = (width - w) / 2;
    format!("{}{}", " ".repeat(left), t)
}

/// 四捨五入到整數（金額一律整數元）。
pub fn round_i64(x: f64) -> i64 {
    x.round() as i64
}

/// 1,234,567 這種千分位格式；負數前面加「-」。
pub fn fmt_int(n: i64) -> String {
    let neg = n < 0;
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if neg {
        format!("-{}", out)
    } else {
        out
    }
}

/// 單價或數量：整數就不顯示小數，否則最多 4 位小數並去掉尾端 0。
pub fn fmt_num(x: f64) -> String {
    if (x - x.round()).abs() < 1e-9 {
        return fmt_int(x.round() as i64);
    }
    let s = format!("{:.4}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    let (int_part, frac) = match s.split_once('.') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (s.clone(), None),
    };
    let i: i64 = int_part.parse().unwrap_or(0);
    let mut out = if int_part.starts_with('-') && i == 0 {
        "-0".to_string()
    } else {
        fmt_int(i)
    };
    if let Some(f) = frac {
        out.push('.');
        out.push_str(&f);
    }
    out
}

/// 全形數字與符號轉半形。
fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '０'..='９' => char::from_u32(c as u32 - 0xFF10 + '0' as u32).unwrap_or(c),
            '．' => '.',
            '　' => ' ',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// 解析數量文字：「3」「2.5」「2斤5兩」「2斤」「5兩」「1斤8」。
/// 斤兩一律換算成斤（1 斤 = 16 兩）。
pub fn parse_qty(text: &str) -> Result<f64, String> {
    let s = normalize(text).replace(' ', "");
    if s.is_empty() {
        return Err("數量不可空白".into());
    }
    let num = |t: &str| -> Result<f64, String> {
        t.parse::<f64>()
            .map_err(|_| format!("看不懂數量「{}」", text))
    };
    let value = if let Some((jin, rest)) = s.split_once('斤') {
        let j = if jin.is_empty() { 0.0 } else { num(jin)? };
        let rest = rest.trim_end_matches('兩');
        let l = if rest.is_empty() { 0.0 } else { num(rest)? };
        if l >= 16.0 {
            return Err(format!("「{}」的兩數要小於 16", text));
        }
        j + l / 16.0
    } else if let Some(liang) = s.strip_suffix('兩') {
        num(liang)? / 16.0
    } else {
        num(&s)?
    };
    if value < 0.0 {
        return Err("數量不可為負數".into());
    }
    Ok(value)
}

pub fn line_amount(qty: f64, price: f64) -> i64 {
    round_i64(qty * price)
}

/// 依稅別計算（未稅, 稅金, 合計）。sign 為 -1 時代表退貨／折讓。
pub fn compute_totals(lines: &[DocLine], tax_mode: &str, sign: i64) -> (i64, i64, i64) {
    let sum: i64 = lines.iter().map(|l| l.amount).sum();
    let (sub, tax, total) = totals_from_sum(sum, tax_mode);
    (sub * sign, tax * sign, total * sign)
}

/// 由金額加總依稅別算出（未稅, 稅金, 合計）。
pub fn totals_from_sum(sum: i64, tax_mode: &str) -> (i64, i64, i64) {
    match tax_mode {
        "inclusive" => {
            let sub = round_i64(sum as f64 / 1.05);
            (sub, sum - sub, sum)
        }
        "none" => (sum, 0, sum),
        _ => {
            let tax = round_i64(sum as f64 * 0.05);
            (sum, tax, sum + tax)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qty() {
        assert_eq!(parse_qty("3").unwrap(), 3.0);
        assert_eq!(parse_qty("2.5").unwrap(), 2.5);
        assert_eq!(parse_qty("2斤5兩").unwrap(), 2.3125);
        assert_eq!(parse_qty("２斤８").unwrap(), 2.5);
        assert_eq!(parse_qty("8兩").unwrap(), 0.5);
        assert_eq!(parse_qty("1斤").unwrap(), 1.0);
        assert!(parse_qty("2斤16兩").is_err());
        assert!(parse_qty("abc").is_err());
    }

    #[test]
    fn widths() {
        assert_eq!(str_width("白蝦A1"), 6);
        assert_eq!(pad_right("透抽", 6), "透抽  ");
        assert_eq!(pad_left("12", 5), "   12");
        assert_eq!(truncate_width("鮭魚切片", 5), "鮭魚");
    }

    #[test]
    fn numbers() {
        assert_eq!(fmt_int(1234567), "1,234,567");
        assert_eq!(fmt_int(-630), "-630");
        assert_eq!(fmt_num(380.0), "380");
        assert_eq!(fmt_num(2.3125), "2.3125");
        assert_eq!(fmt_num(1234.5), "1,234.5");
    }

    #[test]
    fn totals() {
        let l = |a| DocLine { amount: a, ..Default::default() };
        assert_eq!(compute_totals(&[l(5124)], "exclusive", 1), (5124, 256, 5380));
        assert_eq!(compute_totals(&[l(1050)], "inclusive", 1), (1000, 50, 1050));
        assert_eq!(compute_totals(&[l(600)], "exclusive", -1), (-600, -30, -630));
    }
}
