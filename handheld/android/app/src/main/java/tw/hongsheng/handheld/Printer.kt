package tw.hongsheng.handheld

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.os.Build
import android.util.Base64
import com.sunmi.peripheral.printer.InnerPrinterCallback
import com.sunmi.peripheral.printer.InnerPrinterManager
import com.sunmi.peripheral.printer.InnerResultCallback
import com.sunmi.peripheral.printer.SunmiPrinterService
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * 熱感印表機。畫面傳來的是單據圖片：80mm 紙 576 點寬、58mm 紙 384 點寬。
 * 藍牙印表機可以用 ESC/POS（大部分收據機）或 CPCL（斑馬行動印表機）送出。
 *  - auto：有商米內建印表機服務就用它；一般手機用已配對的藍牙印表機（例如 Zebra ZR128）。
 *  - bt:XX:XX:...：指定的藍牙熱感印表機（ESC/POS 點陣圖）。
 */
class Printer(private val ctx: Context) {
    @Volatile private var sunmi: SunmiPrinterService? = null
    private var sunmiError = ""

    private val callback = object : InnerPrinterCallback() {
        override fun onConnected(service: SunmiPrinterService) { sunmi = service }
        override fun onDisconnected() { sunmi = null }
    }

    fun bindSunmi() {
        try {
            if (!InnerPrinterManager.getInstance().bindService(ctx, callback)) sunmiError = "這台機器沒有商米印表機服務"
        } catch (e: Throwable) {
            sunmiError = "商米印表機服務無法使用：${e.message ?: e.javaClass.simpleName}"
        }
    }

    fun unbind() {
        try { InnerPrinterManager.getInstance().unBindService(ctx, callback) } catch (_: Throwable) { }
    }

    fun info(): JSONObject {
        val list = JSONArray()
        var err = if (sunmi == null) sunmiError else ""
        try {
            for (d in bondedDevices()) list.put(JSONObject().put("name", d.first).put("address", d.second))
        } catch (e: SecurityException) {
            err = (if (err.isEmpty()) "" else "$err；") + "沒有藍牙權限（請在系統設定允許「附近的裝置」）"
        } catch (_: Exception) { }
        return JSONObject().put("sunmi", sunmi != null).put("bluetooth", list).put("error", err)
    }

    @SuppressLint("MissingPermission")
    private fun bondedDevices(): List<Pair<String, String>> {
        if (Build.VERSION.SDK_INT >= 31 &&
            ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            throw SecurityException("no permission")
        }
        val adapter = ctx.getSystemService(BluetoothManager::class.java)?.adapter ?: return emptyList()
        return adapter.bondedDevices.map { (it.name ?: it.address) to it.address }
    }

    /** optsJson：{"printer": "auto" | "bt:位址", "lang": "escpos" | "cpcl"}（舊版只傳印表機字串） */
    fun print(pngBase64: String, optsJson: String) {
        val opts = try { JSONObject(optsJson) } catch (_: Exception) { JSONObject().put("printer", optsJson) }
        val target = opts.optString("printer", "auto")
        val cpcl = opts.optString("lang", "escpos") == "cpcl"
        val bytes = Base64.decode(pngBase64, Base64.DEFAULT)
        val bmp = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: throw Exception("單據圖片錯誤")
        if (target.startsWith("bt:")) { printBluetooth(bmp, target.removePrefix("bt:"), cpcl); return }
        val s = sunmi
        if (s != null) { printSunmi(s, bmp); return }
        // 一般手機：用已配對的藍牙印表機
        val bonded = try { bondedDevices() } catch (e: SecurityException) {
            throw Exception("沒有藍牙權限（請在系統設定允許「附近的裝置」）")
        }
        val printerLike = Regex("InnerPrinter|Zebra|ZR\\d|ZQ\\d|print|POS|MTP|XP-|RPP", RegexOption.IGNORE_CASE)
        val pick = bonded.firstOrNull { it.first.contains("InnerPrinter", ignoreCase = true) }
            ?: bonded.singleOrNull()
            ?: bonded.filter { printerLike.containsMatchIn(it.first) }.singleOrNull()
            ?: throw Exception(
                if (bonded.isEmpty()) "找不到印表機：請先到手機的「設定 → 藍牙」配對印表機"
                else "配對了好幾台藍牙裝置，請到 App 的「設定 → 印表機」選擇要用的印表機"
            )
        printBluetooth(bmp, pick.second, cpcl)
    }

    // 商米印表機狀態碼
    private fun stateMessage(code: Int): String? = when (code) {
        1 -> null
        2 -> "印表機準備中，請稍候再印"
        3 -> "印表機通訊異常"
        4 -> "沒有紙了，請放入熱感紙"
        5 -> "印表機過熱，請稍候"
        6 -> "印表機蓋子沒蓋好"
        505 -> "這台機器沒有印表機"
        else -> null
    }

