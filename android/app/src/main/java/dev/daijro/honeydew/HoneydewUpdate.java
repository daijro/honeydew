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
import java.util.Locale;

/**
 * Read-only, non-exported ContentProvider serving verified APK updates from files/update
 * to Android's package installer via temporary read URI grants.
 */
public final class HoneydewUpdate extends ContentProvider {
    static final String APK = "application/vnd.android.package-archive";
    private static final String[] DEFAULT_PROJECTION = {
            OpenableColumns.DISPLAY_NAME,
            OpenableColumns.SIZE
    };

    static Uri uri(Context context, String name) {
        return new Uri.Builder()
                .scheme("content")
                .authority(context.getPackageName() + ".update")
                .appendPath(name)
                .build();
    }

    private File file(Uri uri) throws FileNotFoundException {
        Context context = getContext();
        if (context == null || uri == null) {
            throw new FileNotFoundException("Unavailable update URI");
        }
        String name = uri.getLastPathSegment();
        if (name == null || !name.toLowerCase(Locale.ROOT).endsWith(".apk")) {
            throw new FileNotFoundException("Not an APK: " + uri);
        }
        File dir = new File(context.getFilesDir(), "update");
        File candidate = new File(dir, name);
        try {
            if (!dir.getCanonicalPath().equals(candidate.getCanonicalFile().getParent()) || !candidate.isFile()) {
                throw new FileNotFoundException(String.valueOf(uri));
            }
        } catch (IOException e) {
            throw new FileNotFoundException(String.valueOf(uri));
        }
        return candidate;
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

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        String[] cols = (projection != null && projection.length > 0) ? projection : DEFAULT_PROJECTION;
        MatrixCursor cursor = new MatrixCursor(cols, 1);
        try {
            File target = file(uri);
            Object[] row = new Object[cols.length];
            for (int i = 0; i < cols.length; i++) {
                if (OpenableColumns.DISPLAY_NAME.equals(cols[i])) {
                    row[i] = target.getName();
                } else if (OpenableColumns.SIZE.equals(cols[i])) {
                    row[i] = target.length();
                } else {
                    row[i] = null;
                }
            }
            cursor.addRow(row);
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
