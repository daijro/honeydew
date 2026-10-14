package dev.daijro.honeydew;

import android.app.AlertDialog;
import android.app.NativeActivity;
import android.content.res.Configuration;
import android.os.Bundle;
import android.text.InputType;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;

public final class HoneydewActivity extends NativeActivity {
    static { System.loadLibrary("honeydew"); }

    private AlertDialog editorDialog;
    private long activeRequestId;

    private static native void onTextResult(long requestId, String text, boolean accepted);
    private static native void onDisplayDpi(float dpi);

    @Override
    protected void onCreate(Bundle state) {
        // Publish physical density before NativeActivity starts the Rust app thread.
        onDisplayDpi(getResources().getDisplayMetrics().xdpi);
        super.onCreate(state);
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        onDisplayDpi(getResources().getDisplayMetrics().xdpi);
    }

    // NativeActivity's key events do not carry complete IME composition or pasted text.
    public void editText(long requestId, String text, boolean multiline) {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) {
                onTextResult(requestId, text, false);
                return;
            }
            if (editorDialog != null) editorDialog.dismiss();

            EditText input = new EditText(this);
            input.setInputType(InputType.TYPE_CLASS_TEXT
                    | (multiline ? InputType.TYPE_TEXT_FLAG_MULTI_LINE : 0));
            input.setSingleLine(!multiline);
            input.setImeOptions(EditorInfo.IME_FLAG_NO_FULLSCREEN
                    | (multiline ? EditorInfo.IME_ACTION_NONE : EditorInfo.IME_ACTION_DONE));
            input.setText(text);
            input.setSelection(input.length());

            boolean[] accepted = {false};
            AlertDialog dialog = new AlertDialog.Builder(this)
                    .setTitle("Edit text")
                    .setView(input)
                    .setPositiveButton("Done", (whichDialog, whichButton) -> accepted[0] = true)
                    .setNegativeButton("Cancel", null)
                    .create();
            dialog.setOnDismissListener(whichDialog -> {
                if (editorDialog == dialog) {
                    editorDialog = null;
                    activeRequestId = 0;
                }
                onTextResult(requestId, input.getText().toString(), accepted[0]);
            });
            input.setOnEditorActionListener((view, action, event) -> {
                if (!multiline && action == EditorInfo.IME_ACTION_DONE) {
                    accepted[0] = true;
                    dialog.dismiss();
                    return true;
                }
                return false;
            });
            editorDialog = dialog;
            activeRequestId = requestId;
            dialog.show();
            input.requestFocus();
            dialog.getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE
                    | WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        });
    }

    public void cancelTextEdit(long requestId) {
        runOnUiThread(() -> {
            if (editorDialog != null && activeRequestId == requestId) editorDialog.dismiss();
        });
    }

    @Override
    protected void onDestroy() {
        if (editorDialog != null) editorDialog.dismiss();
        super.onDestroy();
        onDisplayDpi(0);
    }
}
