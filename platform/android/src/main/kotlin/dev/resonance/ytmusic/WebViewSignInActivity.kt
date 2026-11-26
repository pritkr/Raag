package dev.resonance.ytmusic

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebStorage
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher

/**
 * Isolated, ephemeral Android WebView Google Sign-In Activity.
 *
 * Strips `; wv` and `Version/X.Y` from the WebView User-Agent so Google Sign-In
 * does not reject the embedded browser with `403: disallowed_useragent`.
 * Once `SAPISID` / `__Secure-3PAPISID` and `SID` are present on `https://music.youtube.com`,
 * forwards cookies to Rust via `ResonanceNativeBridge.nativeOnAuthCookiesCaptured`,
 * wipes all WebView cookies/storage, destroys the WebView, and finishes.
 */
class WebViewSignInActivity : Activity() {
    private var webView: WebView? = null
    private var completed = false
    private var backCallback: Any? = null

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val container = FrameLayout(this).apply {
            setBackgroundColor(Color.parseColor("#0A0B0D"))
        }

        val wv = WebView(this).apply {
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.userAgentString = stripWebViewUaMarkers(settings.userAgentString)
        }

        wv.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                tryCaptureCookies(request.url)
                return false
            }

            override fun onPageFinished(view: WebView, url: String?) {
                val uri = url?.let { runCatching { Uri.parse(it) }.getOrNull() } ?: return
                tryCaptureCookies(uri)
            }
        }

        container.addView(wv)
        setContentView(container)
        webView = wv

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val cb = OnBackInvokedCallback { handleBackOrCancel() }
            backCallback = cb
            onBackInvokedDispatcher.registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                cb,
            )
        }

        wv.loadUrl(GOOGLE_SIGN_IN_URL)
    }

    private fun tryCaptureCookies(uri: Uri) {
        if (completed) return
        if (uri.scheme != "https" || uri.host != "music.youtube.com") return
        val cookies = CookieManager.getInstance().getCookie("https://music.youtube.com").orEmpty()
        val hasSapisid = cookies.contains("SAPISID=") || cookies.contains("__Secure-3PAPISID=")
        val hasSid = cookies.contains("SID=")
        if (hasSapisid && hasSid) {
            completed = true
            ResonanceNativeBridge.nativeOnAuthCookiesCaptured(cookies)
            teardownWebView()
            finish()
        }
    }

    private fun handleBackOrCancel() {
        val wv = webView
        if (wv != null && wv.canGoBack()) {
            wv.goBack()
            return
        }
        if (!completed) {
            completed = true
            ResonanceNativeBridge.nativeOnWebViewCancelled()
        }
        teardownWebView()
        finish()
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        handleBackOrCancel()
    }

    override fun onDestroy() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            (backCallback as? OnBackInvokedCallback)?.let { cb ->
                runCatching { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(cb) }
            }
            backCallback = null
        }
        if (!completed) {
            completed = true
            ResonanceNativeBridge.nativeOnWebViewCancelled()
        }
        teardownWebView()
        super.onDestroy()
    }

    private fun teardownWebView() {
        val wv = webView ?: return
        webView = null
        runCatching {
            CookieManager.getInstance().removeAllCookies(null)
            CookieManager.getInstance().flush()
            WebStorage.getInstance().deleteAllData()
            wv.clearHistory()
            wv.clearCache(true)
            wv.stopLoading()
            wv.loadUrl("about:blank")
            wv.destroy()
        }
    }

    companion object {
        private const val GOOGLE_SIGN_IN_URL =
            "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F"

        internal fun stripWebViewUaMarkers(ua: String): String {
            return ua
                .replace("; wv)", ")")
                .replace("; wv;", ";")
                .replace("; wv", "")
                .replace(Regex("Version/\\S+\\s*"), "")
                .trim()
        }
    }
}
