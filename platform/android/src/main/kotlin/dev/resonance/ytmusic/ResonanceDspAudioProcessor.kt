package dev.resonance.ytmusic

import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.BaseAudioProcessor
import androidx.media3.common.util.UnstableApi
import java.nio.ByteBuffer

/**
 * Zero-copy Media3 [BaseAudioProcessor] routing decoded PCM direct [ByteBuffer]s
 * straight into Raag's Rust `DspPipeline` (10-band EQ, ReplayGain, Silence Skip)
 * without JVM byte-array copies.
 */
@OptIn(UnstableApi::class)
class ResonanceDspAudioProcessor : BaseAudioProcessor() {

    private var dspHandle: Long = 0L
    private var isFloatPcm: Boolean = false

    override fun onConfigure(
        inputAudioFormat: AudioProcessor.AudioFormat
    ): AudioProcessor.AudioFormat {
        val encoding = inputAudioFormat.encoding
        if (encoding != C.ENCODING_PCM_16BIT && encoding != C.ENCODING_PCM_FLOAT) {
            return AudioProcessor.AudioFormat.NOT_SET
        }
        isFloatPcm = encoding == C.ENCODING_PCM_FLOAT
        if (dspHandle != 0L) {
            nativeDestroyDspHandle(dspHandle)
            dspHandle = 0L
        }
        dspHandle = nativeCreateDspHandle(
            inputAudioFormat.sampleRate,
            inputAudioFormat.channelCount
        )
        return inputAudioFormat
    }

    override fun queueInput(inputBuffer: ByteBuffer) {
        val remaining = inputBuffer.remaining()
        if (remaining <= 0) return

        val out = replaceOutputBuffer(remaining)
        if (dspHandle != 0L && inputBuffer.isDirect && out.isDirect) {
            val keep = nativeProcessPcm(
                dspHandle,
                inputBuffer,
                inputBuffer.position(),
                remaining,
                out,
                isFloatPcm
            )
            inputBuffer.position(inputBuffer.limit())
            if (keep) {
                out.position(0)
                out.limit(remaining)
            } else {
                // Silence-skipped block: emit 0-byte output cleanly.
                out.position(0)
                out.limit(0)
            }
        } else {
            out.put(inputBuffer)
            out.flip()
        }
    }

    fun updateDspConfig(
        loudnessEnabled: Boolean,
        silenceSkipEnabled: Boolean,
        eqGainsDb: FloatArray,
        trackLoudnessDb: Float?
    ) {
        if (dspHandle != 0L) {
            nativeUpdateDspConfig(
                dspHandle,
                loudnessEnabled,
                silenceSkipEnabled,
                eqGainsDb,
                trackLoudnessDb ?: Float.NaN
            )
        }
    }

    override fun onReset() {
        if (dspHandle != 0L) {
            nativeDestroyDspHandle(dspHandle)
            dspHandle = 0L
        }
    }

    companion object {
        @JvmStatic
        external fun nativeCreateDspHandle(sampleRate: Int, channelCount: Int): Long

        @JvmStatic
        external fun nativeProcessPcm(
            handle: Long,
            inputBuffer: ByteBuffer,
            inputOffset: Int,
            byteLength: Int,
            outputBuffer: ByteBuffer,
            isFloat: Boolean
        ): Boolean

        @JvmStatic
        external fun nativeUpdateDspConfig(
            handle: Long,
            loudnessEnabled: Boolean,
            silenceSkipEnabled: Boolean,
            eqGainsDb: FloatArray,
            trackLoudnessDb: Float
        )

        @JvmStatic
        external fun nativeDestroyDspHandle(handle: Long)
    }
}
