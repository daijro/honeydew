package dev.daijro.honeydew;

import android.app.AlertDialog;
import android.Manifest;
import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

public final class HoneydewActivity extends NativeActivity {
    static { System.loadLibrary("honeydew"); }

    private AlertDialog editorDialog;
    private long activeRequestId;

    private static native void onTextResult(long requestId, String text, boolean accepted);
    private static native void onDisplayDpi(float dpi);
    private static native void onDocumentPicked(String path, String error);
    private static native void onFolderPicked(String files, String error);
    private static native void onFilesCopied(String error);

    private static final int OPEN_DOCUMENT = 1;
    private static final int OPEN_FOLDER = 2;
    private static final int RECORD_AUDIO = 3;
    private long openLimit;

    @Override
    protected void onCreate(Bundle state) {
        // Publish physical density before NativeActivity starts the Rust app thread.
        onDisplayDpi(getResources().getDisplayMetrics().xdpi);
        super.onCreate(state);
        hideSystemBars();
        askForMicrophone();
    }

    // Audio recording reads the microphone: asked for once when the app starts, as the system asks for any permission.
    private void askForMicrophone() {
        if (!hasRecordPermission()) {
            requestPermissions(new String[] {Manifest.permission.RECORD_AUDIO}, RECORD_AUDIO);
        }
    }

    // The native side opens the input only with this granted (the request's answer comes back to the next open).
    public boolean hasRecordPermission() {
        return checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED;
    }

