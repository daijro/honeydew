package dev.daijro.honeydew;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;

// An update's APK for the system's installer (HoneydewActivity.installApk): the files in files/update, read only,
// reached only through the read grant the install intent carries (not exported). Our own small provider rather than
// androidx's FileProvider, which the app has no dependency for; its authority (<applicationId>.update) is apart from
// HoneydewDocuments'.
public final class HoneydewUpdate extends ContentProvider {
    static final String APK = "application/vnd.android.package-archive";

    static Uri uri(Context context, String name) {
        return new Uri.Builder().scheme("content").authority(context.getPackageName() + ".update").appendPath(name).build();
    }

    // The file `uri` names, in files/update and nowhere else.
    private File file(Uri uri) throws FileNotFoundException {
        File dir = new File(getContext().getFilesDir(), "update");
        String name = uri.getLastPathSegment();
        File file = new File(dir, name == null ? "" : name);
        try {
            if (name == null || !dir.getCanonicalPath().equals(file.getCanonicalFile().getParent()) || !file.isFile()) {
                throw new FileNotFoundException(String.valueOf(uri));
            }
        } catch (IOException e) {
            throw new FileNotFoundException(String.valueOf(uri));
        }
        return file;
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        return APK;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new SecurityException("read only");
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    // The name and size the installer may ask for.
    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        MatrixCursor cursor = new MatrixCursor(new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE});
        try {
            File file = file(uri);
            cursor.addRow(new Object[] {file.getName(), file.length()});
        } catch (FileNotFoundException ignored) {
        }
        return cursor;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("read only");
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] args) {
        throw new UnsupportedOperationException("read only");
    }

    @Override
    public int delete(Uri uri, String selection, String[] args) {
        throw new UnsupportedOperationException("read only");
    }
}