    private fun printSunmi(s: SunmiPrinterService, bmp: Bitmap) {
        try { stateMessage(s.updatePrinterState())?.let { throw Exception(it) } } catch (e: android.os.RemoteException) { }
        val done = CountDownLatch(1)
        var error: String? = null
        val cb = object : InnerResultCallback() {
            override fun onRunResult(isSuccess: Boolean) { if (!isSuccess) error = "列印失敗"; done.countDown() }
            override fun onReturnString(result: String?) {}
            override fun onRaiseException(code: Int, msg: String?) { error = msg ?: "列印失敗（$code）"; done.countDown() }
            override fun onPrintResult(code: Int, msg: String?) {}
        }
        s.setAlignment(1, null)
        s.printBitmap(bmp, null)
        s.lineWrap(4, cb)
        if (!done.await(30, TimeUnit.SECONDS)) throw Exception("印表機沒有回應")
        error?.let { throw Exception(it) }
    }

    @SuppressLint("MissingPermission")
    private fun printBluetooth(bmp: Bitmap, address: String, cpcl: Boolean) {
        if (Build.VERSION.SDK_INT >= 31 &&
            ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            throw Exception("沒有藍牙權限（請在系統設定允許「附近的裝置」）")
        }
        val adapter = ctx.getSystemService(BluetoothManager::class.java)?.adapter ?: throw Exception("這台機器沒有藍牙")
        if (!adapter.isEnabled) throw Exception("藍牙沒有開")
        val dev = adapter.getRemoteDevice(address)
        val sock = dev.createRfcommSocketToServiceRecord(UUID.fromString("00001101-0000-1000-8000-00805F9B34FB"))
        try {
            adapter.cancelDiscovery()
            sock.connect()
            val data = if (cpcl) cpclRaster(bmp) else escposRaster(bmp)
            // 分批送，避免印表機緩衝區塞滿
            var off = 0
            while (off < data.size) {
                val n = minOf(4096, data.size - off)
                sock.outputStream.write(data, off, n)
                sock.outputStream.flush()
                off += n
                Thread.sleep(20)
            }
            Thread.sleep(1000L + data.size / 40) // 等資料送完再斷線
        } catch (e: java.io.IOException) {
            throw Exception("連不上藍牙印表機：${e.message ?: ""}")
        } finally {
            try { sock.close() } catch (_: Exception) { }
        }
    }

    /** 圖片寬度調成 8 的倍數、最寬 576 點（80mm），再轉成黑白點陣（1＝黑，高位元在左） */
    private class Mono(val bytesPerRow: Int, val height: Int, val rows: Array<ByteArray>)

    private fun toMono(src: Bitmap): Mono {
        val w0 = minOf(576, src.width) / 8 * 8
        val bmp = if (src.width == w0) src else Bitmap.createScaledBitmap(src, w0, src.height * w0 / src.width, true)
        val w = bmp.width
        val bytesPerRow = w / 8
        val px = IntArray(w)
        val rows = Array(bmp.height) { y ->
            bmp.getPixels(px, 0, w, 0, y, w, 1)
            ByteArray(bytesPerRow) { bx ->
                var b = 0
                for (bit in 0 until 8) {
                    val c = px[bx * 8 + bit]
                    val lum = (Color.red(c) * 299 + Color.green(c) * 587 + Color.blue(c) * 114) / 1000
                    if (Color.alpha(c) > 128 && lum < 128) b = b or (0x80 shr bit)
                }
                b.toByte()
            }
        }
        return Mono(bytesPerRow, bmp.height, rows)
    }

    /** ESC/POS：GS v 0 點陣圖，分段送（每段最多 240 行），最後走紙 4 行 */
    private fun escposRaster(src: Bitmap): ByteArray {
        val m = toMono(src)
        val out = ByteArrayOutputStream()
        out.write(byteArrayOf(0x1B, 0x40)) // 初始化
        var y0 = 0
        while (y0 < m.height) {
            val band = minOf(240, m.height - y0)
            out.write(byteArrayOf(0x1D, 0x76, 0x30, 0x00, (m.bytesPerRow and 0xFF).toByte(), (m.bytesPerRow shr 8).toByte(),
                (band and 0xFF).toByte(), (band shr 8).toByte()))
            for (y in y0 until y0 + band) out.write(m.rows[y])
            y0 += band
        }
        out.write(byteArrayOf(0x1B, 0x64, 4)) // 走紙
        return out.toByteArray()
    }

    /** CPCL（斑馬）：每段最多 800 行當一張「標籤」送出，收據紙上會接續印成一張 */
    private fun cpclRaster(src: Bitmap): ByteArray {
        val m = toMono(src)
        val out = ByteArrayOutputStream()
        val ascii = { t: String -> out.write(t.toByteArray(Charsets.US_ASCII)) }
        var y0 = 0
        while (y0 < m.height) {
            val band = minOf(800, m.height - y0)
            ascii("! 0 200 200 $band 1\r\n")
            ascii("PAGE-WIDTH ${m.bytesPerRow * 8}\r\n")
            ascii("CG ${m.bytesPerRow} $band 0 0 ")
            for (y in y0 until y0 + band) out.write(m.rows[y])
            ascii("\r\nPRINT\r\n")
            y0 += band
        }
        return out.toByteArray()
    }
}
