# ==============================================================================
# Honeydew Android R8 / ProGuard Configuration
# ==============================================================================
# Keep stack trace line numbers for crash diagnostics
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile

# Keep all native method declarations and their declaring classes intact
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}

# Keep HoneydewActivity and EVERY method invoked via JNI from Rust
# (src/android.rs & src/android_text.rs)
-keep,includedescriptorclasses class dev.daijro.honeydew.HoneydewActivity {
    public <init>();
    static native <methods>;
    public *** askMic();
    public *** inputs();
    public *** recording(boolean);
    public *** openDocument(long);
    public *** openFolder();
    public *** copyFiles(java.lang.String);
    public *** requestOrientation(int);
    public *** showFolder(java.lang.String);
    public *** share(java.lang.String, java.lang.String);
    public *** network();
    public *** download(java.lang.String, java.lang.String);
    public *** usableSpace();
    public *** installApk(java.lang.String);
    public *** editText(long, java.lang.String, boolean);
    public *** editText(long, java.lang.String, boolean, int);
    public *** cancelTextEdit(long);
    public *** quitApp();
    public *** setKeepScreenOn(boolean);
    public *** hapticTick();
}

# Keep AndroidManifest ContentProviders & DocumentsProviders
-keep public class dev.daijro.honeydew.HoneydewDocuments extends android.provider.DocumentsProvider {
    public <init>();
}
-keep public class dev.daijro.honeydew.HoneydewUpdate extends android.content.ContentProvider {
    public <init>();
}

# Strip verbose / debug / info logs in release builds while keeping warn & error
-assumenosideeffects class android.util.Log {
    public static int v(...);
    public static int d(...);
    public static int i(...);
}
