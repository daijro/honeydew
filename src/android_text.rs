//! Standard Android text editor for NativeActivity, whose winit backend has no composed-text events.

#[cfg(any(target_os = "android", test))]
use eframe::egui::{Context, Id};

#[cfg(any(target_os = "android", test))]
#[derive(Default)]
struct Editor {
    serial: u64,
    active: Option<(u64, Id, Context)>,
    result: Option<Option<String>>,
    #[cfg(target_os = "android")]
    last_focus: Option<Id>,
    #[cfg(target_os = "android")]
    enter: Option<Id>,
    #[cfg(target_os = "android")]
    seen: bool,
}

#[cfg(any(target_os = "android", test))]
impl Editor {
    fn start(&mut self, field: Id, ctx: &Context) -> u64 {
        self.serial += 1;
        self.active = Some((self.serial, field, ctx.clone()));
        self.result = None;
        #[cfg(target_os = "android")]
        {
            self.seen = true;
        }
        self.serial
    }

    fn complete(&mut self, serial: u64, text: Option<String>) {
        if let Some((current, _, ctx)) = &self.active
            && *current == serial
        {
            self.result = Some(text);
            ctx.request_repaint();
        }
    }

    fn take(&mut self, field: Id) -> Option<Option<String>> {
        if self.active.as_ref().is_some_and(|(_, id, _)| *id == field) {
            let result = self.result.take()?;
            self.active = None;
            Some(result)
        } else {
            None
        }
    }

    fn cancel(&mut self) -> Option<u64> {
        self.result = None;
        #[cfg(target_os = "android")]
        {
            self.enter = None;
        }
        self.active.take().map(|(serial, _, _)| serial)
    }
}

#[cfg(target_os = "android")]
use eframe::egui::{Event, Key, Modifiers, Response};
#[cfg(target_os = "android")]
use std::sync::Mutex;
#[cfg(target_os = "android")]
static EDITOR: Mutex<Editor> = Mutex::new(Editor {
    serial: 0,
    active: None,
    result: None,
    last_focus: None,
    enter: None,
    seen: false,
});

#[cfg(target_os = "android")]
fn cancel_dialog(serial: u64) {
    let _ = crate::android::with_activity(|env, activity| {
        env.call_method(
            activity,
            "cancelTextEdit",
            "(J)V",
            &[jni::objects::JValue::Long(serial as i64)],
        )?;
        Ok(())
    });
}

#[cfg(target_os = "android")]
pub fn reset() {
    let serial = {
        let mut editor = EDITOR.lock().unwrap();
        editor.last_focus = None;
        editor.cancel()
    };
    if let Some(serial) = serial {
        cancel_dialog(serial);
    }
}

#[cfg(target_os = "android")]
pub fn begin_frame(ctx: &Context) {
    let mut editor = EDITOR.lock().unwrap();
    editor.seen = false;
    if ctx.input(|i| i.focused)
        && let Some(field) = editor.enter.take()
        && ctx.memory(|m| m.has_focus(field))
    {
        ctx.input_mut(|i| {
            i.events.push(Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            })
        });
    }
}

/// Call after each text field is drawn, before its changed/Enter handling.
#[cfg(target_os = "android")]
pub fn edit(response: &mut Response, value: &mut String, limit: usize) {
    let ctx = response.ctx.clone();
    let focused = ctx.memory(|m| m.has_focus(response.id));
    let mut editor = EDITOR.lock().unwrap();
    if editor
        .active
        .as_ref()
        .is_some_and(|(_, id, _)| *id == response.id)
    {
        editor.seen = focused;
        if focused && let Some(result) = editor.take(response.id) {
            if let Some(text) = result {
                *value = text.chars().take(limit).collect();
                response.mark_changed();
                editor.enter = Some(response.id);
                response.ctx.request_repaint();
            }
            return;
        }
    }
    if !response.enabled()
        || !focused
        || editor.active.is_some()
        || !(response.clicked() || editor.last_focus != Some(response.id))
    {
        return;
    }
    editor.last_focus = Some(response.id);
    let serial = editor.start(response.id, &ctx);
    drop(editor);
    let result = crate::android::with_activity(|env, activity| {
        // The app thread stays attached to the VM, so release this local reference after every dialog call.
        let text = env.auto_local(env.new_string(value.as_str())?);
        env.call_method(
            activity,
            "editText",
            "(JLjava/lang/String;Z)V",
            &[
                jni::objects::JValue::Long(serial as i64),
                jni::objects::JValue::Object(text.as_ref()),
                jni::objects::JValue::Bool(0),
            ],
        )?;
        Ok(())
    });
    if let Err(err) = result {
        eprintln!("Could not open text editor: {err}");
        EDITOR.lock().unwrap().complete(serial, None);
    }
}

#[cfg(target_os = "android")]
pub fn end_frame(ctx: &Context) {
    // The native dialog owns the IME; eframe must not open a second keyboard on NativeActivity's surface.
    ctx.output_mut(|o| o.ime = None);
    let serial = {
        let mut editor = EDITOR.lock().unwrap();
        if editor.last_focus != ctx.memory(|m| m.focused()) {
            editor.last_focus = None;
        }
        if editor.active.is_some() && !editor.seen {
            editor.cancel()
        } else {
            None
        }
    };
    if let Some(serial) = serial {
        cancel_dialog(serial);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onTextResult(
    mut env: jni::JNIEnv<'_>,
    _: jni::objects::JClass<'_>,
    serial: jni::sys::jlong,
    text: jni::objects::JString<'_>,
    accepted: jni::sys::jboolean,
) {
    let text = (accepted != 0)
        .then(|| env.get_string(&text).ok().map(String::from))
        .flatten();
    if let Ok(mut editor) = EDITOR.lock() {
        editor.complete(serial as u64, text);
    }
}

#[cfg(not(target_os = "android"))]
pub fn edit(_: &mut eframe::egui::Response, _: &mut String, _: usize) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_results_only_reach_their_request_and_cancel_never_changes_a_field() {
        let mut editor = Editor::default();
        let ctx = Context::default();
        let a = Id::new("song");
        let b = Id::new("channel");
        let old = editor.start(a, &ctx);
        editor.cancel();
        let current = editor.start(b, &ctx);
        editor.complete(old, Some("stale".into()));
        assert!(editor.take(b).is_none());
        editor.complete(current, Some("鍵盤 🎹 café".into()));
        assert!(editor.take(a).is_none());
        assert_eq!(editor.take(b), Some(Some("鍵盤 🎹 café".into())));
        let cancelled = editor.start(a, &ctx);
        editor.complete(cancelled, None);
        assert_eq!(editor.take(a), Some(None));
        assert!(editor.take(a).is_none());
    }
}
