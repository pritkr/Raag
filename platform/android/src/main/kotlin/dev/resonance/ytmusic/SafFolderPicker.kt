package dev.resonance.ytmusic

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.annotation.Keep

/**
 * Android Storage Access Framework (`ACTION_OPEN_DOCUMENT_TREE`) helper (`P4`)
 * that requests read-only persistable URI permissions for user-selected local music folders.
 */
@Keep
object SafFolderPicker {
    const val REQUEST_CODE_OPEN_MUSIC_TREE = 4401

    @JvmStatic
    fun launchFolderPicker(activity: Activity) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
        }
        activity.startActivityForResult(intent, REQUEST_CODE_OPEN_MUSIC_TREE)
    }

    @JvmStatic
    fun handleActivityResult(activity: Activity, requestCode: Int, resultCode: Int, data: Intent?): Boolean {
        if (requestCode != REQUEST_CODE_OPEN_MUSIC_TREE) return false
        if (resultCode != Activity.RESULT_OK) return true
        val uri: Uri = data?.data ?: return true
        val takeFlags = Intent.FLAG_GRANT_READ_URI_PERMISSION
        runCatching {
            activity.contentResolver.takePersistableUriPermission(uri, takeFlags)
        }
        ResonanceNativeBridge.nativeOnLocalFolderPicked(uri.toString())
        return true
    }
}
