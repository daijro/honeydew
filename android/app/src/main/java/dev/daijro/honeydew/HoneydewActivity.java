package dev.daijro.honeydew;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.os.Build;
import android.os.Bundle;
import android.util.DisplayMetrics;
import android.view.DisplayCutout;
import android.view.HapticFeedbackConstants;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

/**
 * Slim NativeActivity coordinator and JNI facade for Honeydew.
 * Delegates audio routing, Material 3 dialogs, SAF storage, and HTTPS networking
 * to focused, lifecycle-safe collaborators.
 */
public final class HoneydewActivity extends NativeActivity {
    static { System.loadLibrary("honeydew"); }

    private static native void onTextResult(long requestId, String text, boolean accepted);
    private static native void onDisplayDpi(float dpi);
    private static native void onCutoutInsets(int left, int top, int right, int bottom);
    private static native void onDocumentPicked(String path, String error);
    private static native void onFolderPicked(String files, String error);
    private static native void onFilesCopied(String error);
    private static native void onMic(boolean granted);
    private static native void onHeadphones(boolean on);
    private static native void onInterrupted();
    private static native void onLeaving();
    private static native void onBack();

    private HoneydewAudio audio;
    private HoneydewDialogs dialogs;
    private HoneydewStorage storage;
    private HoneydewNetwork network;

    // Stored as Object so ART on API 26-32 never verifies the API 33 interface as a field type.
    private Object backCallback;
    private int lastCutoutLeft = -1;
    private int lastCutoutTop = -1;
    private int lastCutoutRight = -1;
    private int lastCutoutBottom = -1;

