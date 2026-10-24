//! 直接把 ESC/P 位元組送進 Windows 列印佇列（RAW），不經過驅動程式排版。

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    #[repr(C)]
    struct DocInfo1W {
        doc_name: *const u16,
        output_file: *const u16,
        datatype: *const u16,
    }

    #[repr(C)]
    struct PrinterInfo4W {
        printer_name: *mut u16,
        server_name: *mut u16,
        attributes: u32,
    }

    #[link(name = "winspool")]
    extern "system" {
        fn OpenPrinterW(name: *const u16, handle: *mut isize, defaults: *const c_void) -> i32;
        fn ClosePrinter(h: isize) -> i32;
        fn StartDocPrinterW(h: isize, level: u32, info: *const DocInfo1W) -> u32;
        fn EndDocPrinter(h: isize) -> i32;
        fn StartPagePrinter(h: isize) -> i32;
        fn EndPagePrinter(h: isize) -> i32;
        fn WritePrinter(h: isize, buf: *const c_void, cb: u32, written: *mut u32) -> i32;
        fn EnumPrintersW(flags: u32, name: *const u16, level: u32, buf: *mut u8, cb: u32, needed: *mut u32, returned: *mut u32) -> i32;
        fn GetDefaultPrinterW(buf: *mut u16, size: *mut u32) -> i32;
    }

    const PRINTER_ENUM_LOCAL: u32 = 2;
    const PRINTER_ENUM_CONNECTIONS: u32 = 4;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn from_wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }

    pub fn list_printers() -> Result<Vec<String>, String> {
        unsafe {
            let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
            let mut needed = 0u32;
            let mut count = 0u32;
            EnumPrintersW(flags, null(), 4, null_mut(), 0, &mut needed, &mut count);
            if needed == 0 {
                return Ok(vec![]);
            }
            // 用 u64 陣列確保結構對齊
            let mut buf = vec![0u64; (needed as usize + 7) / 8];
            let ok = EnumPrintersW(flags, null(), 4, buf.as_mut_ptr() as *mut u8, needed, &mut needed, &mut count);
            if ok == 0 {
                return Err("無法取得印表機清單".into());
            }
            let infos = buf.as_ptr() as *const PrinterInfo4W;
            let mut out = vec![];
            for i in 0..count as usize {
                out.push(from_wide((*infos.add(i)).printer_name));
            }
            Ok(out)
        }
    }

    pub fn default_printer() -> Option<String> {
        unsafe {
            let mut size = 0u32;
            GetDefaultPrinterW(null_mut(), &mut size);
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u16; size as usize];
            if GetDefaultPrinterW(buf.as_mut_ptr(), &mut size) == 0 {
                return None;
            }
            Some(from_wide(buf.as_ptr()))
        }
    }

    pub fn print_raw(printer: &str, job_name: &str, data: &[u8]) -> Result<(), String> {
        unsafe {
            let name = wide(printer);
            let mut h: isize = 0;
            if OpenPrinterW(name.as_ptr(), &mut h, null()) == 0 || h == 0 {
                return Err(format!("無法開啟印表機「{}」，請到設定確認印表機名稱", printer));
            }
            let job = wide(job_name);
            let raw = wide("RAW");
            let info = DocInfo1W { doc_name: job.as_ptr(), output_file: null(), datatype: raw.as_ptr() };
            if StartDocPrinterW(h, 1, &info) == 0 {
                ClosePrinter(h);
                return Err("印表機拒絕列印工作（StartDocPrinter 失敗）".into());
            }
            StartPagePrinter(h);
            let mut written = 0u32;
            let ok = WritePrinter(h, data.as_ptr() as *const c_void, data.len() as u32, &mut written);
            EndPagePrinter(h);
            EndDocPrinter(h);
            ClosePrinter(h);
            if ok == 0 || written as usize != data.len() {
                return Err(format!("送資料到印表機失敗（送出 {} / {} 位元組）", written, data.len()));
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
pub use win::{default_printer, list_printers, print_raw};

#[cfg(not(windows))]
pub fn list_printers() -> Result<Vec<String>, String> {
    Ok(vec![])
}

#[cfg(not(windows))]
pub fn default_printer() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn print_raw(_printer: &str, _job_name: &str, _data: &[u8]) -> Result<(), String> {
    Err("文字模式列印只支援 Windows".into())
}
