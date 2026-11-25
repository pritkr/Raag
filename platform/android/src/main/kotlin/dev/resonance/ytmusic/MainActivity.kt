package dev.resonance.ytmusic

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.content.ContextCompat

/**
 * Resonance Android Host Activity.
 *
 * Initializes `ResonanceNativeBridge` (`JavaVM` + `filesDir`/`cacheDir`), starts the
 * Media3 `PlaybackForegroundService` via `ContextCompat.startForegroundService`, requests
 * `POST_NOTIFICATIONS` on Android 13+, and hosts the `; wv`-stripped Google Sign-In WebView modal.
 */
class MainActivity : Activity() {
    private var signInDialog: AlertDialog? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        ResonanceNativeBridge.initHost(applicationContext)
        ResonanceNativeBridge.registerActivity(this)
        runCatching {
            ResonanceNativeBridge.nativeInitHost(
                ResonanceNativeBridge::class.java,
                filesDir.absolutePath,
                cacheDir.absolutePath,
            )
        }

        requestNotificationPermissionIfNeeded()

        ContextCompat.startForegroundService(
            this,
            Intent(this, PlaybackForegroundService::class.java),
        )

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setBackgroundColor(Color.parseColor("#0A0B0D"))
            setPadding(48, 64, 48, 64)
        }

        val title = TextView(this).apply {
            text = "Resonance"
            textSize = 26f
            setTextColor(Color.parseColor("#F4F5F7"))
            gravity = Gravity.CENTER
        }

        val subtitle = TextView(this).apply {
            text = "Pure Rust YouTube Music • arm64-v8a + Media3 Foreground Service"
            textSize = 14f
            setTextColor(Color.parseColor("#9FA6B2"))
            gravity = Gravity.CENTER
            setPadding(0, 16, 0, 0)
        }

        root.addView(title)
        root.addView(subtitle)
        setContentView(root)
    }

    private fun requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val granted = ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.POST_NOTIFICATIONS,
            ) == PackageManager.PERMISSION_GRANTED
            if (!granted) {
                requestPermissions(
                    arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                    REQ_POST_NOTIFICATIONS,
                )
            }
        }
    }

    fun launchGoogleSignInWebView() {
        if (isFinishing || isDestroyed) return
        signInDialog?.dismiss()

        val webView = WebView(this).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                1200,
            )
        }

        val dialog = AlertDialog.Builder(this)
            .setTitle("Sign in to YouTube Music")
            .setView(webView)
            .setNegativeButton("Cancel") { d, _ -> d.dismiss() }
            .setOnDismissListener {
                webView.stopLoading()
                webView.loadUrl("about:blank")
                webView.destroy()
                if (signInDialog === it) {
                    signInDialog = null
                }
            }
            .create()

        PlaybackForegroundService.configureGoogleSignInWebView(webView) { cookieHeader ->
            ResonanceNativeBridge.nativeOnAuthCookiesCaptured(cookieHeader)
            dialog.dismiss()
        }

        signInDialog = dialog
        dialog.show()
    }

    override fun onDestroy() {
        signInDialog?.dismiss()
        signInDialog = null
        ResonanceNativeBridge.unregisterActivity(this)
        super.onDestroy()
    }

    companion object {
        private const val REQ_POST_NOTIFICATIONS = 1001

        init {
            runCatching { System.loadLibrary("resonance") }
        }
    }
}
