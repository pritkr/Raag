package dev.resonance.ytmusic

import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.webkit.CookieManager
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.annotation.OptIn
import androidx.core.content.ContextCompat
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * Thin Android Media Lifecycle Bridge required by Phase 0 & ST-4.
 *
 * Wraps Media3 ExoPlayer + ForwardingPlayer + MediaSessionService + AudioFocus +
 * ACTION_AUDIO_BECOMING_NOISY (`RECEIVER_NOT_EXPORTED`) so playback survives screen lock,
 * backgrounding, Bluetooth AVRCP, and headphone unplug.
 * Streams strictly from the in-process Rust Loopback HTTP Proxy (`http://127.0.0.1:<port>/stream/...`).
 */
@OptIn(UnstableApi::class)
class PlaybackForegroundService : MediaSessionService() {

    private var player: ExoPlayer? = null
    private var mediaSession: MediaSession? = null

    @Volatile
    private var currentGeneration: Long = 0L

    private val progressHandler = Handler(Looper.getMainLooper())
    private val progressTicker = object : Runnable {
        override fun run() {
            val active = player ?: return
            if (active.isPlaying) {
                ResonanceNativeBridge.nativeOnPlaybackProgress(
                    currentGeneration,
                    active.currentPosition.coerceAtLeast(0L),
                    active.duration.coerceAtLeast(0L),
                )
                progressHandler.postDelayed(this, PROGRESS_INTERVAL_MS)
            }
        }
    }

