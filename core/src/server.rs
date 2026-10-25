//! 同步服務：手機透過 Tailscale 或區域網路連到電腦上的這個小型網頁服務。
//!
//! - `GET  /api/ping`  測試連線（不需配對）
//! - `POST /api/pair`  輸入電腦上產生的配對碼，取得之後同步用的 token
//! - `POST /api/sync`  上傳手機的變更並取回電腦的變更（標頭帶 `Authorization: Bearer <token>`）

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tiny_http::{Header, Method, Request, Response, Server};

use crate::db::Db;
use crate::sync::SyncRequest;

pub type SharedDb = Arc<Mutex<Option<Db>>>;

/// 上傳資料上限（一次同步的 JSON）
const MAX_BODY: u64 = 16 * 1024 * 1024;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PairBody {
    code: String,
    device_name: String,
}

#[derive(Debug, Serialize)]
struct Ping {
    app: &'static str,
    version: &'static str,
    company_name: String,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

pub struct SyncServer {
    server: Arc<Server>,
    thread: Option<JoinHandle<()>>,
    pub addr: SocketAddr,
}

impl SyncServer {
    /// 在指定連接埠啟動（0＝自動挑一個，測試用）
    pub fn start(db: SharedDb, port: u16) -> Result<SyncServer> {
        let server = Server::http(("0.0.0.0", port)).map_err(|e| anyhow!("同步服務無法使用連接埠 {}：{}", port, e))?;
        let addr = server.server_addr().to_ip().ok_or_else(|| anyhow!("無法取得同步服務的位址"))?;
        let server = Arc::new(server);
        let s2 = server.clone();
        let thread = std::thread::Builder::new().name("sync-server".into()).spawn(move || {
            for req in s2.incoming_requests() {
                handle(&db, req);
            }
        })?;
        Ok(SyncServer { server, thread: Some(thread), addr })
    }

    pub fn stop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SyncServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn json_response<T: Serialize>(status: u16, body: &T) -> Response<std::io::Cursor<Vec<u8>>> {
    let data = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    Response::from_data(data)
        .with_status_code(status)
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap())
}

fn error(status: u16, msg: impl Into<String>) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(status, &ErrorBody { error: msg.into() })
}

fn read_body(req: &mut Request) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    req.as_reader().take(MAX_BODY + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_BODY {
        return Err(anyhow!("資料太大"));
    }
    Ok(buf)
}

fn bearer(req: &Request) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| h.value.as_str().strip_prefix("Bearer ").map(|t| t.trim().to_string()))
}

