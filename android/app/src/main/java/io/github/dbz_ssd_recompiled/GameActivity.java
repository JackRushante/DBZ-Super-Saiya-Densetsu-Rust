package io.github.dbz_ssd_recompiled;

import android.os.Build;
import android.os.Bundle;
import android.view.Display;
import android.view.HapticFeedbackConstants;
import android.view.Surface;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

import org.libsdl.app.SDLActivity;

/**
 * The game's activity: SDL's activity, which loads libSDL3.so and libmain.so and calls SDL_main.
 */
public class GameActivity extends SDLActivity {
    /** A light haptic tick under the finger (the touch controls); sent by the native side. */
    private static final int HAPTIC = 0x8002;
    /** The game's frame rate (the SNES's 60.1, which a 60 Hz screen shows). */
    private static final float GAME_RATE = 60f;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // The screen stays on while the game is in front, and the picture reaches the edges (cutouts included).
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams attrs = getWindow().getAttributes();
            attrs.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            getWindow().setAttributes(attrs);
        }
        hideSystemBars();
        requestRefreshRate();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
            // Again: a fold or an unfold is another display, with its own modes.
            requestRefreshRate();
        }
    }

    /** Immersive full screen: the system bars are hidden and come back with a swipe from the edge. */
    private void hideSystemBars() {
        Window window = getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            // The flags below are ignored by recent systems; the insets controller is their replacement.
            window.setDecorFitsSystemWindows(false);
            WindowInsetsController controller = window.getInsetsController();
            if (controller != null) {
                controller.hide(WindowInsets.Type.statusBars() | WindowInsets.Type.navigationBars());
                controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
            return;
        }
        window.getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                        | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                        | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_FULLSCREEN);
    }

    /**
     * Asks the screen for a refresh rate the game's 60 frames fit: the lowest mode of the current resolution that is a
     * multiple of 60 Hz, and the surface's frame rate. An adaptive (LTPO) panel otherwise drops to 24 Hz or so while
     * nothing touches it, and the game, whose frames wait for the screen, slows down with it (and its sound breaks up).
     */
    private void requestRefreshRate() {
        Display display = getWindowManager().getDefaultDisplay();
        Display.Mode current = display.getMode();
        Display.Mode best = null;
        for (Display.Mode mode : display.getSupportedModes()) {
            if (mode.getPhysicalWidth() != current.getPhysicalWidth()
                    || mode.getPhysicalHeight() != current.getPhysicalHeight()) {
                continue;
            }
            float multiple = mode.getRefreshRate() / GAME_RATE;
            boolean fits = multiple >= 0.98f && Math.abs(multiple - Math.round(multiple)) < 0.02f;
            if (fits && (best == null || mode.getRefreshRate() < best.getRefreshRate())) {
                best = mode;
            }
        }
        WindowManager.LayoutParams attrs = getWindow().getAttributes();
        if (best != null) {
            attrs.preferredDisplayModeId = best.getModeId();
        }
        attrs.preferredRefreshRate = best != null ? best.getRefreshRate() : GAME_RATE;
        getWindow().setAttributes(attrs);
        if (Build.VERSION.SDK_INT >= 30 && mSurface != null) {
            Surface surface = mSurface.getHolder().getSurface();
            if (surface != null && surface.isValid()) {
                surface.setFrameRate(GAME_RATE, Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE);
            }
        }
    }

    @Override
    protected boolean onUnhandledMessage(int command, Object param) {
        if (command == HAPTIC) {
            // The system's keyboard-tap feedback: light, and it needs no permission.
            getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP);
            return true;
        }
        return false;
    }
}
