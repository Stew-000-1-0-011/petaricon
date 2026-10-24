# PETARICON changes to i-slint-backend-android-activity 1.18.1

This directory is a copy of the crates.io `i-slint-backend-android-activity` 1.18.1 (only the
registry marker `.cargo-ok` removed). It replaces the registry crate via `[patch.crates-io]` in the
root Cargo.toml; `slint` stays pinned to `=1.18.1`.

## Why

On an Android phone with a Japanese IME, editing a Text property inserted the composing characters
many times before they were confirmed: `SlintInputView.setText` replaces the whole `Editable` when
the text changed, which drops the IME composing span. The product decision is that only an English
(ASCII) keyboard is supported, so the editor asks the IME for a non-composing Latin layout instead
of fixing composition. Pasting (including Japanese text) still works, because a paste is committed
without composition. Nothing is stripped on the Rust side.

## Change

Only `java/SlintAndroidJavaHelper.java`, `SlintInputView.onCreateInputConnection` (the EditorInfo).
For `TYPE_CLASS_TEXT` fields that are not already a password variation the EditorInfo asks for
`TYPE_TEXT_VARIATION_VISIBLE_PASSWORD | TYPE_TEXT_FLAG_NO_SUGGESTIONS` (no auto-correct, auto-complete
or capitalisation flags) and adds `IME_FLAG_FORCE_ASCII`. Password fields keep their type and only
get `IME_FLAG_FORCE_ASCII`. Number/decimal fields are unchanged. `mInputType` is deliberately NOT
changed: `setText` compares it with the type Slint sends, so changing it would restart the input
endlessly. The mapping is applied only where the EditorInfo is built.

Re-apply on a newer Slint: copy the new crate over this directory and redo the diff below.

## Diff against upstream 1.18.1

```diff
--- a/java/SlintAndroidJavaHelper.java
+++ b/java/SlintAndroidJavaHelper.java
@@ -176,8 +176,27 @@
 
     @Override
     public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
-        outAttrs.inputType = mInputType;
-        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI;
+        // PETARICON: ask for a non-composing ASCII keyboard (see PETARICON.md).
+        // Only the EditorInfo is changed; mInputType stays as Slint set it, so
+        // setText() does not see a type change and does not restart the input.
+        int inputType = mInputType;
+        int imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI;
+        int inputClass = inputType & android.text.InputType.TYPE_MASK_CLASS;
+        int variation = inputType & android.text.InputType.TYPE_MASK_VARIATION;
+        if (inputClass == android.text.InputType.TYPE_CLASS_TEXT) {
+            boolean isPassword =
+                variation == android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
+                || variation == android.text.InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD
+                || variation == android.text.InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD;
+            if (!isPassword) {
+                inputType = android.text.InputType.TYPE_CLASS_TEXT
+                    | android.text.InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
+                    | android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
+            }
+            imeOptions |= EditorInfo.IME_FLAG_FORCE_ASCII;
+        }
+        outAttrs.inputType = inputType;
+        outAttrs.imeOptions = imeOptions;
         outAttrs.initialSelStart = mCursorPosition;
         outAttrs.initialSelEnd = mAnchorPosition;
         return new BaseInputConnection(this, true) {
```