    @Override
    protected void onCreate(Bundle state) {
        // Publish physical density before NativeActivity starts the Rust app thread.
        onDisplayDpi(resolveDisplayDpi());
        super.onCreate(state);

        dialogs = new HoneydewDialogs(this, HoneydewActivity::onTextResult);
        audio = new HoneydewAudio(this, new HoneydewAudio.Listener() {
            @Override
            public void onHeadphonesChanged(boolean connected) {
                onHeadphones(connected);
            }

            @Override
            public void onInterrupted() {
                HoneydewActivity.onInterrupted();
            }
        });
        storage = new HoneydewStorage(this, new HoneydewStorage.Listener() {
            @Override
            public void onDocumentPicked(String path, String error) {
                HoneydewActivity.onDocumentPicked(path, error);
            }

            @Override
            public void onFolderPicked(String files, String error) {
                HoneydewActivity.onFolderPicked(files, error);
            }

            @Override
            public void onFilesCopied(String error) {
                HoneydewActivity.onFilesCopied(error);
            }

            @Override
            public void onShowFolderFallback(String path) {
                dialogs.showFolderFallback(path);
            }
        });
        network = new HoneydewNetwork(this);

        hideSystemBars();
        installCutoutListener();
        audio.start();

        // Register predictive back callback on Android 13+ and keep a handle to unregister in onDestroy().
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            android.window.OnBackInvokedCallback callback = HoneydewActivity::onBack;
            backCallback = callback;
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback);
        }
    }

    @Override
    public void onAttachedToWindow() {
        super.onAttachedToWindow();
        requestCutoutRefresh();
    }

    @Override
    protected void onPause() {
        onLeaving();
        super.onPause();
        onInterrupted();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
            requestCutoutRefresh();
        }
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        onDisplayDpi(resolveDisplayDpi());
        requestCutoutRefresh();
    }

    @Override
    public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        if (request != HoneydewAudio.REQUEST_RECORD_AUDIO) {
            super.onRequestPermissionsResult(request, permissions, results);
            return;
        }
        onMic(results != null && results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED);
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        if (storage != null && storage.handleActivityResult(request, result, data)) {
            return;
        }
        super.onActivityResult(request, result, data);
    }

    @Override
    protected void onDestroy() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && backCallback != null) {
            getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(
                    (android.window.OnBackInvokedCallback) backCallback);
            backCallback = null;
        }
        if (dialogs != null) dialogs.dismissAll();
        if (audio != null) audio.release();
        if (storage != null) storage.shutdown();
        super.onDestroy();
        onCutoutInsets(0, 0, 0, 0);
        onDisplayDpi(0f);
    }

    // =========================================================================
    // JNI entry points called from Rust (src/android.rs & src/android_text.rs)
    // =========================================================================

    public String inputs() {
        return audio == null ? "" : audio.listInputs();
    }

    public boolean askMic() {
        return audio != null && audio.askMicPermission();
    }

    public void recording(boolean on) {
        if (audio != null) audio.setRecordingFocus(on);
    }

    public void requestOrientation(final int orientation) {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) return;
            setRequestedOrientation(orientation);
        });
    }

    public void editText(long requestId, String text, boolean multiline) {
        editText(requestId, text, multiline, 0);
    }

    public void editText(long requestId, String text, boolean multiline, int limit) {
        if (dialogs != null) {
            dialogs.showTextEditor(requestId, text, multiline, limit);
        } else {
            onTextResult(requestId, text == null ? "" : text, false);
        }
    }

    public void cancelTextEdit(long requestId) {
        if (dialogs != null) dialogs.cancelTextEditor(requestId);
    }

    public void openDocument(long maxBytes) {
        if (storage != null) storage.openDocument(maxBytes);
    }

    public void openFolder() {
        if (storage != null) storage.openFolder();
    }

    public void copyFiles(String files) {
        if (storage != null) storage.copyFiles(files);
    }

    public void showFolder(String path) {
        if (storage != null) storage.showFolder(path);
    }

    public String share(String paths, String type) {
        return storage == null ? "Activity unavailable" : storage.share(paths, type);
    }

    public int network() {
        return network == null ? 0 : network.networkState();
    }

    public String download(String url, String path) {
        return network == null ? "Activity unavailable" : network.download(url, path);
    }

    public long usableSpace() {
        return storage == null ? -1L : storage.usableSpace();
    }

    public boolean installApk(String name) {
        return network != null && network.installApk(name);
    }

    /** Cleanly exits Honeydew and removes its task from Recents (issue #147). */
    public void quitApp() {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) return;
            finishAndRemoveTask();
        });
    }

    /** Keeps the screen awake while playback or recording is active. */
    public void setKeepScreenOn(boolean on) {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) return;
            Window window = getWindow();
            if (window == null) return;
            if (on) {
                window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            } else {
                window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            }
        });
    }

    /** Triggers a subtle native Android tactile tick for key UI confirmations. */
    public void hapticTick() {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) return;
            Window window = getWindow();
            View decor = (window == null) ? null : window.getDecorView();
            if (decor != null) {
                decor.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK);
            }
        });
    }

    // =========================================================================
    // Display, edge-to-edge immersive bars, and DisplayCutout safe-area insets
    // =========================================================================

    private float resolveDisplayDpi() {
        DisplayMetrics dm = getResources().getDisplayMetrics();
        float x = dm.xdpi;
        float y = dm.ydpi;
        float logical = dm.densityDpi;
        // Guard against corrupt OEM EDID values (e.g., 0, <72, >1000, or wildly skewed x/y axes).
        boolean validAxes = x >= 72f && x <= 1000f && y >= 72f && y <= 1000f
                && Math.max(x, y) / Math.min(x, y) < 1.35f;
        if (validAxes) {
            float geo = (float) Math.sqrt((double) x * (double) y);
            if (logical <= 0f || (geo / logical >= 0.65f && geo / logical <= 1.55f)) {
                return geo;
            }
        }
        return logical > 0f ? logical : 160f;
    }

    private void hideSystemBars() {
        Window window = getWindow();
        if (window == null) return;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false);
            WindowInsetsController controller = window.getInsetsController();
            if (controller == null) return;
            controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            controller.hide(WindowInsets.Type.systemBars());
        } else {
            View decor = window.getDecorView();
            if (decor != null) {
                @SuppressWarnings("deprecation")
                int flags = View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                        | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_LAYOUT_STABLE;
                decor.setSystemUiVisibility(flags);
            }
        }
    }

    private void installCutoutListener() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return;
        Window window = getWindow();
        View decor = (window == null) ? null : window.getDecorView();
        if (decor == null) return;
        decor.setOnApplyWindowInsetsListener((view, insets) -> {
            dispatchCutoutInsets(insets);
            return view.onApplyWindowInsets(insets);
        });
    }

    private void requestCutoutRefresh() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return;
        Window window = getWindow();
        View decor = (window == null) ? null : window.getDecorView();
        if (decor == null) return;
        WindowInsets rootInsets = decor.getRootWindowInsets();
        if (rootInsets != null) {
            dispatchCutoutInsets(rootInsets);
        }
        decor.requestApplyInsets();
    }

    private void dispatchCutoutInsets(WindowInsets insets) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P || insets == null) return;
        DisplayCutout cutout = insets.getDisplayCutout();
        int left = cutout == null ? 0 : cutout.getSafeInsetLeft();
        int top = cutout == null ? 0 : cutout.getSafeInsetTop();
        int right = cutout == null ? 0 : cutout.getSafeInsetRight();
        int bottom = cutout == null ? 0 : cutout.getSafeInsetBottom();
        if (left != lastCutoutLeft || top != lastCutoutTop
                || right != lastCutoutRight || bottom != lastCutoutBottom) {
            lastCutoutLeft = left;
            lastCutoutTop = top;
            lastCutoutRight = right;
            lastCutoutBottom = bottom;
            onCutoutInsets(left, top, right, bottom);
        }
    }
}
