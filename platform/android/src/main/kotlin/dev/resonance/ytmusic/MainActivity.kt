package dev.resonance.ytmusic

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.MutableContextWrapper
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture

/**
 * Resonance Android Host Activity.
 *
 * 1. Enables edge-to-edge drawing (`WindowCompat.setDecorFitsSystemWindows(window, false)`)
 *    and applies safe-area insets so NSH `.pb_safe_area()` renders cleanly.
 * 2. Initializes `ResonanceNativeBridge` (`JavaVM` + `filesDir`/`cacheDir` via `nativeInitHost` /
 *    `nativeInitPaths`) **before** mounting the NSH native host surface.
 * 3. Mounts the NSH Rust UI surface (`com.longbridge.nsh.NshView`) when packaged with NSH,
 *    with a clean native fallback container for standalone Gradle builds.
 * 4. Forwards predictive/system back events to NSH (`nativeOnBackPressed()`); moves the task
 *    to back on root instead of killing audio playback.
 * 5. Binds `PlaybackForegroundService` via Media3 `MediaController.Builder`.
 */
class MainActivity : Activity() {
    private var signInDialog: AlertDialog? = null
    private var nshView: View? = null
    private var mediaControllerFuture: ListenableFuture<MediaController>? = null
    private var backCallback: Any? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // 1. Edge-to-edge system bars; NSH handles safe-area insets.
        WindowCompat.setDecorFitsSystemWindows(window, false)

        // 2. Initialize Rust JNI host + AppPaths BEFORE mounting NSH UI or Tokio work.
        ResonanceNativeBridge.initHost(applicationContext)
        ResonanceNativeBridge.registerActivity(this)
        runCatching {
            ResonanceNativeBridge.nativeInitHost(
                ResonanceNativeBridge::class.java,
                filesDir.absolutePath,
                cacheDir.absolutePath,
            )
            ResonanceNativeBridge.nativeInitPaths(
                filesDir.absolutePath,
                cacheDir.absolutePath,
            )
        }

        requestNotificationPermissionIfNeeded()

        ContextCompat.startForegroundService(
            this,
            Intent(this, PlaybackForegroundService::class.java),
        )

        // 3. Mount NSH Rust host surface when bundled, else minimal fallback host.
        val mountedNsh = tryMountNshHostView()
        if (mountedNsh != null) {
            nshView = mountedNsh
            setContentView(mountedNsh)
        } else {
            val root = FrameLayout(this).apply {
                setBackgroundColor(Color.parseColor("#0A0B0D"))
            }
            val fallback = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.CENTER
                setPadding(48, 64, 48, 64)
            }
            val title = TextView(this).apply {
                text = "Raag"
                textSize = 26f
                setTextColor(Color.parseColor("#F4F5F7"))
                gravity = Gravity.CENTER
            }
            val subtitle = TextView(this).apply {
                text = "Pure Rust YouTube Music • arm64-v8a + Media3"
                textSize = 14f
                setTextColor(Color.parseColor("#9FA6B2"))
                gravity = Gravity.CENTER
                setPadding(0, 16, 0, 0)
            }
            fallback.addView(title)
            fallback.addView(subtitle)
            root.addView(fallback)
            ViewCompat.setOnApplyWindowInsetsListener(root) { v, insets ->
                val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
                v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
                insets
            }
            setContentView(root)
        }

        // 4. Predictive back registration (Android 13+).
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val cb = OnBackInvokedCallback { handleBackNavigation() }
            backCallback = cb
            onBackInvokedDispatcher.registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                cb,
            )
        }
    }

    override fun onStart() {
        super.onStart()
        val sessionToken = SessionToken(
            this,
            ComponentName(this, PlaybackForegroundService::class.java),
        )
        mediaControllerFuture = MediaController.Builder(this, sessionToken).buildAsync()
    }

    override fun onStop() {
        mediaControllerFuture?.let { MediaController.releaseFuture(it) }
        mediaControllerFuture = null
        super.onStop()
    }

    private fun tryMountNshHostView(): View? {
        return runCatching {
            val cls = Class.forName("com.longbridge.nsh.NshView")
            val ctor = cls.getConstructor(Context::class.java)
            val view = ctor.newInstance(MutableContextWrapper(this)) as View
            runCatching { cls.getMethod("useStandaloneLimits").invoke(view) }
            runCatching { cls.getMethod("ownSystemBars").invoke(view) }
            runCatching { cls.getMethod("runRustApp", String::class.java).invoke(view, "nsh") }
            view
        }.getOrNull()
    }

    private fun handleBackNavigation() {
        val nshHandled = runCatching {
            val view = nshView ?: return@runCatching false
            val method = view.javaClass.getMethod("handleBack")
            (method.invoke(view) as? Boolean) == true
        }.getOrDefault(false)

        if (nshHandled) return

        val rustHandled = runCatching {
            ResonanceNativeBridge.nativeOnBackPressed()
        }.getOrDefault(false)

        if (!rustHandled) {
            // Keep Media3 ForegroundService alive when user backs out from root screen.
            moveTaskToBack(true)
        }
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        handleBackNavigation()
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
        val launchedActivity = runCatching {
            startActivity(Intent(this, WebViewSignInActivity::class.java))
            true
        }.getOrDefault(false)
        if (launchedActivity) return

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
            .setNegativeButton("Cancel") { d, _ ->
                ResonanceNativeBridge.nativeOnWebViewCancelled()
                d.dismiss()
            }
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
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            (backCallback as? OnBackInvokedCallback)?.let { cb ->
                runCatching { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(cb) }
            }
            backCallback = null
        }
        signInDialog?.dismiss()
        signInDialog = null
        runCatching {
            nshView?.let { it.javaClass.getMethod("destroy").invoke(it) }
        }
        nshView = null
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
