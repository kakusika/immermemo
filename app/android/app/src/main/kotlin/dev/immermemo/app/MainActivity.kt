package dev.immermemo.app

import android.app.NativeActivity
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
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
 *
 * Also exposes [setStatusBarVisible], called over JNI from
 * `src/android/system_ui.rs`, for the Settings toggle that hides the
 * status bar while editing a note.
 */
class MainActivity : NativeActivity() {
    private val firstFrameRendered = AtomicBoolean(false)

    // The last state [setStatusBarVisible] was asked for -- distinct
    // from the system's own current flags, which a swipe-to-reveal
    // gesture changes out from under us. Read by the system-UI-visibility
    // listener below to tell "the user just revealed it over a hidden
    // request" (re-hide) apart from "it's visible because it's meant to
    // be" (leave alone).
    private var statusBarShouldBeVisible = true

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
        // SYSTEM_UI_FLAG_IMMERSIVE_STICKY (applied in
        // applyStatusBarVisibility) already makes a swipe-reveal
        // transient and auto-hide again on its own, but re-asserting
        // here too is cheap insurance against an OEM skin that doesn't
        // honor that for a plain NativeActivity.
        @Suppress("DEPRECATION")
        window.decorView.setOnSystemUiVisibilityChangeListener { flags ->
            val systemShowsIt = (flags and View.SYSTEM_UI_FLAG_FULLSCREEN) == 0
            if (!statusBarShouldBeVisible && systemShowsIt) {
                applyStatusBarVisibility(false)
            }
        }
        super.onCreate(savedInstanceState)
    }

    @Suppress("unused")
    fun markFirstFrameRendered() {
        firstFrameRendered.set(true)
    }

    /**
     * Called over JNI from `src/android/system_ui.rs` to show/hide the
     * status bar -- used to free up screen space while editing a note,
     * when the user opts into that in Settings. View mutations must
     * happen on the UI thread, which this JNI call doesn't originate
     * from (Slint's Android backend runs its own event loop thread).
     */
    @Suppress("unused")
    fun setStatusBarVisible(visible: Boolean) {
        runOnUiThread {
            statusBarShouldBeVisible = visible
            applyStatusBarVisibility(visible)
        }
    }

    @Suppress("DEPRECATION")
    private fun applyStatusBarVisibility(visible: Boolean) {
        window.decorView.systemUiVisibility =
            if (visible) {
                View.SYSTEM_UI_FLAG_LAYOUT_STABLE
            } else {
                // IMMERSIVE_STICKY: without it, a swipe-to-reveal clears
                // FULLSCREEN permanently instead of just temporarily --
                // the bug this is fixing (status bar stays shown forever
                // after the first swipe once "Always Hide" is set).
                View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
                    View.SYSTEM_UI_FLAG_FULLSCREEN or
                    View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or
                    View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
            }
    }

    companion object {
        private const val SPLASH_TIMEOUT_MS = 500L
    }
}
