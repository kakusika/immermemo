# Keep MainActivity and its JNI entry points
-keep class dev.immermemo.app.MainActivity {
    public *;
}

# Keep widget provider and service
-keep class dev.immermemo.app.widget.** { *; }

# Keep NativeActivity subclasses
-keep class * extends android.app.NativeActivity {
    <init>(...);
    public *;
}

# Keep native methods
-keepclasseswithmembernames class * {
    native <methods>;
}
