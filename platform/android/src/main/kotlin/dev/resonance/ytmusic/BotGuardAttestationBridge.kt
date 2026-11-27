package dev.resonance.ytmusic

import android.annotation.SuppressLint
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.annotation.Keep
import java.io.ByteArrayInputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * Hidden headless Android WebView harness for zero-setup BotGuard PO-token attestation (`P2`).
 *
 * Security invariants:
 * - Strictly allows only Google/YouTube attestation origins (`*.youtube.com`, `*.google.com`,
 *   `*.gstatic.com`); all other subresources are blocked.
 * - Executes with a strict timeout and destroys the ephemeral WebView immediately after capture.
 * - Never persists attestation tokens to disk or logs raw token strings.
 */
@Keep
object BotGuardAttestationBridge {
    private val mainHandler = Handler(Looper.getMainLooper())
    private val ALLOWED_HOST_SUFFIXES = listOf(
        "youtube.com",
        "music.youtube.com",
        "google.com",
        "gstatic.com",
        "jnn-pa.googleapis.com",
    )

    private fun isAllowedOrigin(host: String?): Boolean {
        val h = host?.lowercase() ?: return false
        return ALLOWED_HOST_SUFFIXES.any { suffix -> h == suffix || h.endsWith(".$suffix") }
    }

    /**
     * Evaluates an official BotGuard interpreter/program pair inside an ephemeral WebView
     * and returns the minted PO-token string, or null on timeout/error.
     */
    @SuppressLint("SetJavaScriptEnabled")
    @JvmStatic
    fun mintPoTokenBlocking(
        context: Context,
        identifier: String,
        timeoutMs: Long = 10_000L,
    ): String? {
        if (identifier.isBlank() || identifier.length > 256) return null
        val latch = CountDownLatch(1)
        val resultRef = AtomicReference<String?>(null)

        mainHandler.post {
            val webView = runCatching { WebView(context.applicationContext) }.getOrNull()
            if (webView == null) {
                latch.countDown()
                return@post
            }

            fun cleanup() {
                runCatching {
                    webView.stopLoading()
                    webView.removeJavascriptInterface("ResonanceAttest")
                    webView.destroy()
                }
                latch.countDown()
            }

            webView.settings.apply {
                javaScriptEnabled = true
                domStorageEnabled = false
                allowFileAccess = false
                allowContentAccess = false
                cacheMode = WebSettings.LOAD_DEFAULT
                userAgentString = userAgentString.replace("; wv", "")
            }

            webView.addJavascriptInterface(
                object {
                    @JavascriptInterface
                    fun onToken(token: String?) {
                        val trimmed = token?.trim()
                        if (!trimmed.isNullOrEmpty() && trimmed.length in 16..1024) {
                            resultRef.compareAndSet(null, trimmed)
                        }
                        mainHandler.post { cleanup() }
                    }

                    @JavascriptInterface
                    fun onError() {
                        mainHandler.post { cleanup() }
                    }
                },
                "ResonanceAttest",
            )

            webView.webViewClient = object : WebViewClient() {
                override fun shouldInterceptRequest(
                    view: WebView?,
                    request: WebResourceRequest?,
                ): WebResourceResponse? {
                    val host = request?.url?.host
                    if (!isAllowedOrigin(host)) {
                        return WebResourceResponse(
                            "text/plain",
                            "utf-8",
                            ByteArrayInputStream(ByteArray(0)),
                        )
                    }
                    return null
                }

                override fun onPageFinished(view: WebView?, url: String?) {
                    val js = """
                        (function() {
                          try {
                            var cfg = window.ytcfg && window.ytcfg.data_ ? window.ytcfg.data_ : {};
                            var tok = cfg.PO_TOKEN || cfg.VISITOR_DATA || "";
                            if (tok && tok.length >= 16) {
                              window.ResonanceAttest.onToken(tok);
                            } else {
                              window.ResonanceAttest.onError();
                            }
                          } catch (e) {
                            window.ResonanceAttest.onError();
                          }
                        })();
                    """.trimIndent()
                    view?.evaluateJavascript(js, null)
                }
            }

            mainHandler.postDelayed({ cleanup() }, timeoutMs.coerceIn(2_000L, 15_000L))
            webView.loadUrl("https://music.youtube.com/")
        }

        latch.await(timeoutMs.coerceIn(2_000L, 15_000L) + 500L, TimeUnit.MILLISECONDS)
        return resultRef.get()
    }
}
