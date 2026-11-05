package dev.daijro.honeydew;

import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.CancellationSignal;
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
import java.util.ArrayList;
import java.util.Locale;

// The user folder (songs, samples, soundfonts, Original Content) as the "Honeydew" root of the system Files app and of
// every app's document picker, and over USB where the phone offers document providers. The app's own folders are out
// of reach of file managers otherwise. A document ID is `honeydew` and the path below the user folder
// (`honeydew/My Songs/A.flm`), so that it stays the same when the user folder moves (#58).
public final class HoneydewDocuments extends DocumentsProvider {
    static final String ROOT = "honeydew";
    private static final String[] ROOT_COLUMNS = {Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE,
            Root.COLUMN_FLAGS, Root.COLUMN_ICON, Root.COLUMN_MIME_TYPES, Root.COLUMN_AVAILABLE_BYTES};
    private static final String[] DOCUMENT_COLUMNS = {Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME,
            Document.COLUMN_MIME_TYPE, Document.COLUMN_FLAGS, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED};

    static String authority(Context context) {
        return context.getPackageName() + ".documents";
    }

    // The folder the app uses (`ui::main_menu::user_dir`): the external files folder, unless the user folder stayed in
    // the app data folder, which then holds the marker `migrate_user_dir` leaves (`STAYS`).
    static File userFolder(Context context) throws IOException {
        File internal = new File(context.getFilesDir(), "Honeydew");
        File external = context.getExternalFilesDir(null);
        boolean stays = external == null || new File(internal, ".user-folder-stays-here").exists();
        return (stays ? internal : external).getCanonicalFile();
    }

    // The folder at `path` as a document (`HoneydewActivity.showFolder`); the root, when it is not in the user folder.
    static Uri folderUri(Context context, String path) {
        String id = ROOT;
        try {
            File root = userFolder(context), folder = new File(path).getCanonicalFile();
            if (inside(root, folder)) id = id(root, folder);
        } catch (IOException ignored) {
        }
        return DocumentsContract.buildDocumentUri(authority(context), id);
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Cursor queryRoots(String[] projection) throws FileNotFoundException {
        File root = root();
        root.mkdirs();
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : ROOT_COLUMNS);
        cursor.newRow()
                .add(Root.COLUMN_ROOT_ID, ROOT)
                .add(Root.COLUMN_DOCUMENT_ID, ROOT)
                .add(Root.COLUMN_TITLE, getContext().getString(R.string.app_name))
                .add(Root.COLUMN_FLAGS, Root.FLAG_SUPPORTS_CREATE | Root.FLAG_SUPPORTS_IS_CHILD | Root.FLAG_LOCAL_ONLY)
                .add(Root.COLUMN_ICON, R.drawable.honeydew)
                .add(Root.COLUMN_MIME_TYPES, "*/*")
                .add(Root.COLUMN_AVAILABLE_BYTES, root.getFreeSpace());
        return cursor;
    }

