use anyhow::{Context, Result};
use chrono::Local;
use shipping_core::db::Db;
use std::fs;
use std::path::{Path, PathBuf};

const KEEP_BACKUPS: usize = 30;

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".write_test");
    let ok = fs::write(&probe, b"ok").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// 優先使用 exe 旁邊的 data 資料夾（免安裝、整個資料夾帶著走）；
/// 沒有寫入權限時（例如放在 Program Files）改用 %APPDATA%。
pub fn pick_data_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let d = dir.join("data");
            if fs::create_dir_all(&d).is_ok() && writable(&d) {
                return d;
            }
        }
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join("HongshengShipping");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn open(data_dir: &Path) -> Result<Db> {
    fs::create_dir_all(data_dir).with_context(|| format!("無法建立資料夾 {}", data_dir.display()))?;
    Db::open(&data_dir.join("shipping.db"))
}

fn prune(dir: &Path) {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("shipping-") && n.ends_with(".db"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return,
    };
    files.sort();
    if files.len() > KEEP_BACKUPS {
        for f in &files[..files.len() - KEEP_BACKUPS] {
            let _ = fs::remove_file(f);
        }
    }
}

fn copy_extra(db: &Db, file: &Path) {
    if let Ok(s) = db.get_settings() {
        let extra = s.extra_backup_dir.trim();
        if !extra.is_empty() {
            let dir = PathBuf::from(extra);
            if fs::create_dir_all(&dir).is_ok() {
                if let Some(name) = file.file_name() {
                    let _ = fs::copy(file, dir.join(name));
                    prune(&dir);
                }
            }
        }
    }
}

/// 每天第一次開程式時備份一次。
pub fn daily_backup(db: &Db, data_dir: &Path) -> Result<Option<PathBuf>> {
    let dir = data_dir.join("backup");
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("shipping-{}.db", Local::now().format("%Y%m%d")));
    if file.exists() {
        return Ok(None);
    }
    db.backup_to(&file)?;
    prune(&dir);
    copy_extra(db, &file);
    Ok(Some(file))
}

pub fn backup_now(db: &Db, data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join("backup");
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("shipping-{}.db", Local::now().format("%Y%m%d-%H%M%S")));
    db.backup_to(&file)?;
    prune(&dir);
    copy_extra(db, &file);
    Ok(file)
}

pub fn last_backup(data_dir: &Path) -> Option<String> {
    let dir = data_dir.join("backup");
    let newest = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?.modified().ok()?;
            Some(m)
        })
        .max()?;
    let dt: chrono::DateTime<Local> = newest.into();
    Some(dt.format("%Y/%m/%d %H:%M").to_string())
}
