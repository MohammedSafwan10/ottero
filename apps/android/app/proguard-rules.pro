# Keep JNA native bridge and UniFFI generated bindings intact under R8
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.ottero_mobile_ffi.** { *; }
-dontwarn java.awt.*
-dontwarn com.sun.jna.**