    @Override
    public Cursor queryDocument(String id, String[] projection) throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : DOCUMENT_COLUMNS);
        File root = root();
        row(cursor, root, file(root, id));
        return cursor;
    }

    @Override
    public Cursor queryChildDocuments(String parentId, String[] projection, String sortOrder)
            throws FileNotFoundException {
        File root = root(), parent = file(root, parentId);
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : DOCUMENT_COLUMNS);
        File[] children = parent.listFiles();
        // Dot files are the app's (the marker above, a settings write's temp file).
        if (children != null) for (File child : children) {
            if (!child.getName().startsWith(".")) row(cursor, root, child);
        }
        cursor.setNotificationUri(getContext().getContentResolver(), children(parentId));
        return cursor;
    }

    @Override
    public ParcelFileDescriptor openDocument(String id, String mode, CancellationSignal signal)
            throws FileNotFoundException {
        return ParcelFileDescriptor.open(file(root(), id), ParcelFileDescriptor.parseMode(mode));
    }

    // A name taken gets a number, as the app's own copies do: `Kick (1).wav`.
    @Override
    public String createDocument(String parentId, String mimeType, String displayName) throws FileNotFoundException {
        File root = root(), parent = file(root, parentId);
        String name = name(displayName);
        boolean folder = Document.MIME_TYPE_DIR.equals(mimeType);
        int dot = folder ? -1 : name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name, extension = dot > 0 ? name.substring(dot) : "";
        File file = new File(parent, name);
        for (int n = 1; file.exists(); n++) file = new File(parent, stem + " (" + n + ")" + extension);
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
        File root = root(), file = notRoot(root, id), to = new File(file.getParentFile(), name(displayName));
        if (to.exists() || !file.renameTo(to)) throw new FileNotFoundException("Cannot rename to " + to.getName());
        revokeDocumentPermission(id);
        notifyChanged(id(root, file.getParentFile()));
        return id(root, to);
    }

    @Override
    public void deleteDocument(String id) throws FileNotFoundException {
        File root = root(), file = notRoot(root, id);
        if (!delete(file)) throw new FileNotFoundException("Cannot delete " + file.getName());
        revokeDocumentPermission(id);
        notifyChanged(id(root, file.getParentFile()));
    }

    // Tree grants (`ACTION_OPEN_DOCUMENT_TREE`) and moves ask this; the parent itself counts.
    @Override
    public boolean isChildDocument(String parentId, String id) {
        try {
            File root = root();
            return inside(file(root, parentId), file(root, id));
        } catch (FileNotFoundException e) {
            return false;
        }
    }

    // The Files app opens a folder (`HoneydewActivity.showFolder`) by the IDs from the root, or from `parentId`, to it.
    @Override
    public DocumentsContract.Path findDocumentPath(String parentId, String id) throws FileNotFoundException {
        File root = root(), top = file(root, parentId == null ? ROOT : parentId), file = file(root, id);
        if (!inside(top, file)) throw new FileNotFoundException("Not in " + top.getName() + ": " + id);
        ArrayList<String> ids = new ArrayList<>();
        for (File f = file; !f.equals(top); f = f.getParentFile()) ids.add(0, id(root, f));
        ids.add(0, id(root, top));
        return new DocumentsContract.Path(parentId == null ? ROOT : null, ids);
    }

    private File root() throws FileNotFoundException {
        try {
            return userFolder(getContext());
        } catch (IOException e) {
            throw new FileNotFoundException(e.getMessage());
        }
    }

    // Every ID from outside goes through here: no `..` or link may lead out of the user folder.
    private static File file(File root, String id) throws FileNotFoundException {
        if (id.equals(ROOT) || id.startsWith(ROOT + "/")) {
            try {
                File file = new File(root, id.substring(ROOT.length())).getCanonicalFile();
                if (inside(root, file)) return file;
            } catch (IOException ignored) {
            }
        }
        throw new FileNotFoundException("Not in the Honeydew folder: " + id);
    }

    // `file` is in `root`.
    private static String id(File root, File file) {
        return ROOT + file.getPath().substring(root.getPath().length());
    }

    private static File notRoot(File root, String id) throws FileNotFoundException {
        File file = file(root, id);
        if (file.equals(root)) throw new FileNotFoundException("The Honeydew folder itself stays");
        return file;
    }

    private static boolean inside(File folder, File file) {
        String f = folder.getPath(), p = file.getPath();
        return p.equals(f) || p.startsWith(f + File.separator);
    }

    // A plain file name: a separator becomes `_`, and `.` or `..` names nothing.
    private static String name(String displayName) throws FileNotFoundException {
        String name = displayName == null ? "" : displayName.replace('/', '_');
        if (name.isEmpty() || name.equals(".") || name.equals("..")) throw new FileNotFoundException("Bad name");
        return name;
    }

    // A link is removed, never what it points at.
    private static boolean delete(File file) {
        File[] children = Files.isSymbolicLink(file.toPath()) ? null : file.listFiles();
        if (children != null) for (File child : children) delete(child);
        return file.delete();
    }

    private void row(MatrixCursor cursor, File root, File file) {
        boolean folder = file.isDirectory(), isRoot = file.equals(root);
        int flags = folder ? Document.FLAG_DIR_SUPPORTS_CREATE : Document.FLAG_SUPPORTS_WRITE;
        if (!isRoot) flags |= Document.FLAG_SUPPORTS_DELETE | Document.FLAG_SUPPORTS_RENAME;
        cursor.newRow()
                .add(Document.COLUMN_DOCUMENT_ID, id(root, file))
                .add(Document.COLUMN_DISPLAY_NAME, isRoot ? getContext().getString(R.string.app_name) : file.getName())
                .add(Document.COLUMN_MIME_TYPE, folder ? Document.MIME_TYPE_DIR : mimeType(file.getName()))
                .add(Document.COLUMN_FLAGS, flags)
                .add(Document.COLUMN_SIZE, folder ? null : file.length())
                .add(Document.COLUMN_LAST_MODIFIED, file.lastModified());
    }

    private static String mimeType(String name) {
        int dot = name.lastIndexOf('.');
        String extension = name.substring(dot + 1).toLowerCase(Locale.ROOT);
        String type = dot < 0 ? null : MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
        return type != null ? type : "application/octet-stream";
    }

    private Uri children(String folderId) {
        return DocumentsContract.buildChildDocumentsUri(authority(getContext()), folderId);
    }

    private void notifyChanged(String folderId) {
        getContext().getContentResolver().notifyChange(children(folderId), null);
    }
}
