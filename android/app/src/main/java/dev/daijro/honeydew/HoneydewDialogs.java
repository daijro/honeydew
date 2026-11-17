package dev.daijro.honeydew;

import android.app.Activity;
import android.app.Dialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.graphics.Typeface;
import android.text.Editable;
import android.text.InputFilter;
import android.text.InputType;
import android.text.TextWatcher;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.HapticFeedbackConstants;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;

/**
 * Native Material 3 styled dialogs built with zero third-party UI dependencies.
 * Tracks every shown dialog so Activity teardown never triggers WindowLeaked crashes.
 */
final class HoneydewDialogs {
    interface TextCallback {
        void onTextResult(long requestId, String text, boolean accepted);
    }

    private final Activity activity;
    private final TextCallback textCallback;
    private Dialog activeDialog;
    private long activeRequestId;

    HoneydewDialogs(Activity activity, TextCallback textCallback) {
        this.activity = activity;
        this.textCallback = textCallback;
    }

    void showTextEditor(long requestId, String initialText, boolean multiline, int limit) {
        activity.runOnUiThread(() -> {
            String safeText = initialText == null ? "" : initialText;
            if (activity.isFinishing() || activity.isDestroyed()) {
                textCallback.onTextResult(requestId, safeText, false);
                return;
            }
            dismissActive();

            Dialog dialog = createM3Dialog();
            LinearLayout card = createM3Card();

            // Title (M3 Headline Small / Title Large)
            TextView title = new TextView(activity);
            title.setText(multiline ? "Edit notes" : "Edit text");
            title.setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f);
            title.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
            title.setTextColor(activity.getColor(R.color.m3_on_surface));
            card.addView(title);

            // Outlined M3 text field
            EditText input = new EditText(activity);
            input.setBackground(activity.getDrawable(R.drawable.bg_m3_textfield));
            input.setTextColor(activity.getColor(R.color.m3_on_surface));
            input.setHintTextColor(activity.getColor(R.color.m3_on_surface_variant));
            input.setHighlightColor(activity.getColor(R.color.m3_ripple_primary));
            input.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f);
            input.setPadding(dp(16), dp(14), dp(16), dp(14));
            input.setMinHeight(dp(52));
            input.setInputType(InputType.TYPE_CLASS_TEXT
                    | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
                    | (multiline ? InputType.TYPE_TEXT_FLAG_MULTI_LINE : 0));
            input.setSingleLine(!multiline);
            if (multiline) {
                input.setMinLines(3);
                input.setMaxLines(6);
                input.setGravity(Gravity.TOP | Gravity.START);
            }
            if (limit > 0) {
                input.setFilters(new InputFilter[] {new InputFilter.LengthFilter(limit)});
            }
            input.setImeOptions(EditorInfo.IME_FLAG_NO_FULLSCREEN
                    | (multiline ? EditorInfo.IME_ACTION_NONE : EditorInfo.IME_ACTION_DONE));
            input.setText(safeText);
            input.setSelection(input.length());

            LinearLayout.LayoutParams inputParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            inputParams.topMargin = dp(16);
            card.addView(input, inputParams);

            // Supporting row: live character counter (when bounded) + quick Clear action
            LinearLayout metaRow = new LinearLayout(activity);
            metaRow.setOrientation(LinearLayout.HORIZONTAL);
            metaRow.setGravity(Gravity.CENTER_VERTICAL);
            LinearLayout.LayoutParams metaParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            metaParams.topMargin = dp(6);

            TextView counter = new TextView(activity);
            counter.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f);
            counter.setTextColor(activity.getColor(R.color.m3_on_surface_variant));
            if (limit > 0) {
                counter.setText(safeText.length() + " / " + limit);
            }
            LinearLayout.LayoutParams counterParams = new LinearLayout.LayoutParams(
                    0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f);
            metaRow.addView(counter, counterParams);

