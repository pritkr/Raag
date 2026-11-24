package dev.resonance.ytmusic

import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.net.Uri
import android.webkit.CookieManager
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.core.content.ContextCompat
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * Thin Android Media Lifecycle Bridge required by Phase 0 Audit of NSH.
 *
 * Wraps Media3 ExoPlayer + MediaSessionService + AudioFocus + ACTION_AUDIO_BECOMING_NOISY
 * so playback survives screen lock, backgrounding, Bluetooth AVRCP, and headphone unplug.
 * Streams strictly from the in-process Rust Loopback HTTP Proxy (`http://127.0.0.1:<port>/stream/...`).
 */
class PlaybackForegroundService : MediaSessionService() {

    private var player: ExoPlayer? = null
    private var mediaSession: MediaSession? = null

    private val noisyReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                player?.pause()
                nativeOnMediaCommand(7, 0L) // MediaCommand::BecomingNoisy
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        val attrs = AudioAttributes.Builder()
            .setUsage(C.USAGE_MEDIA)
            .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
            .build()

        val exo = ExoPlayer.Builder(this)
            .setAudioAttributes(attrs, /* handleAudioFocus = */ true)
            .setHandleAudioBecomingNoisy(true)
            .build()

        player = exo
        mediaSession = MediaSession.Builder(this, exo).build()

        ContextCompat.registerReceiver(
            this,
            noisyReceiver,
            IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        val activePlayer = mediaSession?.player
        if (activePlayer == null || !activePlayer.playWhenReady || activePlayer.mediaItemCount == 0) {
            stopSelf()
        }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? {
        return mediaSession
    }

    fun playFromLoopbackProxy(
        proxyUrl: String,
        title: String,
        artist: String,
        album: String?,
        artworkUrl: String?,
    ) {
        val metadata = MediaMetadata.Builder()
            .setTitle(title)
            .setArtist(artist)
            .setAlbumTitle(album)
            .setArtworkUri(artworkUrl?.let { Uri.parse(it) })
            .build()
        val item = MediaItem.Builder()
            .setUri(proxyUrl)
            .setMediaMetadata(metadata)
            .build()
        player?.apply {
            setMediaItem(item)
            prepare()
            play()
        }
    }

    override fun onDestroy() {
        unregisterReceiver(noisyReceiver)
        mediaSession?.run {
            player.release()
            release()
            mediaSession = null
        }
        player = null
        super.onDestroy()
    }

    companion object {
        init {
            System.loadLibrary("resonance")
        }

        @JvmStatic
        external fun nativeOnMediaCommand(opcode: Int, argMs: Long)

        /**
         * Configures an Android WebView for Google Sign-In with `; wv` stripped
         * to avoid `403: disallowed_useragent`, captures YouTube Music cookies, and
         * destroys the WebView immediately once `SAPISID` / `__Secure-3PAPISID` arrive.
         */
        @SuppressLint("SetJavaScriptEnabled")
        fun configureGoogleSignInWebView(
            webView: WebView,
            onCookiesCaptured: (cookieHeader: String) -> Unit,
        ) {
            val settings: WebSettings = webView.settings
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            // Protocol Rule #3: Strip `; wv` and `Version/X.Y` from Android WebView UA
            val sanitizedUa = settings.userAgentString
                .replace("; wv)", ")")
                .replace("; wv;", ";")
                .replace("; wv", "")
                .replace(Regex("Version/\\S+\\s*"), "")
            settings.userAgentString = sanitizedUa

            webView.webViewClient = object : WebViewClient() {
                override fun onPageFinished(view: WebView, url: String?) {
                    if (url != null && url.startsWith("https://music.youtube.com")) {
                        val cookies = CookieManager.getInstance().getCookie(url).orEmpty()
                        if (cookies.contains("SAPISID=") || cookies.contains("__Secure-3PAPISID=")) {
                            onCookiesCaptured(cookies)
                            // Destroy WebView immediately after storing credentials in Keystore
                            view.stopLoading()
                            view.loadUrl("about:blank")
                            view.destroy()
                        }
                    }
                }
            }
            webView.loadUrl("https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F")
        }
    }
}
