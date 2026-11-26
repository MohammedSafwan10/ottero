# Keep JNA native bridge and UniFFI generated bindings intact under R8
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.Structure { public <fields>; }
-keep class uniffi.ottero_mobile_ffi.** { *; }
-keep class com.nexdark.ottero.core.autofill.OtteroAutofillService { *; }

# Suppress optional JVM/desktop references inside JNA and OkHttp
-dontwarn java.awt.**
-dontwarn com.sun.jna.**
-dontwarn org.bouncycastle.**
-dontwarn org.conscrypt.**
-dontwarn org.openjsse.**
-dontwarn javax.annotation.**
