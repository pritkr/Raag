package dev.resonance.ytmusic

import android.net.Uri
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata

/**
 * Car-safe 2-level Android Auto Media3 browse tree.
 *
 * Root tabs (<= 4 for Android Auto glanceability):
 * - "tab_home": Home Quick Picks & Recent Mixes
 * - "tab_downloads": Offline Vault & Completed Downloads (100% offline-first)
 * - "tab_library": Liked Songs + User Playlists + Saved Albums
 * - "tab_queue": Active Playback Queue
 *
 * Every MediaItem explicitly sets `.setIsBrowsable(...)` and `.setIsPlayable(...)`
 * because Android Auto silently drops items missing either flag.
 */
object AndroidAutoBrowseTree {
    const val ROOT_ID = "root"
    const val TAB_HOME_ID = "tab_home"
    const val TAB_DOWNLOADS_ID = "tab_downloads"
    const val TAB_LIBRARY_ID = "tab_library"
    const val TAB_QUEUE_ID = "tab_queue"
    const val MAX_ITEMS_PER_PAGE = 100

    fun rootItem(): MediaItem {
        val meta = MediaMetadata.Builder()
            .setTitle("Raag")
            .setIsBrowsable(true)
            .setIsPlayable(false)
            .setMediaType(MediaMetadata.MEDIA_TYPE_FOLDER_MIXED)
            .build()
        return MediaItem.Builder()
            .setMediaId(ROOT_ID)
            .setMediaMetadata(meta)
            .build()
    }

    fun topLevelTabs(): List<MediaItem> = listOf(
        browsableFolder(TAB_DOWNLOADS_ID, "Downloads", MediaMetadata.MEDIA_TYPE_FOLDER_ALBUMS),
        browsableFolder(TAB_HOME_ID, "Home", MediaMetadata.MEDIA_TYPE_FOLDER_MIXED),
        browsableFolder(TAB_LIBRARY_ID, "Library", MediaMetadata.MEDIA_TYPE_FOLDER_PLAYLISTS),
        browsableFolder(TAB_QUEUE_ID, "Queue", MediaMetadata.MEDIA_TYPE_FOLDER_MIXED),
    )

    fun browsableFolder(
        mediaId: String,
        title: String,
        mediaType: Int = MediaMetadata.MEDIA_TYPE_FOLDER_MIXED,
    ): MediaItem {
        val meta = MediaMetadata.Builder()
            .setTitle(title)
            .setIsBrowsable(true)
            .setIsPlayable(false)
            .setMediaType(mediaType)
            .build()
        return MediaItem.Builder()
            .setMediaId(mediaId)
            .setMediaMetadata(meta)
            .build()
    }

    fun playableTrack(
        videoId: String,
        title: String,
        artist: String,
        album: String? = null,
        artworkUri: Uri? = null,
        loopbackUri: Uri? = null,
    ): MediaItem {
        val meta = MediaMetadata.Builder()
            .setTitle(title)
            .setArtist(artist)
            .setAlbumTitle(album)
            .setArtworkUri(artworkUri)
            .setIsBrowsable(false)
            .setIsPlayable(true)
            .setMediaType(MediaMetadata.MEDIA_TYPE_MUSIC)
            .build()
        val builder = MediaItem.Builder()
            .setMediaId("track:$videoId")
            .setMediaMetadata(meta)
        if (loopbackUri != null && loopbackUri.host in setOf("127.0.0.1", "localhost")) {
            builder.setUri(loopbackUri)
        }
        return builder.build()
    }

    fun paginate(items: List<MediaItem>, page: Int, pageSize: Int): List<MediaItem> {
        if (items.isEmpty() || page < 0 || pageSize <= 0) return emptyList()
        val boundedSize = pageSize.coerceAtMost(MAX_ITEMS_PER_PAGE)
        val from = (page * boundedSize).coerceAtMost(items.size)
        val to = (from + boundedSize).coerceAtMost(items.size)
        return items.subList(from, to)
    }

    /**
     * Offline-safe fallback search for Google Assistant / Android Auto voice queries
     * when cellular connectivity drops.
     */
    fun offlineFallbackSearch(query: String, candidates: List<MediaItem>): List<MediaItem> {
        val q = query.trim().lowercase()
        if (q.isEmpty()) return candidates.take(MAX_ITEMS_PER_PAGE)
        return candidates.filter { item ->
            val meta = item.mediaMetadata
            val title = meta.title?.toString()?.lowercase().orEmpty()
            val artist = meta.artist?.toString()?.lowercase().orEmpty()
            val album = meta.albumTitle?.toString()?.lowercase().orEmpty()
            title.contains(q) || artist.contains(q) || album.contains(q)
        }.take(MAX_ITEMS_PER_PAGE)
    }
}
