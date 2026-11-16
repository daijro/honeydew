package dev.daijro.honeydew;

import android.Manifest;
import android.app.Activity;
import android.content.pm.PackageManager;
import android.media.AudioAttributes;
import android.media.AudioDeviceCallback;
import android.media.AudioDeviceInfo;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.os.Build;

/**
 * Manages Android audio device routing, USB/wired/Bluetooth headphone detection,
 * hardware input enumeration, microphone permissions, and transient audio focus.
 */
final class HoneydewAudio {
    interface Listener {
        void onHeadphonesChanged(boolean connected);
        void onInterrupted();
    }

    static final int REQUEST_RECORD_AUDIO = 3;

    private final Activity activity;
    private final AudioManager audioManager;
    private final Listener listener;
    private AudioFocusRequest focusRequest;
    private boolean registered;

    private final AudioDeviceCallback outputCallback = new AudioDeviceCallback() {
        @Override
        public void onAudioDevicesAdded(AudioDeviceInfo[] addedDevices) {
            listener.onHeadphonesChanged(hasHeadphones());
        }

        @Override
        public void onAudioDevicesRemoved(AudioDeviceInfo[] removedDevices) {
            listener.onHeadphonesChanged(hasHeadphones());
        }
    };

    HoneydewAudio(Activity activity, Listener listener) {
        this.activity = activity;
        this.audioManager = activity.getSystemService(AudioManager.class);
        this.listener = listener;
    }

    void start() {
        if (audioManager == null || registered) return;
        audioManager.registerAudioDeviceCallback(outputCallback, null);
        registered = true;
        listener.onHeadphonesChanged(hasHeadphones());
    }

    void release() {
        if (audioManager == null) return;
        if (registered) {
            audioManager.unregisterAudioDeviceCallback(outputCallback);
            registered = false;
        }
        abandonFocus();
    }

    /**
     * Returns true when a headphone, headset, USB audio interface, or Bluetooth A2DP/BLE headset
     * output is connected, so microphone monitoring will not feed back into the phone speaker.
     */
    boolean hasHeadphones() {
        if (audioManager == null) return false;
        AudioDeviceInfo[] devices = audioManager.getDevices(AudioManager.GET_DEVICES_OUTPUTS);
        if (devices == null) return false;
        for (AudioDeviceInfo device : devices) {
            int type = device.getType();
            switch (type) {
                case AudioDeviceInfo.TYPE_WIRED_HEADSET:
                case AudioDeviceInfo.TYPE_WIRED_HEADPHONES:
                case AudioDeviceInfo.TYPE_USB_HEADSET:
                case AudioDeviceInfo.TYPE_USB_DEVICE:
                case AudioDeviceInfo.TYPE_BLUETOOTH_A2DP:
                case AudioDeviceInfo.TYPE_BLE_HEADSET:
                    return true;
                default:
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P
                            && type == AudioDeviceInfo.TYPE_HEARING_AID) {
                        return true;
                    }
                    break;
            }
        }
        return false;
    }

    /**
     * Lists available audio inputs as NUL-delimited "id\0label\0" pairs for the Settings tab.
     * Bluetooth SCO inputs are excluded because they require a voice-call link.
     */
    String listInputs() {
        if (audioManager == null) return "";
        AudioDeviceInfo[] devices = audioManager.getDevices(AudioManager.GET_DEVICES_INPUTS);
        if (devices == null) return "";
        StringBuilder list = new StringBuilder();
        for (AudioDeviceInfo device : devices) {
            String label;
            switch (device.getType()) {
                case AudioDeviceInfo.TYPE_BUILTIN_MIC:
                    String addr = device.getAddress();
                    label = (addr == null || addr.isEmpty() || "bottom".equals(addr))
                            ? "Built-in mic"
                            : "Built-in mic (" + addr + ")";
                    break;
                case AudioDeviceInfo.TYPE_WIRED_HEADSET:
                    label = "Headset mic";
                    break;
                case AudioDeviceInfo.TYPE_USB_DEVICE:
                case AudioDeviceInfo.TYPE_USB_HEADSET:
                case AudioDeviceInfo.TYPE_USB_ACCESSORY:
                    CharSequence product = device.getProductName();
                    String name = (product != null && product.length() > 0)
                            ? product.toString().replace('\0', ' ')
                            : ("Device " + device.getId());
                    label = "USB: " + name;
                    break;
                default:
                    continue;
            }
            list.append(device.getId()).append('\0').append(label).append('\0');
        }
        return list.toString();
    }

    boolean askMicPermission() {
        if (activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO)
                == PackageManager.PERMISSION_GRANTED) {
            return true;
        }
        activity.runOnUiThread(() -> {
            if (activity.isFinishing() || activity.isDestroyed()) return;
            activity.requestPermissions(
                    new String[] {Manifest.permission.RECORD_AUDIO},
                    REQUEST_RECORD_AUDIO);
        });
        return false;
    }

    void setRecordingFocus(boolean on) {
        activity.runOnUiThread(() -> {
            if (audioManager == null) return;
            abandonFocus();
            if (!on || activity.isFinishing() || activity.isDestroyed()) return;
            AudioAttributes attrs = new AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build();
            focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE)
                    .setAudioAttributes(attrs)
                    .setOnAudioFocusChangeListener(change -> {
                        if (change == AudioManager.AUDIOFOCUS_LOSS
                                || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) {
                            listener.onInterrupted();
                        }
                    })
                    .build();
            if (audioManager.requestAudioFocus(focusRequest) == AudioManager.AUDIOFOCUS_REQUEST_FAILED) {
                listener.onInterrupted();
            }
        });
    }

    private void abandonFocus() {
        if (audioManager != null && focusRequest != null) {
            audioManager.abandonAudioFocusRequest(focusRequest);
            focusRequest = null;
        }
    }
}