    private val noisyReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                player?.pause()
                ResonanceNativeBridge.nativeOnMediaCommand(7, 0L) // MediaCommand::BecomingNoisy
            }
        }
    }

    private val playerListener = object : Player.Listener {
        override fun onPlaybackStateChanged(playbackState: Int) {
            val stateCode = when (playbackState) {
                Player.STATE_IDLE -> 1
                Player.STATE_BUFFERING -> 2
                Player.STATE_READY -> 3
                Player.STATE_ENDED -> {
                    stopProgressTicker()
                    4
                }
                else -> return
            }
            ResonanceNativeBridge.nativeOnPlaybackState(currentGeneration, stateCode, null)
        }

        override fun onIsPlayingChanged(isPlaying: Boolean) {
            if (isPlaying) {
                startProgressTicker()
            } else {
                stopProgressTicker()
            }
            ResonanceNativeBridge.nativeOnPlaybackState(
                currentGeneration,
                if (isPlaying) 5 else 6,
                null,
            )
        }

        override fun onPlayerError(error: PlaybackException) {
            stopProgressTicker()
            ResonanceNativeBridge.nativeOnPlaybackState(
                currentGeneration,
                -1,
                redactErrorMessage(error.message),
            )
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
        exo.addListener(playerListener)

        val forwardingPlayer = object : ForwardingPlayer(exo) {
            override fun getAvailableCommands(): Player.Commands {
                return super.getAvailableCommands().buildUpon()
                    .add(COMMAND_SEEK_TO_NEXT)
                    .add(COMMAND_SEEK_TO_PREVIOUS)
                    .add(COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
                    .add(COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
                    .add(COMMAND_PLAY_PAUSE)
                    .add(COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM)
                    .add(COMMAND_STOP)
                    .build()
            }

            override fun play() {
                ResonanceNativeBridge.nativeOnMediaCommand(1, 0L)
            }

            override fun pause() {
                ResonanceNativeBridge.nativeOnMediaCommand(2, 0L)
            }

            override fun stop() {
                ResonanceNativeBridge.nativeOnMediaCommand(10, 0L)
            }

            override fun seekToNext() {
                ResonanceNativeBridge.nativeOnMediaCommand(4, 0L)
            }

            override fun seekToNextMediaItem() {
                ResonanceNativeBridge.nativeOnMediaCommand(4, 0L)
            }

            override fun seekToPrevious() {
                ResonanceNativeBridge.nativeOnMediaCommand(5, 0L)
            }

            override fun seekToPreviousMediaItem() {
                ResonanceNativeBridge.nativeOnMediaCommand(5, 0L)
            }

            override fun seekTo(positionMs: Long) {
                ResonanceNativeBridge.nativeOnMediaCommand(6, positionMs.coerceAtLeast(0L))
            }
        }

        player = exo
        mediaSession = MediaSession.Builder(this, forwardingPlayer).build()

        ContextCompat.registerReceiver(
            this,
            noisyReceiver,
            IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )

        ResonanceNativeBridge.registerService(this)
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        val activePlayer = player
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
        durationMs: Long = 0L,
        generation: Long = currentGeneration + 1L,
    ) {
        val uri = runCatching { Uri.parse(proxyUrl) }.getOrNull()
        if (uri == null || uri.scheme != "http" || uri.host !in setOf("127.0.0.1", "localhost")) {
            ResonanceNativeBridge.nativeOnPlaybackState(
                generation,
                -1,
                "Refused non-loopback stream URL",
            )
            return
        }
        currentGeneration = generation

        val metadata = buildMediaMetadata(title, artist, album, artworkUrl)
        val item = MediaItem.Builder()
            .setUri(uri)
            .setMediaMetadata(metadata)
            .build()
        player?.apply {
            setMediaItem(item)
            prepare()
            play()
        }
    }

    fun resumePlayback() {
        player?.play()
    }

    fun pausePlayback() {
        player?.pause()
    }

    fun stopPlayback() {
        stopProgressTicker()
        player?.stop()
        player?.clearMediaItems()
    }

    fun seekToMs(positionMs: Long) {
        player?.seekTo(positionMs.coerceAtLeast(0L))
    }

    fun setVolume(volume: Float) {
        player?.volume = volume.coerceIn(0f, 1f)
    }

    fun updateNowPlayingMetadata(
        title: String,
        artist: String,
        album: String?,
        artworkUrl: String?,
        playing: Boolean,
        positionMs: Long,
    ) {
        val exo = player ?: return
        val current = exo.currentMediaItem
        if (current != null) {
            val updated = current.buildUpon()
                .setMediaMetadata(buildMediaMetadata(title, artist, album, artworkUrl))
                .build()
            exo.replaceMediaItem(exo.currentMediaItemIndex, updated)
        }
        if (kotlin.math.abs(exo.currentPosition - positionMs) > 1500L) {
            exo.seekTo(positionMs.coerceAtLeast(0L))
        }
        if (playing && !exo.isPlaying) {
            exo.play()
        } else if (!playing && exo.isPlaying) {
            exo.pause()
        }
    }

    private fun buildMediaMetadata(
        title: String,
        artist: String,
        album: String?,
        artworkUrl: String?,
    ): MediaMetadata {
        return MediaMetadata.Builder()
            .setTitle(title)
            .setArtist(artist)
            .setAlbumTitle(album)
            .setArtworkUri(artworkUrl?.let { Uri.parse(it) })
            .build()
    }

    private fun startProgressTicker() {
        progressHandler.removeCallbacks(progressTicker)
        progressHandler.postDelayed(progressTicker, PROGRESS_INTERVAL_MS)
    }

    private fun stopProgressTicker() {
        progressHandler.removeCallbacks(progressTicker)
    }

    override fun onDestroy() {
        stopProgressTicker()
        ResonanceNativeBridge.unregisterService(this)
        unregisterReceiver(noisyReceiver)
        mediaSession?.run {
            release()
            mediaSession = null
        }
        player?.run {
            removeListener(playerListener)
            release()
        }
        player = null
        super.onDestroy()
    }

    companion object {
        private const val PROGRESS_INTERVAL_MS = 500L

        init {
            runCatching { System.loadLibrary("resonance") }
        }

        private fun redactErrorMessage(raw: String?): String {
            if (raw.isNullOrBlank()) return "ExoPlayer playback error"
            return raw
                .replace(Regex("https?://\\S+"), "[REDACTED_URL]")
                .replace(Regex("(SAPISID|__Secure-3PAPISID|Authorization)=\\S+"), "$1=[REDACTED]")
        }

        /**
         * Configures an Android WebView for Google Sign-In with `; wv` stripped
         * to avoid `403: disallowed_useragent`, captures YouTube Music cookies strictly
         * from `https://music.youtube.com`, and destroys the WebView immediately once
         * `SAPISID` / `__Secure-3PAPISID` arrive.
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
                    val uri = url?.let { runCatching { Uri.parse(it) }.getOrNull() } ?: return
                    if (uri.scheme == "https" && uri.host == "music.youtube.com") {
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
            webView.loadUrl(
                "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F",
            )
        }
    }
}