fn handle(db: &SharedDb, mut req: Request) {
    let path = req.url().split('?').next().unwrap_or("").to_string();
    let method = req.method().clone();
    let resp = match (method, path.as_str()) {
        (Method::Get, "/api/ping") => {
            let company = db
                .lock()
                .ok()
                .and_then(|g| g.as_ref().and_then(|d| d.get_settings().ok()))
                .map(|s| s.company_name)
                .unwrap_or_default();
            json_response(200, &Ping { app: "hongsheng-shipping", version: env!("CARGO_PKG_VERSION"), company_name: company })
        }
        (Method::Post, "/api/pair") => match read_body(&mut req).and_then(|b| Ok(serde_json::from_slice::<PairBody>(&b)?)) {
            Err(e) => error(400, format!("{:#}", e)),
            Ok(body) => match db.lock() {
                Err(_) => error(500, "資料庫忙碌中"),
                Ok(guard) => match guard.as_ref() {
                    None => error(503, "電腦上的資料庫沒有開啟"),
                    Some(d) => match d.pair_device(&body.code, &body.device_name) {
                        Ok(r) => json_response(200, &r),
                        Err(e) => error(403, format!("{:#}", e)),
                    },
                },
            },
        },
        (Method::Post, "/api/sync") => {
            let token = bearer(&req);
            match read_body(&mut req).and_then(|b| Ok(serde_json::from_slice::<SyncRequest>(&b)?)) {
                Err(e) => error(400, format!("資料格式錯誤：{:#}", e)),
                Ok(body) => match db.lock() {
                    Err(_) => error(500, "資料庫忙碌中"),
                    Ok(mut guard) => match guard.as_mut() {
                        None => error(503, "電腦上的資料庫沒有開啟"),
                        Some(d) => match token.as_deref().map(|t| d.device_by_token(t)) {
                            None => error(401, "缺少手機憑證，請重新配對"),
                            Some(Err(e)) => error(401, format!("{:#}", e)),
                            Some(Ok(device)) => match d.sync(&device, &body) {
                                Ok(r) => json_response(200, &r),
                                Err(e) => error(500, format!("同步失敗：{:#}", e)),
                            },
                        },
                    },
                },
            }
        }
        _ => error(404, "沒有這個功能"),
    };
    let _ = req.respond(resp);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Customer, Product};
    use crate::sync::{PairResult, PushDoc, SyncResponse, WireDoc, WireLine};
    use serde_json::json;

    fn start() -> (SyncServer, SharedDb, String) {
        let db = Db::open_in_memory().unwrap();
        db.save_customer(&Customer { code: "C001".into(), name: "時光樹影吧檯".into(), tax_mode: "exclusive".into(), active: true, ..Default::default() }).unwrap();
        db.save_product(&Product { name: "白蝦（大）".into(), unit: "斤".into(), price: Some(380.0), active: true, ..Default::default() }).unwrap();
        let shared: SharedDb = Arc::new(Mutex::new(Some(db)));
        let srv = SyncServer::start(shared.clone(), 0).unwrap();
        let base = format!("http://127.0.0.1:{}", srv.addr.port());
        (srv, shared, base)
    }

    fn post(url: &str, token: Option<&str>, body: serde_json::Value) -> (u16, serde_json::Value) {
        let mut r = ureq::post(url);
        if let Some(t) = token {
            r = r.set("Authorization", &format!("Bearer {}", t));
        }
        match r.send_json(body) {
            Ok(resp) => (resp.status(), resp.into_json().unwrap()),
            Err(ureq::Error::Status(code, resp)) => (code, resp.into_json().unwrap()),
            Err(e) => panic!("{}", e),
        }
    }

    /// 模擬手機走完整個流程：測試連線 → 配對 → 第一次同步 → 離線開單後上傳
    #[test]
    fn handheld_over_http() {
        let (mut srv, shared, base) = start();

        let ping: serde_json::Value = ureq::get(&format!("{}/api/ping", base)).call().unwrap().into_json().unwrap();
        assert_eq!(ping["app"], "hongsheng-shipping");
        assert_eq!(ping["company_name"], "泓昇海鮮");

        // 沒配對就同步：拒絕
        let (code, body) = post(&format!("{}/api/sync", base), None, json!({}));
        assert_eq!(code, 401, "{}", body);
        let (code, _) = post(&format!("{}/api/sync", base), Some("guess"), json!({}));
        assert_eq!(code, 401);

        // 錯的配對碼
        let (code, _) = post(&format!("{}/api/pair", base), None, json!({"code": "123", "device_name": "x"}));
        assert_eq!(code, 403);

        // 電腦上產生配對碼，手機輸入
        let pc = shared.lock().unwrap().as_ref().unwrap().create_pairing_code().unwrap();
        let (code, body) = post(&format!("{}/api/pair", base), None, json!({"code": pc.code, "device_name": "V3 測試機"}));
        assert_eq!(code, 200, "{}", body);
        let pair: PairResult = serde_json::from_value(body).unwrap();
        assert_eq!(pair.prefix, "V1");
        // 配對碼只能用一次
        let (code, _) = post(&format!("{}/api/pair", base), None, json!({"code": pc.code, "device_name": "別台"}));
        assert_eq!(code, 403);

        // 第一次同步
        let (code, body) = post(&format!("{}/api/sync", base), Some(&pair.token), json!({"since_rev": 0}));
        assert_eq!(code, 200, "{}", body);
        let first: SyncResponse = serde_json::from_value(body).unwrap();
        assert!(first.full);
        assert_eq!(first.customers.len(), 1);
        assert_eq!(first.products.len(), 1);
        let cust = first.customers[0].uid.clone();
        let shrimp = first.products[0].uid.clone();

        // 離線開的單上傳
        let doc = WireDoc {
            uid: "hh-1".into(),
            doc_no: "V1-202610-0001".into(),
            customer_uid: cust,
            doc_date: "2026-10-25".into(),
            status: "issued".into(),
            lines: vec![WireLine { product_uid: Some(shrimp), name: "白蝦（大）".into(), unit: "斤".into(), qty_text: "2斤5兩".into(), price: 380.0, ..Default::default() }],
            ..Default::default()
        };
        let push = json!({"since_rev": first.server_rev, "documents": [PushDoc { doc, base_rev: None, deleted: false }]});
        let (code, body) = post(&format!("{}/api/sync", base), Some(&pair.token), push);
        assert_eq!(code, 200, "{}", body);
        let second: SyncResponse = serde_json::from_value(body).unwrap();
        assert_eq!(second.results[0].status, "applied", "{:?}", second.results);
        assert_eq!(second.documents.len(), 1);
        assert_eq!(second.documents[0].total, 923, "電腦重算：879 + 稅 44");

        // 在電腦上停用這台手機 → 不能再同步
        {
            let g = shared.lock().unwrap();
            let d = g.as_ref().unwrap();
            let dev = d.list_devices().unwrap().remove(0);
            assert!(dev.last_sync_at.is_some());
            d.revoke_device(dev.id).unwrap();
        }
        let (code, _) = post(&format!("{}/api/sync", base), Some(&pair.token), json!({"since_rev": second.server_rev}));
        assert_eq!(code, 401);

        // 格式錯誤
        let resp = ureq::post(&format!("{}/api/pair", base)).send_string("not json");
        assert!(matches!(resp, Err(ureq::Error::Status(400, _))));
        srv.stop();
    }
}
