# Android Bitmap media fixtures

These 3 x 2 fixtures were produced by Android 16 (API 36) `Bitmap.compress`, not by a generic image encoder.

## Regeneration

1. Compile and run the following program on an API 36 emulator:

   ```java
   import android.graphics.Bitmap;
   import android.graphics.Color;
   import android.graphics.ColorSpace;
   import java.io.FileOutputStream;

   public final class Main {
     private static void write(Bitmap bitmap, Bitmap.CompressFormat format, String stem)
         throws Exception {
       String extension = format == Bitmap.CompressFormat.PNG ? "png" : "jpg";
       try (FileOutputStream output =
           new FileOutputStream("/data/local/tmp/" + stem + "." + extension)) {
         if (!bitmap.compress(format, 100, output)) {
           throw new IllegalStateException("Bitmap.compress failed for " + stem);
         }
       }
     }

     public static void main(String[] args) throws Exception {
       Bitmap srgb = Bitmap.createBitmap(3, 2, Bitmap.Config.ARGB_8888);
       srgb.setPixels(new int[] {
         Color.argb(255, 255, 0, 0), Color.argb(255, 0, 255, 0),
         Color.argb(255, 0, 0, 255), Color.argb(128, 255, 255, 0),
         Color.argb(64, 0, 255, 255), Color.argb(0, 255, 0, 255),
       }, 0, 3, 0, 0, 3, 2);
       write(srgb, Bitmap.CompressFormat.PNG, "bitmap-srgb");
       write(srgb, Bitmap.CompressFormat.JPEG, "bitmap-srgb");

       Bitmap displayP3 = Bitmap.createBitmap(
           3, 2, Bitmap.Config.RGBA_F16, true,
           ColorSpace.get(ColorSpace.Named.DISPLAY_P3));
       displayP3.eraseColor(Color.pack(
           1.0f, 0.0f, 0.0f, 1.0f,
           ColorSpace.get(ColorSpace.Named.DISPLAY_P3)));
       write(displayP3, Bitmap.CompressFormat.PNG, "bitmap-display-p3");
       write(displayP3, Bitmap.CompressFormat.JPEG, "bitmap-display-p3");
     }
   }
   ```

2. Pull the four files from `/data/local/tmp/` into this directory.
3. Run `cargo test -p nimino-media android_` to verify the relay accepts every sanitized fixture while rejecting the unsanitized fixtures that contain forbidden metadata.

The sanitized counterparts are compatibility baselines. Replace an encoded and
sanitized pair only together, with a standalone sanitizer harness and a review
of the relay validation contract.
