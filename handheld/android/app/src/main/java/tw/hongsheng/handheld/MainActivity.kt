package tw.hongsheng.handheld

import android.Manifest
import android.annotation.SuppressLint
import android.content.Intent
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.webkit.WebViewAssetLoader
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors

/**
 * 外殼：用 WebView 顯示 handheld/web 的畫面，提供 window.Native 給畫面使用
 * （存檔、連電腦、列印）。畫面與邏輯都在 HTML/JS，這裡只做手機才能做的事。
 */
class MainActivity : AppCompatActivity() {
    private lateinit var web: WebView
    private lateinit var printer: Printer
    private val io = Executors.newFixedThreadPool(3)
    private val main = Handler(Looper.getMainLooper())
    private var netCallback: ConnectivityManager.NetworkCallback? = null

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        printer = Printer(this)
        printer.bindSunmi()

        val loader = WebViewAssetLoader.Builder()
            .addPathHandler("/assets/", WebViewAssetLoader.AssetsPathHandler(this))
            .build()
        web = WebView(this)
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        web.settings.textZoom = 100 // 不受系統字體大小影響版面
        web.webViewClient = object : WebViewClient() {
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                loader.shouldInterceptRequest(request.url)

            // 客戶頁的「撥打」：交給手機的電話 App
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                if (request.url.scheme == "tel") {
                    try { startActivity(Intent(Intent.ACTION_DIAL, request.url)) } catch (_: Exception) { }
                    return true
                }
                return false
            }
        }
        web.addJavascriptInterface(Bridge(), "Native")
        setContentView(web)
        web.loadUrl("https://appassets.androidplatform.net/assets/index.html")

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                web.evaluateJavascript("window.onAndroidBack ? window.onAndroidBack() : false") { r ->
                    if (r != "true") moveTaskToBack(true)
                }
            }
        })

        // 網路恢復時自動同步
        val cm = getSystemService(ConnectivityManager::class.java)
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                main.postDelayed({ callJs("window.onNetworkAvailable && window.onNetworkAvailable()") }, 1500)
            }
        }
        try { cm.registerDefaultNetworkCallback(cb); netCallback = cb } catch (_: Exception) { }

        if (Build.VERSION.SDK_INT >= 31 &&
            checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.BLUETOOTH_CONNECT), 1)
        }
    }

    override fun onResume() {
        super.onResume()
        if (::web.isInitialized) callJs("window.onNativeResume && window.onNativeResume()")
    }

    override fun onDestroy() {
        netCallback?.let { try { getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(it) } catch (_: Exception) { } }
        printer.unbind()
        io.shutdown()
        super.onDestroy()
    }

    private fun callJs(js: String) = main.post { web.evaluateJavascript(js, null) }

    private fun reply(id: String, ok: Boolean, status: Int, body: String) =
        callJs("window.__native(${JSONObject.quote(id)}, $ok, $status, ${JSONObject.quote(body)})")

    inner class Bridge {
        private val stateFile get() = File(filesDir, "state.json")

        @JavascriptInterface
        fun loadState(): String = try {
            if (stateFile.exists()) stateFile.readText() else ""
        } catch (e: Exception) { "" }

        // 先寫暫存檔再換名，斷電或當機也不會留下寫一半的檔案
        @JavascriptInterface
        fun saveState(json: String): String = try {
            val tmp = File(filesDir, "state.json.tmp")
            tmp.outputStream().use { out -> out.write(json.toByteArray(Charsets.UTF_8)); out.fd.sync() }
            if (!tmp.renameTo(stateFile)) throw Exception("無法寫入資料檔")
            "ok"
        } catch (e: Exception) { e.message ?: "存檔失敗" }

        @JavascriptInterface
        fun request(id: String, method: String, url: String, headersJson: String, body: String) {
            io.execute {
                try {
                    val conn = URL(url).openConnection() as HttpURLConnection
                    conn.requestMethod = method
                    conn.connectTimeout = 8000
                    conn.readTimeout = 60000
                    val headers = JSONObject(headersJson)
                    headers.keys().forEach { k -> conn.setRequestProperty(k, headers.getString(k)) }
                    if (method != "GET" && body.isNotEmpty()) {
                        conn.doOutput = true
                        conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
                    }
                    val status = conn.responseCode
                    val stream = if (status >= 400) conn.errorStream else conn.inputStream
                    val text = stream?.bufferedReader(Charsets.UTF_8)?.use { it.readText() } ?: ""
                    conn.disconnect()
                    reply(id, true, status, text)
                } catch (e: Exception) {
                    reply(id, false, 0, "offline: ${e.javaClass.simpleName} ${e.message ?: ""}")
                }
            }
        }

        @JavascriptInterface
        fun print(id: String, pngBase64: String, target: String) {
            io.execute {
                try {
                    printer.print(pngBase64, target)
                    reply(id, true, 0, "ok")
                } catch (e: Exception) {
                    reply(id, false, 0, e.message ?: "列印失敗")
                }
            }
        }

        @JavascriptInterface
        fun printerInfo(): String = printer.info().toString()

        @JavascriptInterface
        fun deviceName(): String = "${Build.MANUFACTURER} ${Build.MODEL}".trim()

        @JavascriptInterface
        fun appVersion(): String = BuildConfig.VERSION_NAME
    }
}
