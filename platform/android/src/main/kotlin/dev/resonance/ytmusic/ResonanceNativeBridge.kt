package dev.resonance.ytmusic

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import androidx.annotation.Keep
import androidx.core.content.ContextCompat
import java.lang.ref.WeakReference

/**
 * Centralized, R8-safe bidirectional JNI facade (`@Keep`) between `libresonance.so`
 * and Android UI / Media3 `PlaybackForegroundService` / `KeystoreBridge`.
 */
@Keep
object ResonanceNativeBridge {
    private data class PendingLoadAndPlay(
        val url: String,
        val title: String,
        val artist: String,
        val album: String?,
        val artworkUrl: String?,
        val durationMs: Long,
        val generation: Long,
    )

    @Volatile
    private var appContext: Context? = null

    @Volatile
    private var serviceRef: WeakReference<PlaybackForegroundService>? = null

    @Volatile
    private var activityRef: WeakReference<MainActivity>? = null

    private val mainHandler = Handler(Looper.getMainLooper())
    private val pendingLock = Any()
    private var pendingLoad: PendingLoadAndPlay? = null

    fun initHost(context: Context) {
        appContext = context.applicationContext
    }

    fun registerActivity(activity: MainActivity) {
        appContext = activity.applicationContext
        activityRef = WeakReference(activity)
    }

    fun unregisterActivity(activity: MainActivity) {
        if (activityRef?.get() === activity) {
            activityRef = null
        }
    }

    fun registerService(service: PlaybackForegroundService) {
        appContext = service.applicationContext
        serviceRef = WeakReference(service)
        val pending = synchronized(pendingLock) {
            val item = pendingLoad
            pendingLoad = null
            item
        }
        if (pending != null) {
            mainHandler.post {
                service.playFromLoopbackProxy(
                    pending.url,
                    pending.title,
                    pending.artist,
                    pending.album,
                    pending.artworkUrl,
                    pending.durationMs,
                    pending.generation,
                )
            }
        }
    }

    fun unregisterService(service: PlaybackForegroundService) {
        if (serviceRef?.get() === service) {
            serviceRef = null
        }
    }

    internal fun isLoopbackUrl(rawUrl: String): Boolean {
        val uri = runCatching { Uri.parse(rawUrl) }.getOrNull() ?: return false
        return uri.scheme == "http" && uri.host in setOf("127.0.0.1", "localhost")
    }

    @JvmStatic
    fun audioLoadAndPlay(
        url: String,
        title: String,
        artist: String,
        album: String,
        artworkUrl: String,
        durationMs: Long,
        generation: Long,
    ) {
        if (!isLoopbackUrl(url)) {
            nativeOnPlaybackState(generation, -1, "Rejected non-loopback stream URL")
            return
        }
        val cleanAlbum = album.ifEmpty { null }
        val cleanArtwork = artworkUrl.ifEmpty { null }
        mainHandler.post {
            val service = serviceRef?.get()
            if (service != null) {
                service.playFromLoopbackProxy(
                    url,
                    title,
                    artist,
                    cleanAlbum,
                    cleanArtwork,
                    durationMs,
                    generation,
                )
            } else {
                synchronized(pendingLock) {
                    pendingLoad = PendingLoadAndPlay(
                        url = url,
                        title = title,
                        artist = artist,
                        album = cleanAlbum,
                        artworkUrl = cleanArtwork,
                        durationMs = durationMs,
                        generation = generation,
                    )
                }
                val ctx = appContext ?: return@post
                ContextCompat.startForegroundService(
                    ctx,
                    Intent(ctx, PlaybackForegroundService::class.java),
                )
            }
        }
    }

    @JvmStatic
    fun audioPlay() {
        mainHandler.post { serviceRef?.get()?.resumePlayback() }
    }

    @JvmStatic
    fun audioPause() {
        mainHandler.post { serviceRef?.get()?.pausePlayback() }
    }

    @JvmStatic
    fun audioStop() {
        mainHandler.post { serviceRef?.get()?.stopPlayback() }
    }

    @JvmStatic
    fun audioSeekMs(positionMs: Long) {
        mainHandler.post { serviceRef?.get()?.seekToMs(positionMs) }
    }

    @JvmStatic
    fun audioSetVolume(volume: Float) {
        mainHandler.post { serviceRef?.get()?.setVolume(volume) }
    }

    @JvmStatic
    fun audioUpdateMetadata(
        title: String,
        artist: String,
        album: String,
        artworkUrl: String,
        playing: Boolean,
        positionMs: Long,
    ) {
        val cleanAlbum = album.ifEmpty { null }
        val cleanArtwork = artworkUrl.ifEmpty { null }
        mainHandler.post {
            serviceRef?.get()?.updateNowPlayingMetadata(
                title,
                artist,
                cleanAlbum,
                cleanArtwork,
                playing,
                positionMs,
            )
        }
    }

    @JvmStatic
    fun keystoreWrite(key: String, plaintext: String): Boolean {
        val ctx = appContext ?: return false
        return KeystoreBridge.writeSecret(ctx, key, plaintext)
    }

    @JvmStatic
    fun keystoreRead(key: String): String? {
        val ctx = appContext ?: return null
        return KeystoreBridge.readSecret(ctx, key)
    }

    @JvmStatic
    fun keystoreDelete(key: String) {
        val ctx = appContext ?: return
        KeystoreBridge.deleteSecret(ctx, key)
    }

    @JvmStatic
    fun launchSignInWebView() {
        mainHandler.post { activityRef?.get()?.launchGoogleSignInWebView() }
    }

    @JvmStatic
    external fun nativeInitHost(
        bridgeClass: Class<ResonanceNativeBridge>,
        filesDir: String,
        cacheDir: String,
    )

    @JvmStatic
    external fun nativeOnMediaCommand(opcode: Int, argMs: Long)

    @JvmStatic
    external fun nativeOnPlaybackProgress(generation: Long, positionMs: Long, durationMs: Long)

    @JvmStatic
    external fun nativeOnPlaybackState(generation: Long, stateCode: Int, errorMsg: String?)

    @JvmStatic
    external fun nativeOnAuthCookiesCaptured(cookieHeader: String)

    init {
        runCatching { System.loadLibrary("resonance") }
    }
}
