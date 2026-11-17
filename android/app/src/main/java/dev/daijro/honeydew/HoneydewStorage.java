package dev.daijro.honeydew;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Handles Android Storage Access Framework pickers, managed background file transfers,
 * folder reveal, and share sheets without leaking Activity references.
 */
final class HoneydewStorage {
    interface Listener {
        void onDocumentPicked(String path, String error);
        void onFolderPicked(String files, String error);
        void onFilesCopied(String error);
        void onShowFolderFallback(String path);
    }

    static final int REQUEST_OPEN_DOCUMENT = 1;
    static final int REQUEST_OPEN_FOLDER = 2;

    private static final int IO_BUFFER_BYTES = 64 * 1024;
    private static final int MAX_TREE_DEPTH = 32;

    private final Activity activity;
    private final Context appContext;
    private final Listener listener;
    private final ExecutorService ioExecutor = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "Honeydew-IO");
        t.setDaemon(true);
        return t;
    });

    private volatile long openLimit;

    HoneydewStorage(Activity activity, Listener listener) {
        this.activity = activity;
        this.appContext = activity.getApplicationContext();
        this.listener = listener;
    }

    void shutdown() {
        ioExecutor.shutdownNow();
    }

    void openDocument(long maxBytes) {
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) {
                listener.onDocumentPicked(null, "Activity closed");
                return;
            }
            openLimit = Math.max(0L, maxBytes);
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    .setType("*/*");
            try {
                activity.startActivityForResult(intent, REQUEST_OPEN_DOCUMENT);
            } catch (RuntimeException e) {
                listener.onDocumentPicked(null, safeMessage(e));
            }
        });
    }

    void openFolder() {
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) {
                listener.onFolderPicked(null, "Activity closed");
                return;
            }
            try {
                activity.startActivityForResult(
                        new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE),
                        REQUEST_OPEN_FOLDER);
            } catch (RuntimeException e) {
                listener.onFolderPicked(null, safeMessage(e));
            }
        });
    }

    boolean handleActivityResult(int request, int result, Intent data) {
        if (request != REQUEST_OPEN_DOCUMENT && request != REQUEST_OPEN_FOLDER) {
            return false;
        }
        Uri uri = (result == Activity.RESULT_OK && data != null) ? data.getData() : null;
        if (request == REQUEST_OPEN_FOLDER) {
            if (uri == null) {
                listener.onFolderPicked(null, null);
            } else {
                listFolderAsync(uri);
            }
            return true;
        }
        if (uri == null) {
            listener.onDocumentPicked(null, null);
        } else {
            copyPickedDocumentAsync(uri, openLimit);
        }
        return true;
    }

    void copyFiles(String files) {
        if (files == null || files.isEmpty()) {
            listener.onFilesCopied(null);
            return;
        }
        String[] parts = files.split("\0");
        ioExecutor.execute(() -> {
            try {
                for (int i = 0; i + 1 < parts.length; i += 2) {
                    if (Thread.currentThread().isInterrupted()) {
                        listener.onFilesCopied("Import cancelled");
                        return;
                    }
                    copyFile(Uri.parse(parts[i]), new File(parts[i + 1]));
                }
                listener.onFilesCopied(null);
            } catch (Exception e) {
                listener.onFilesCopied(safeMessage(e));
            }
        });
    }

    void showFolder(String path) {
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) return;
            Uri uri = HoneydewDocuments.folderUri(appContext, path);
            Intent view = new Intent(Intent.ACTION_VIEW)
                    .setDataAndType(uri, Document.MIME_TYPE_DIR)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            Intent tree = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
                    .putExtra(DocumentsContract.EXTRA_INITIAL_URI, uri);
            for (Intent intent : new Intent[] {view, tree}) {
                try {
                    activity.startActivity(intent);
                    return;
                } catch (RuntimeException ignored) {
                }
            }
            listener.onShowFolderFallback(path);
        });
    }

    String share(String paths, String type) {
        if (paths == null || paths.isEmpty()) return "No files to share";
        ArrayList<Uri> uris = new ArrayList<>();
        for (String path : paths.split("\0")) {
            if (path.isEmpty()) continue;
            Uri uri = HoneydewDocuments.documentUri(appContext, path);
            if (uri == null) return path + ": not in the user folder";
            uris.add(uri);
        }
        if (uris.isEmpty()) return "No files to share";
        Intent send = new Intent(uris.size() == 1 ? Intent.ACTION_SEND : Intent.ACTION_SEND_MULTIPLE)
                .setType(type == null || type.isEmpty() ? "*/*" : type)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        if (uris.size() == 1) {
            send.putExtra(Intent.EXTRA_STREAM, uris.get(0));
        } else {
            send.putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris);
        }
        ClipData clip = ClipData.newRawUri(null, uris.get(0));
        for (int i = 1; i < uris.size(); i++) {
            clip.addItem(new ClipData.Item(uris.get(i)));
        }
        send.setClipData(clip);
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) return;
            try {
                activity.startActivity(Intent.createChooser(send, null));
            } catch (ActivityNotFoundException ignored) {
            }
        });
        return null;
    }

    long usableSpace() {
        File dir = appContext.getFilesDir();
        return dir == null ? -1L : dir.getUsableSpace();
    }

    private void copyPickedDocumentAsync(Uri uri, long limit) {
        ioExecutor.execute(() -> {
            File file = null;
            try {
                File dir = new File(appContext.getCacheDir(), "picked");
                File[] old = dir.listFiles();
                if (old != null) {
                    for (File f : old) f.delete();
                }
                if (!dir.isDirectory() && !dir.mkdirs()) {
                    throw new IOException("No cache folder");
                }
                file = new File(dir, displayName(uri));
                // Avoid Long.MAX_VALUE + 1 overflow (which would wrap negative and copy 0 bytes).
                long readLimit = (limit >= Long.MAX_VALUE) ? Long.MAX_VALUE : limit + 1L;
                try (InputStream in = appContext.getContentResolver().openInputStream(uri);
                     OutputStream out = new FileOutputStream(file)) {
                    if (in == null) throw new IOException("The file cannot be read");
                    copyStream(in, out, readLimit);
                }
                listener.onDocumentPicked(file.getPath(), null);
            } catch (Exception e) {
                if (file != null) file.delete();
                listener.onDocumentPicked(null, safeMessage(e));
            }
        });
    }

    private void listFolderAsync(Uri tree) {
        ioExecutor.execute(() -> {
            try {
                String root = DocumentsContract.getTreeDocumentId(tree);
                String name = displayName(DocumentsContract.buildDocumentUriUsingTree(tree, root));
                StringBuilder files = new StringBuilder();
                listRecursive(tree, root, name + "/", 0, files);
                listener.onFolderPicked(files.toString(), null);
            } catch (Exception e) {
                listener.onFolderPicked(null, safeMessage(e));
            }
        });
    }

    private void listRecursive(Uri tree, String id, String path, int depth, StringBuilder files) {
        if (depth > MAX_TREE_DEPTH || Thread.currentThread().isInterrupted()) return;
        String[] columns = {Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE};
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id);
        try (Cursor c = appContext.getContentResolver().query(children, columns, null, null, null)) {
            while (c != null && c.moveToNext()) {
                String child = c.getString(0);
                if (child == null || child.isEmpty()) continue;
                String name = sanitizePart(c.getString(1));
                if (name.isEmpty() || name.startsWith(".")) continue;
                if (Document.MIME_TYPE_DIR.equals(c.getString(2))) {
                    listRecursive(tree, child, path + name + "/", depth + 1, files);
                } else {
                    Uri uri = DocumentsContract.buildDocumentUriUsingTree(tree, child);
                    files.append(path).append(name).append('\0').append(uri).append('\0');
                }
            }
        }
    }

    private void copyFile(Uri from, File to) throws IOException {
        File dir = to.getParentFile();
        if (dir == null) throw new IOException(to + ": invalid destination path");
        if (!dir.isDirectory() && !dir.mkdirs()) {
            throw new IOException(dir + ": the folder cannot be made");
        }
        File part = new File(dir, "." + to.getName() + ".part");
        try (InputStream in = appContext.getContentResolver().openInputStream(from);
             OutputStream out = new FileOutputStream(part)) {
            if (in == null) throw new IOException(from + ": the file cannot be read");
            copyStream(in, out, Long.MAX_VALUE);
        } catch (IOException | RuntimeException e) {
            part.delete();
            throw e;
        }
        if (!part.renameTo(to)) {
            part.delete();
            throw new IOException(to + ": the file cannot be written");
        }
    }

    /**
     * Copies up to {@code maxBytes} from {@code in} to {@code out} using a 64 KB buffer.
     * Respects thread interruption for responsive cancellation.
     */
    static void copyStream(InputStream in, OutputStream out, long maxBytes) throws IOException {
        byte[] buffer = new byte[IO_BUFFER_BYTES];
        long left = maxBytes;
        while (left > 0L) {
            if (Thread.currentThread().isInterrupted()) {
                throw new IOException("Transfer interrupted");
            }
            int toRead = (int) Math.min(buffer.length, left);
            int n = in.read(buffer, 0, toRead);
            if (n < 0) break;
            out.write(buffer, 0, n);
            left -= n;
        }
    }

    private String displayName(Uri uri) {
        String name = null;
        String[] columns = {OpenableColumns.DISPLAY_NAME};
        try (Cursor c = appContext.getContentResolver().query(uri, columns, null, null, null)) {
            if (c != null && c.moveToFirst()) name = c.getString(0);
        } catch (RuntimeException ignored) {
        }
        if (name == null) name = uri.getLastPathSegment();
        name = sanitizePart(name);
        return name.isEmpty() || ".".equals(name) || "..".equals(name) ? "file" : name;
    }

    static String sanitizePart(String name) {
        return name == null ? "" : name.replace('/', '_').replace('\\', '_').replace('\0', '_');
    }

    static String safeMessage(Throwable t) {
        String msg = t.getMessage();
        return (msg == null || msg.isEmpty()) ? t.getClass().getSimpleName() : msg;
    }
}
