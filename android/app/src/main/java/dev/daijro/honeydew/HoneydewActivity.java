package dev.daijro.honeydew;

import android.app.AlertDialog;
import android.app.NativeActivity;
import android.content.Intent;
import android.content.res.Configuration;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

public final class HoneydewActivity extends NativeActivity {
    static { System.loadLibrary("honeydew"); }

    private AlertDialog editorDialog;
    private long activeRequestId;

    private static native void onTextResult(long requestId, String text, boolean accepted);
    private static native void onDisplayDpi(float dpi);
    private static native void onDocumentPicked(String path, String error);

    private static final int OPEN_DOCUMENT = 1;
    private long openLimit;

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

    // The system picker reaches every document provider (Downloads, Drive, USB). The pick is copied into the cache,
    // at most maxBytes + 1 bytes so the native reader can refuse an oversized file, and its path goes back to Rust.
    public void openDocument(long maxBytes) {
        runOnUiThread(() -> {
            openLimit = maxBytes;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    .setType("*/*");
            try {
                startActivityForResult(intent, OPEN_DOCUMENT);
            } catch (RuntimeException e) {
                onDocumentPicked(null, String.valueOf(e.getMessage()));
            }
        });
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        if (request != OPEN_DOCUMENT) {
            super.onActivityResult(request, result, data);
            return;
        }
        Uri uri = result == RESULT_OK && data != null ? data.getData() : null;
        if (uri == null) {
            onDocumentPicked(null, null);
            return;
        }
        long limit = openLimit;
        new Thread(() -> {
            try {
                File dir = new File(getCacheDir(), "picked");
                File[] old = dir.listFiles();
                if (old != null) for (File f : old) f.delete();
                if (!dir.isDirectory() && !dir.mkdirs()) throw new java.io.IOException("No cache folder");
                File file = new File(dir, displayName(uri));
                try (InputStream in = getContentResolver().openInputStream(uri);
                     OutputStream out = new FileOutputStream(file)) {
                    if (in == null) throw new java.io.IOException("The file cannot be read");
                    byte[] buffer = new byte[1 << 16];
                    long left = limit + 1;
                    for (int n; left > 0; left -= n) {
                        n = in.read(buffer, 0, (int) Math.min(buffer.length, left));
                        if (n < 0) break;
                        out.write(buffer, 0, n);
                    }
                }
                onDocumentPicked(file.getPath(), null);
            } catch (Exception e) {
                onDocumentPicked(null, String.valueOf(e.getMessage()));
            }
        }).start();
    }

    // The provider's display name keeps the extension the importer goes by; never a path.
    private String displayName(Uri uri) {
        String name = null;
        String[] columns = {OpenableColumns.DISPLAY_NAME};
        try (Cursor c = getContentResolver().query(uri, columns, null, null, null)) {
            if (c != null && c.moveToFirst()) name = c.getString(0);
        } catch (RuntimeException ignored) {
        }
        if (name == null) name = uri.getLastPathSegment();
        name = name == null ? "" : name.replace('/', '_').replace('\\', '_');
        return name.isEmpty() || name.equals(".") || name.equals("..") ? "file" : name;
    }

    @Override
    protected void onDestroy() {
        if (editorDialog != null) editorDialog.dismiss();
        super.onDestroy();
        onDisplayDpi(0);
    }
}
