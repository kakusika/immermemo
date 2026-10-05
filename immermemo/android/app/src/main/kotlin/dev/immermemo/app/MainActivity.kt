package dev.immermemo.app

import android.app.NativeActivity
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import java.util.concurrent.atomic.AtomicBoolean

/**
 * [NativeActivity] hands the window's `Surface` straight to native code
 * (`takeSurface`) before any native code has run, bypassing the normal
 * DecorView (and its themed background) entirely -- so without this,
 * the system splash is dismissed immediately on activity start and the
 * OS background (the launcher) briefly shows through the still-empty
 * surface until Slint's first frame lands.
 *
 * This subclass holds the splash open past that point via
 * [androidx.core.splashscreen.SplashScreen]'s `setKeepOnScreenCondition`,
 * released by [markFirstFrameRendered] -- called over JNI from
 * `src/android/splash.rs` once Slint's renderer reports its first
 * frame (see that file for the Rust side).
 */
class MainActivity : NativeActivity() {
    private val firstFrameRendered = AtomicBoolean(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        val splashScreen = installSplashScreen()
        splashScreen.setKeepOnScreenCondition { !firstFrameRendered.get() }
        // Safety net in case the JNI signal never arrives (a renderer
        // backend change, a crash before the first frame, ...). Kept
        // short: a real first frame should land in well under this on
        // any real device, and this bound is what the user actually
        // waits through on that failure path, so it must stay small.
        Handler(Looper.getMainLooper()).postDelayed(
            { firstFrameRendered.set(true) },
            SPLASH_TIMEOUT_MS,
        )
        super.onCreate(savedInstanceState)
    }

    @Suppress("unused")
    fun markFirstFrameRendered() {
        firstFrameRendered.set(true)
    }

    companion object {
        private const val SPLASH_TIMEOUT_MS = 500L
    }
}
