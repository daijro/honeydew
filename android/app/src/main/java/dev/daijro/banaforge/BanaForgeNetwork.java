package dev.daijro.banaforge;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.Uri;
import android.provider.Settings;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.ConnectException;
import java.net.HttpURLConnection;
import java.net.NoRouteToHostException;
import java.net.SocketTimeoutException;
import java.net.URL;
import java.net.UnknownHostException;

/**
 * Handles Android connectivity detection, hardened HTTPS resumable downloads
 * (with pre-connect TLS validation and leak-free redirect handling), and APK installation.
 */
final class BanaForgeNetwork {
    private static final int CONNECT_TIMEOUT_MS = 30_000;
    private static final int READ_TIMEOUT_MS = 60_000;
    private static final int MAX_REDIRECTS = 8;

    private final Activity activity;
    private final Context appContext;

    BanaForgeNetwork(Activity activity) {
        this.activity = activity;
        this.appContext = activity.getApplicationContext();
    }

    /**
     * Reports current validated network state:
     * 0 = offline / unvalidated captive portal, 1 = unmetered (Wi-Fi/Ethernet), 2 = metered.
     */
    int networkState() {
        ConnectivityManager cm = appContext.getSystemService(ConnectivityManager.class);
        if (cm == null) return 0;
        Network active = cm.getActiveNetwork();
        NetworkCapabilities caps = (active == null) ? null : cm.getNetworkCapabilities(active);
        if (caps == null
                || !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                || !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) {
            return 0;
        }
        return cm.isActiveNetworkMetered() ? 2 : 1;
    }

    /**
     * Synchronously downloads {@code rawUrl} into {@code path} (resuming via HTTP Range when
     * partial bytes already exist). Called on Rust's background worker thread.
     * Returns {@code null} on success, {@code "offline"} on transport loss, or an error message.
     */
    String download(String rawUrl, String path) {
        if (rawUrl == null || path == null) return "Invalid download arguments";
        File file = new File(path);
        File parent = file.getParentFile();
        if (parent != null && !parent.isDirectory() && !parent.mkdirs()) {
            return "Could not create download directory";
        }
        long have = file.isFile() ? file.length() : 0L;
        try {
            URL currentUrl = new URL(rawUrl);
            for (int hop = 0; hop <= MAX_REDIRECTS; hop++) {
                // Enforce HTTPS BEFORE opening any socket or sending headers.
                if (!"https".equalsIgnoreCase(currentUrl.getProtocol())) {
                    return "The download left HTTPS";
                }
                HttpURLConnection http = (HttpURLConnection) currentUrl.openConnection();
                try {
                    http.setInstanceFollowRedirects(false);
                    http.setConnectTimeout(CONNECT_TIMEOUT_MS);
                    http.setReadTimeout(READ_TIMEOUT_MS);
                    http.setRequestProperty("User-Agent", "BanaForge");
                    if (have > 0L) {
                        http.setRequestProperty("Range", "bytes=" + have + "-");
                    }
                    int code = http.getResponseCode();
                    if (isRedirect(code)) {
                        String location = http.getHeaderField("Location");
                        if (location == null || location.isEmpty()) {
                            return "Redirect without location";
                        }
                        currentUrl = new URL(currentUrl, location);
                        continue;
                    }
                    // Already complete on disk.
                    if (code == 416 && have > 0L) {
                        return null;
                    }
                    if (code != 200 && code != 206) {
                        return "The server answered " + code + " (" + http.getResponseMessage() + ")";
                    }
                    if (!"https".equalsIgnoreCase(http.getURL().getProtocol())) {
                        return "The download left HTTPS";
                    }
                    boolean append = (code == 206);
                    try (InputStream in = http.getInputStream();
                         OutputStream out = new FileOutputStream(file, append)) {
                        BanaForgeStorage.copyStream(in, out, Long.MAX_VALUE);
                    }
                    return null;
                } finally {
                    http.disconnect();
                }
            }
            return "Too many redirects";
        } catch (UnknownHostException | ConnectException | NoRouteToHostException | SocketTimeoutException e) {
            return "offline";
        } catch (IOException | RuntimeException e) {
            return BanaForgeStorage.safeMessage(e);
        }
    }

    boolean installApk(String name) {
        if (name == null || name.isEmpty()) return false;
        if (!appContext.getPackageManager().canRequestPackageInstalls()) {
            Uri appUri = Uri.parse("package:" + appContext.getPackageName());
            activity.runOnUiThread(() -> {
                if (activity.isFinishing() || activity.isDestroyed()) return;
                try {
                    activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, appUri));
                } catch (ActivityNotFoundException ignored) {
                }
            });
            return false;
        }
        Intent install = new Intent(Intent.ACTION_VIEW)
                .setDataAndType(BanaForgeUpdate.uri(appContext, name), BanaForgeUpdate.APK)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) return;
            try {
                activity.startActivity(install);
            } catch (ActivityNotFoundException ignored) {
            }
        });
        return true;
    }

    private static boolean isRedirect(int code) {
        return code == HttpURLConnection.HTTP_MOVED_PERM
                || code == HttpURLConnection.HTTP_MOVED_TEMP
                || code == HttpURLConnection.HTTP_SEE_OTHER
                || code == 307
                || code == 308;
    }
}
