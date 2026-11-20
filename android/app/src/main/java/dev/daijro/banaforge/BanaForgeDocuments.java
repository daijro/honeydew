package dev.daijro.banaforge;

import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.CancellationSignal;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import android.provider.DocumentsContract.Root;
import android.provider.DocumentsProvider;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.nio.file.Files;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Locale;

/**
 * Exposes the BanaForge user folder (songs, samples, soundfonts, Original Content) as a
 * first-class root in Android's Files app and document pickers, with full search and
 * write-close notification support.
 */
public final class BanaForgeDocuments extends DocumentsProvider {
    static final String ROOT = "banaforge";
    private static final int MAX_SEARCH_RESULTS = 200;
    private static final int MAX_SEARCH_DEPTH = 16;

    private static final String[] ROOT_COLUMNS = {
            Root.COLUMN_ROOT_ID,
            Root.COLUMN_DOCUMENT_ID,
            Root.COLUMN_TITLE,
            Root.COLUMN_FLAGS,
            Root.COLUMN_ICON,
            Root.COLUMN_MIME_TYPES,
            Root.COLUMN_AVAILABLE_BYTES
    };
    private static final String[] DOCUMENT_COLUMNS = {
            Document.COLUMN_DOCUMENT_ID,
            Document.COLUMN_DISPLAY_NAME,
            Document.COLUMN_MIME_TYPE,
            Document.COLUMN_FLAGS,
            Document.COLUMN_SIZE,
            Document.COLUMN_LAST_MODIFIED
    };

    private Handler notifyHandler;

    static String authority(Context context) {
        return context.getPackageName() + ".documents";
    }

    static File userFolder(Context context) throws IOException {
        File internal = new File(context.getFilesDir(), "BanaForge");
        File external = context.getExternalFilesDir(null);
        boolean stays = external == null || new File(internal, ".user-folder-stays-here").exists();
        return (stays ? internal : external).getCanonicalFile();
    }

    static Uri folderUri(Context context, String path) {
        Uri uri = documentUri(context, path);
        return uri != null ? uri : DocumentsContract.buildDocumentUri(authority(context), ROOT);
    }

    static Uri documentUri(Context context, String path) {
        if (context == null || path == null || path.isEmpty()) return null;
        try {
            File root = userFolder(context);
            File file = new File(path).getCanonicalFile();
            if (inside(root, file)) {
                return DocumentsContract.buildDocumentUri(authority(context), id(root, file));
            }
        } catch (IOException ignored) {
        }
        return null;
    }

    @Override
    public boolean onCreate() {
        notifyHandler = new Handler(Looper.getMainLooper());
        return true;
    }