    // Immersive: the status and navigation bars stay hidden, so an edge touch can't press them; a swipe from the edge
    // shows them over the app for a moment. Applied again whenever the window has focus back (after a dialog, the
    // keyboard or the file picker).
    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) hideSystemBars();
    }

    private void hideSystemBars() {
        Window window = getWindow();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false);
            WindowInsetsController controller = window.getInsetsController();
            if (controller == null) return;
            controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            controller.hide(WindowInsets.Type.systemBars());
        } else {
            window.getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_STABLE);
        }
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        onDisplayDpi(getResources().getDisplayMetrics().xdpi);
    }

    // The native side asks for an orientation (Settings' "Screen orientation") from its own thread; the window turns
    // on the UI thread only, as every other call from that side is posted (see editText and openDocument).
    public void requestOrientation(final int orientation) {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) return;
            setRequestedOrientation(orientation);
        });
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

    // SONGS `Import folder`: the system folder picker, which reaches Downloads, an SD card and other providers' folders.
    public void openFolder() {
        runOnUiThread(() -> {
            try {
                startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), OPEN_FOLDER);
            } catch (RuntimeException e) {
                onFolderPicked(null, String.valueOf(e.getMessage()));
            }
        });
    }

    // The picked folder's files, listed on their own thread (a big folder takes a while), go back to Rust.
    private void folderPicked(Uri tree) {
        new Thread(() -> {
            try {
                String root = DocumentsContract.getTreeDocumentId(tree);
                String name = displayName(DocumentsContract.buildDocumentUriUsingTree(tree, root));
                StringBuilder files = new StringBuilder();
                list(tree, root, name + "/", 0, files);
                onFolderPicked(files.toString(), null);
            } catch (Exception e) {
                onFolderPicked(null, String.valueOf(e.getMessage()));
            }
        }).start();
    }

    // Appends the files below the document `id` of the picked `tree` (`path` is its own) to `files`: each one's path
    // from the picked folder's parent and its URI, each ending in '\0'. Hidden files and folders are left out, as the
    // import skips them, and so is anything more than 32 folders down (a link loop).
    private void list(Uri tree, String id, String path, int depth, StringBuilder files) {
        if (depth > 32) return;
        String[] columns = {Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE};
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id);
        try (Cursor c = getContentResolver().query(children, columns, null, null, null)) {
            while (c != null && c.moveToNext()) {
                String child = c.getString(0), name = part(c.getString(1));
                if (name.startsWith(".")) continue;
                if (Document.MIME_TYPE_DIR.equals(c.getString(2))) {
                    list(tree, child, path + name + "/", depth + 1, files);
                } else {
                    Uri uri = DocumentsContract.buildDocumentUriUsingTree(tree, child);
                    files.append(path).append(name).append('\0').append(uri).append('\0');
                }
            }
        }
    }

    // A folder import's copy, on its own thread: `files` alternates a document URI and the user file it becomes, each
    // ending in '\0'. The first file that fails stops it.
    public void copyFiles(String files) {
        String[] parts = files.split("\0");
        new Thread(() -> {
            try {
                for (int i = 0; i + 1 < parts.length; i += 2) copyFile(Uri.parse(parts[i]), new File(parts[i + 1]));
                onFilesCopied(null);
            } catch (Exception e) {
                onFilesCopied(String.valueOf(e.getMessage()));
            }
        }).start();
    }

    // Copies the document `from` to `to`, making its folder: whole or not at all (beside it first, then renamed).
    private void copyFile(Uri from, File to) throws IOException {
        File dir = to.getParentFile(), part = new File(dir, "." + to.getName() + ".part");
        if (!dir.isDirectory() && !dir.mkdirs()) throw new IOException(dir + ": the folder cannot be made");
        try (InputStream in = getContentResolver().openInputStream(from);
             OutputStream out = new FileOutputStream(part)) {
            if (in == null) throw new IOException(from + ": the file cannot be read");
            copy(in, out, Long.MAX_VALUE);
        } catch (IOException | RuntimeException e) {
            part.delete();
            throw e;
        }
        if (!part.renameTo(to)) {
            part.delete();
            throw new IOException(to + ": the file cannot be written");
        }
    }

    // Copies at most `left` bytes.
    private static void copy(InputStream in, OutputStream out, long left) throws IOException {
        byte[] buffer = new byte[1 << 16];
        for (int n; left > 0; left -= n) {
            n = in.read(buffer, 0, (int) Math.min(buffer.length, left));
            if (n < 0) break;
            out.write(buffer, 0, n);
        }
    }

    // SETTINGS `Open user folder` / `Open folder`: `path`, a folder in the user folder, in the system Files app through
    // HoneydewDocuments; else the folder picker open there; else a message with the path.
    public void showFolder(String path) {
        runOnUiThread(() -> {
            Uri uri = HoneydewDocuments.folderUri(this, path);
            Intent view = new Intent(Intent.ACTION_VIEW)
                    .setDataAndType(uri, DocumentsContract.Document.MIME_TYPE_DIR)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            Intent tree = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
                    .putExtra(DocumentsContract.EXTRA_INITIAL_URI, uri);
            for (Intent intent : new Intent[] {view, tree}) {
                try {
                    startActivity(intent);
                    return;
                } catch (RuntimeException ignored) {
                }
            }
            if (isFinishing() || isDestroyed()) return;
            new AlertDialog.Builder(this)
                    .setMessage("No app on this phone can open folders. The folder is " + path)
                    .setPositiveButton("OK", null)
                    .show();
        });
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        if (request != OPEN_DOCUMENT && request != OPEN_FOLDER) {
            super.onActivityResult(request, result, data);
            return;
        }
        Uri uri = result == RESULT_OK && data != null ? data.getData() : null;
        if (request == OPEN_FOLDER) {
            if (uri == null) onFolderPicked(null, null);
            else folderPicked(uri);
            return;
        }
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
                    copy(in, out, limit + 1);
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
        name = part(name);
        return name.isEmpty() || name.equals(".") || name.equals("..") ? "file" : name;
    }

    // A provider's name as one part of a path: no separators, and no '\0' (the lists to Rust end their parts in it).
    private static String part(String name) {
        return name == null ? "" : name.replace('/', '_').replace('\\', '_').replace('\0', '_');
    }

    @Override
    protected void onDestroy() {
        if (editorDialog != null) editorDialog.dismiss();
        super.onDestroy();
        onDisplayDpi(0);
    }
}