            Button clearButton = createTextButton("Clear");
            clearButton.setVisibility(safeText.isEmpty() ? View.INVISIBLE : View.VISIBLE);
            clearButton.setMinimumHeight(dp(36));
            clearButton.setPadding(dp(12), dp(4), dp(12), dp(4));
            clearButton.setOnClickListener(v -> {
                v.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK);
                input.setText("");
            });
            metaRow.addView(clearButton);
            card.addView(metaRow, metaParams);

            input.addTextChangedListener(new TextWatcher() {
                @Override
                public void beforeTextChanged(CharSequence s, int start, int count, int after) {}

                @Override
                public void onTextChanged(CharSequence s, int start, int before, int count) {}

                @Override
                public void afterTextChanged(Editable s) {
                    int len = s == null ? 0 : s.length();
                    if (limit > 0) counter.setText(len + " / " + limit);
                    clearButton.setVisibility(len == 0 ? View.INVISIBLE : View.VISIBLE);
                }
            });

            // Action bar (Cancel text button + Done filled pill button, 48dp touch targets)
            boolean[] accepted = {false};
            LinearLayout actions = createActionRow();

            Button cancel = createTextButton("Cancel");
            cancel.setOnClickListener(v -> dialog.dismiss());
            actions.addView(cancel);

            Button done = createFilledButton("Done");
            LinearLayout.LayoutParams doneParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT, dp(44));
            doneParams.setMarginStart(dp(8));
            done.setOnClickListener(v -> {
                v.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP);
                accepted[0] = true;
                dialog.dismiss();
            });
            actions.addView(done, doneParams);
            card.addView(actions);

            input.setOnEditorActionListener((view, actionId, event) -> {
                if (!multiline && actionId == EditorInfo.IME_ACTION_DONE) {
                    view.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP);
                    accepted[0] = true;
                    dialog.dismiss();
                    return true;
                }
                return false;
            });

            dialog.setContentView(card);
            dialog.setOnDismissListener(d -> {
                if (activeDialog == dialog) {
                    activeDialog = null;
                    activeRequestId = 0;
                }
                Editable edited = input.getText();
                textCallback.onTextResult(requestId, edited == null ? "" : edited.toString(), accepted[0]);
            });

            activeDialog = dialog;
            activeRequestId = requestId;
            showM3Dialog(dialog, true);
            input.requestFocus();
            // Wait until the dialog's editor is attached before asking the Android IME.
            input.post(() -> {
                InputMethodManager imm = (InputMethodManager) activity.getSystemService(Activity.INPUT_METHOD_SERVICE);
                if (imm != null && input.isAttachedToWindow()) {
                    imm.showSoftInput(input, InputMethodManager.SHOW_IMPLICIT);
                }
            });
        });
    }

    void cancelTextEditor(long requestId) {
        activity.runOnUiThread(() -> {
            if (activeDialog != null && activeRequestId == requestId) {
                activeDialog.dismiss();
            }
        });
    }

    void showFolderFallback(String path) {
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) return;
            dismissActive();

            Dialog dialog = createM3Dialog();
            LinearLayout card = createM3Card();

            TextView title = new TextView(activity);
            title.setText("User folder location");
            title.setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f);
            title.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
            title.setTextColor(activity.getColor(R.color.m3_on_surface));
            card.addView(title);

            TextView body = new TextView(activity);
            body.setText("No file manager on this device handled opening the folder directly.\n\n" + path);
            body.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f);
            body.setLineSpacing(dp(3), 1.0f);
            body.setTextColor(activity.getColor(R.color.m3_on_surface_variant));
            body.setTextIsSelectable(true);
            LinearLayout.LayoutParams bodyParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            bodyParams.topMargin = dp(12);
            card.addView(body, bodyParams);

            LinearLayout actions = createActionRow();

            Button copy = createTextButton("Copy path");
            copy.setOnClickListener(v -> {
                ClipboardManager cm = activity.getSystemService(ClipboardManager.class);
                if (cm != null) {
                    cm.setPrimaryClip(ClipData.newPlainText("Honeydew folder", path));
                    Toast.makeText(activity, "Folder path copied", Toast.LENGTH_SHORT).show();
                }
            });
            actions.addView(copy);

            Button ok = createFilledButton("OK");
            LinearLayout.LayoutParams okParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT, dp(44));
            okParams.setMarginStart(dp(8));
            ok.setOnClickListener(v -> dialog.dismiss());
            actions.addView(ok, okParams);

            card.addView(actions);
            dialog.setContentView(card);
            dialog.setOnDismissListener(d -> {
                if (activeDialog == dialog) {
                    activeDialog = null;
                    activeRequestId = 0;
                }
            });

            activeDialog = dialog;
            activeRequestId = 0;
            showM3Dialog(dialog, false);
        });
    }

    void dismissAll() {
        dismissActive();
    }

    private void dismissActive() {
        if (activeDialog != null) {
            try {
                activeDialog.dismiss();
            } catch (RuntimeException ignored) {
            }
            activeDialog = null;
            activeRequestId = 0;
        }
    }

    private Dialog createM3Dialog() {
        Dialog dialog = new Dialog(activity, R.style.Theme_Honeydew_Dialog);
        dialog.setCanceledOnTouchOutside(true);
        return dialog;
    }

    private void showM3Dialog(Dialog dialog, boolean showKeyboard) {
        dialog.show();
        Window window = dialog.getWindow();
        if (window == null) return;
        int screenWidth = activity.getResources().getDisplayMetrics().widthPixels;
        int targetWidth = Math.min(screenWidth - dp(40), dp(460));
        window.setLayout(targetWidth, ViewGroup.LayoutParams.WRAP_CONTENT);
        window.setGravity(Gravity.CENTER);
        if (showKeyboard) {
            window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE
                    | WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        }
    }

    private LinearLayout createM3Card() {
        LinearLayout card = new LinearLayout(activity);
        card.setOrientation(LinearLayout.VERTICAL);
        card.setBackground(activity.getDrawable(R.drawable.bg_m3_dialog));
        card.setPadding(dp(24), dp(22), dp(24), dp(18));
        card.setElevation(dp(8));
        return card;
    }

    private LinearLayout createActionRow() {
        LinearLayout actions = new LinearLayout(activity);
        actions.setOrientation(LinearLayout.HORIZONTAL);
        actions.setGravity(Gravity.END | Gravity.CENTER_VERTICAL);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        params.topMargin = dp(16);
        actions.setLayoutParams(params);
        return actions;
    }

    private Button createFilledButton(String label) {
        Button btn = new Button(activity, null, 0);
        btn.setText(label);
        btn.setAllCaps(false);
        btn.setGravity(Gravity.CENTER);
        btn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f);
        btn.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        btn.setTextColor(activity.getColor(R.color.m3_on_primary));
        btn.setBackground(activity.getDrawable(R.drawable.bg_m3_button_filled));
        btn.setPadding(dp(22), dp(10), dp(22), dp(10));
        btn.setMinWidth(dp(72));
        btn.setMinimumHeight(dp(44));
        return btn;
    }

    private Button createTextButton(String label) {
        Button btn = new Button(activity, null, 0);
        btn.setText(label);
        btn.setAllCaps(false);
        btn.setGravity(Gravity.CENTER);
        btn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f);
        btn.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        btn.setTextColor(activity.getColor(R.color.m3_primary));
        btn.setBackground(activity.getDrawable(R.drawable.bg_m3_button_text));
        btn.setPadding(dp(16), dp(10), dp(16), dp(10));
        btn.setMinWidth(dp(64));
        btn.setMinimumHeight(dp(44));
        return btn;
    }

    private int dp(int value) {
        return Math.round(TypedValue.applyDimension(
                TypedValue.COMPLEX_UNIT_DIP, value, activity.getResources().getDisplayMetrics()));
    }
}