    @Override
    public Cursor queryRoots(String[] projection) throws FileNotFoundException {
        Context context = requireProviderContext();
        File root = root();
        root.mkdirs();
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : ROOT_COLUMNS);
        cursor.newRow()
                .add(Root.COLUMN_ROOT_ID, ROOT)
                .add(Root.COLUMN_DOCUMENT_ID, ROOT)
                .add(Root.COLUMN_TITLE, context.getString(R.string.app_name))
                .add(Root.COLUMN_FLAGS, Root.FLAG_SUPPORTS_CREATE
                        | Root.FLAG_SUPPORTS_IS_CHILD
                        | Root.FLAG_SUPPORTS_SEARCH
                        | Root.FLAG_LOCAL_ONLY)
                .add(Root.COLUMN_ICON, R.mipmap.ic_launcher)
                .add(Root.COLUMN_MIME_TYPES, "*/*")
                .add(Root.COLUMN_AVAILABLE_BYTES, root.getUsableSpace());
        return cursor;
    }

    @Override
    public Cursor queryDocument(String id, String[] projection) throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : DOCUMENT_COLUMNS);
        File root = root();
        File target = file(root, id);
        if (!target.exists()) throw new FileNotFoundException("Missing document: " + id);
        row(cursor, root, target);
        return cursor;
    }

    @Override
    public Cursor queryChildDocuments(String parentId, String[] projection, String sortOrder)
            throws FileNotFoundException {
        Context context = requireProviderContext();
        File root = root();
        File parent = file(root, parentId);
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : DOCUMENT_COLUMNS);
        File[] children = parent.listFiles();
        if (children != null) {
            for (File child : children) {
                if (!child.getName().startsWith(".")) {
                    row(cursor, root, child);
                }
            }
        }
        cursor.setNotificationUri(context.getContentResolver(), children(parentId));
        return cursor;
    }

    @Override
    public Cursor querySearchDocuments(String rootId, String query, String[] projection)
            throws FileNotFoundException {
        if (!ROOT.equals(rootId)) throw new FileNotFoundException("Unknown root: " + rootId);
        File root = root();
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : DOCUMENT_COLUMNS);
        String needle = query == null ? "" : query.trim().toLowerCase(Locale.ROOT);
        if (needle.isEmpty()) return cursor;

        ArrayDeque<File> queue = new ArrayDeque<>();
        ArrayDeque<Integer> depths = new ArrayDeque<>();
        queue.add(root);
        depths.add(0);
        int found = 0;

        while (!queue.isEmpty() && found < MAX_SEARCH_RESULTS) {
            File dir = queue.poll();
            int depth = depths.poll();
            if (dir == null || depth > MAX_SEARCH_DEPTH) continue;
            File[] entries = Files.isSymbolicLink(dir.toPath()) ? null : dir.listFiles();
            if (entries == null) continue;
            for (File entry : entries) {
                String name = entry.getName();
                if (name.startsWith(".")) continue;
                if (name.toLowerCase(Locale.ROOT).contains(needle)) {
                    row(cursor, root, entry);
                    if (++found >= MAX_SEARCH_RESULTS) break;
                }
                if (entry.isDirectory() && depth < MAX_SEARCH_DEPTH) {
                    queue.add(entry);
                    depths.add(depth + 1);
                }
            }
        }
        return cursor;
    }

    @Override
    public ParcelFileDescriptor openDocument(String id, String mode, CancellationSignal signal)
            throws FileNotFoundException {
        File root = root();
        File target = file(root, id);
        int accessMode = ParcelFileDescriptor.parseMode(mode);
        boolean isWrite = mode != null && mode.indexOf('w') >= 0;
        if (!isWrite) {
            return ParcelFileDescriptor.open(target, accessMode);
        }
        File parent = target.getParentFile();
        String parentId = (parent != null && inside(root, parent)) ? id(root, parent) : ROOT;
        try {
            Handler handler = notifyHandler != null ? notifyHandler : new Handler(Looper.getMainLooper());
            return ParcelFileDescriptor.open(target, accessMode, handler, e -> notifyChanged(parentId));
        } catch (IOException e) {
            throw new FileNotFoundException("Cannot open for write: " + target.getName());
        }
    }

    @Override
    public String createDocument(String parentId, String mimeType, String displayName) throws FileNotFoundException {
        File root = root();
        File parent = file(root, parentId);
        String name = sanitizeName(displayName);
        boolean folder = Document.MIME_TYPE_DIR.equals(mimeType);
        int dot = folder ? -1 : name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String extension = dot > 0 ? name.substring(dot) : "";
        File file = new File(parent, name);
        for (int n = 1; file.exists(); n++) {
            file = new File(parent, stem + " (" + n + ")" + extension);
        }
        boolean made;
        try {
            made = folder ? file.mkdir() : file.createNewFile();
        } catch (IOException e) {
            made = false;
        }
        if (!made) throw new FileNotFoundException("Cannot create " + file.getName());
        notifyChanged(parentId);
        return id(root, file);
    }

    @Override
    public String renameDocument(String id, String displayName) throws FileNotFoundException {
        File root = root();
        File file = notRoot(root, id);
        File parent = file.getParentFile();
        if (parent == null) throw new FileNotFoundException("Missing parent");
        File to = new File(parent, sanitizeName(displayName));
        if (to.exists() || !file.renameTo(to)) {
            throw new FileNotFoundException("Cannot rename to " + to.getName());
        }
        revokeDocumentPermission(id);
        notifyChanged(id(root, parent));
        return id(root, to);
    }

    @Override
    public void deleteDocument(String id) throws FileNotFoundException {
        File root = root();
        File file = notRoot(root, id);
        File parent = file.getParentFile();
        if (!deleteRecursive(file)) {
            throw new FileNotFoundException("Cannot delete " + file.getName());
        }
        revokeDocumentPermission(id);
        if (parent != null && inside(root, parent)) {
            notifyChanged(id(root, parent));
        }
    }

    @Override
    public boolean isChildDocument(String parentId, String id) {
        try {
            File root = root();
            return inside(file(root, parentId), file(root, id));
        } catch (FileNotFoundException e) {
            return false;
        }
    }

    @Override
    public DocumentsContract.Path findDocumentPath(String parentId, String id) throws FileNotFoundException {
        File root = root();
        File top = file(root, parentId == null ? ROOT : parentId);
        File target = file(root, id);
        if (!inside(top, target)) {
            throw new FileNotFoundException("Not in " + top.getName() + ": " + id);
        }
        ArrayList<String> ids = new ArrayList<>();
        for (File f = target; f != null && !f.equals(top); f = f.getParentFile()) {
            ids.add(0, id(root, f));
        }
        ids.add(0, id(root, top));
        return new DocumentsContract.Path(parentId == null ? ROOT : null, ids);
    }

    private Context requireProviderContext() throws FileNotFoundException {
        Context context = getContext();
        if (context == null) throw new FileNotFoundException("Provider context unavailable");
        return context;
    }

    private File root() throws FileNotFoundException {
        try {
            return userFolder(requireProviderContext());
        } catch (IOException e) {
            throw new FileNotFoundException(e.getMessage());
        }
    }

    private static File file(File root, String id) throws FileNotFoundException {
        if (id != null && (id.equals(ROOT) || id.startsWith(ROOT + "/"))) {
            try {
                File file = new File(root, id.substring(ROOT.length())).getCanonicalFile();
                if (inside(root, file)) return file;
            } catch (IOException ignored) {
            }
        }
        throw new FileNotFoundException("Not in the BanaForge folder: " + id);
    }

    private static String id(File root, File file) {
        return ROOT + file.getPath().substring(root.getPath().length());
    }

    private static File notRoot(File root, String id) throws FileNotFoundException {
        File file = file(root, id);
        if (file.equals(root)) throw new FileNotFoundException("The BanaForge folder itself stays");
        return file;
    }

    private static boolean inside(File folder, File file) {
        String f = folder.getPath();
        String p = file.getPath();
        return p.equals(f) || p.startsWith(f + File.separator);
    }

    /** Rejects path separators, NUL bytes, and leading dots (protecting internal app markers). */
    private static String sanitizeName(String displayName) throws FileNotFoundException {
        String name = displayName == null
                ? ""
                : displayName.replace('/', '_').replace('\\', '_').replace('\0', '_').trim();
        if (name.isEmpty() || name.startsWith(".")) {
            throw new FileNotFoundException("Bad name");
        }
        return name;
    }

    private static boolean deleteRecursive(File file) {
        boolean childrenOk = true;
        File[] children = Files.isSymbolicLink(file.toPath()) ? null : file.listFiles();
        if (children != null) {
            for (File child : children) {
                childrenOk &= deleteRecursive(child);
            }
        }
        return file.delete() && childrenOk;
    }

    private void row(MatrixCursor cursor, File root, File file) throws FileNotFoundException {
        Context context = requireProviderContext();
        boolean folder = file.isDirectory();
        boolean isRoot = file.equals(root);
        int flags = folder ? Document.FLAG_DIR_SUPPORTS_CREATE : Document.FLAG_SUPPORTS_WRITE;
        if (!isRoot) {
            flags |= Document.FLAG_SUPPORTS_DELETE | Document.FLAG_SUPPORTS_RENAME;
        }
        cursor.newRow()
                .add(Document.COLUMN_DOCUMENT_ID, id(root, file))
                .add(Document.COLUMN_DISPLAY_NAME, isRoot ? context.getString(R.string.app_name) : file.getName())
                .add(Document.COLUMN_MIME_TYPE, folder ? Document.MIME_TYPE_DIR : mimeType(file.getName()))
                .add(Document.COLUMN_FLAGS, flags)
                .add(Document.COLUMN_SIZE, folder ? null : file.length())
                .add(Document.COLUMN_LAST_MODIFIED, file.lastModified());
    }

    private static String mimeType(String name) {
        int dot = name.lastIndexOf('.');
        if (dot <= 0 || dot >= name.length() - 1) return "application/octet-stream";
        String ext = name.substring(dot + 1).toLowerCase(Locale.ROOT);
        switch (ext) {
            case "wav": return "audio/wav";
            case "flac": return "audio/flac";
            case "ogg": return "audio/ogg";
            case "mp3": return "audio/mpeg";
            case "mid":
            case "midi": return "audio/midi";
            case "zip": return "application/zip";
            case "json": return "application/json";
            default:
                String type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
                return type != null ? type : "application/octet-stream";
        }
    }

    private Uri children(String folderId) throws FileNotFoundException {
        return DocumentsContract.buildChildDocumentsUri(authority(requireProviderContext()), folderId);
    }

    private void notifyChanged(String folderId) {
        try {
            Context context = requireProviderContext();
            context.getContentResolver().notifyChange(children(folderId), null);
        } catch (FileNotFoundException ignored) {
        }
    }
}
