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
        if let Some((current, _, _)) = &self.active
            && *current == serial
        {
            self.result = Some(text);
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
fn lock_editor() -> std::sync::MutexGuard<'static, Editor> {
    EDITOR.lock().unwrap_or_else(|err| err.into_inner())
}

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
        let mut editor = lock_editor();
        editor.last_focus = None;
        editor.cancel()
    };
    if let Some(serial) = serial {
        cancel_dialog(serial);
    }
}

#[cfg(target_os = "android")]
pub fn begin_frame(ctx: &Context) {
    let enter = {
        let mut editor = lock_editor();
        editor.seen = false;
        editor.enter.take()
    };
    if ctx.input(|i| i.focused)
        && let Some(field) = enter
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
    let mut editor = lock_editor();
    if editor
        .active
        .as_ref()
        .is_some_and(|(_, id, _)| *id == response.id)
    {
        // A native dialog legitimately takes focus away from NativeActivity.
        // Keep it alive while the owning field is still drawn; cancelling on
        // focus loss dismisses/reopens the IME while the user is typing.
        editor.seen = true;
        if let Some(result) = editor.take(response.id) {
            if let Some(text) = result {
                *value = text.chars().take(limit).collect();
                response.mark_changed();
                editor.enter = Some(response.id);
                drop(editor);
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
    let max_chars = i32::try_from(limit).unwrap_or(0);
    let result = crate::android::with_activity(|env, activity| {
        // The app thread stays attached to the VM, so release this local reference after every dialog call.
        let text = env.auto_local(env.new_string(value.as_str())?);
        env.call_method(
            activity,
            "editText",
            "(JLjava/lang/String;ZI)V",
            &[
                jni::objects::JValue::Long(serial as i64),
                jni::objects::JValue::Object(text.as_ref()),
                jni::objects::JValue::Bool(0),
                jni::objects::JValue::Int(max_chars),
            ],
        )?;
        Ok(())
    });
    if let Err(err) = result {
        eprintln!("Could not open text editor: {err}");
        lock_editor().complete(serial, None);
    }
}

#[cfg(target_os = "android")]
pub fn end_frame(ctx: &Context) {
    // The native dialog owns the IME; eframe must not open a second keyboard on NativeActivity's surface.
    ctx.output_mut(|o| o.ime = None);
    let focused = ctx.memory(|m| m.focused());
    let serial = {
        let mut editor = lock_editor();
        if editor.last_focus != focused {
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
pub extern "system" fn Java_dev_daijro_banaforge_BanaForgeActivity_onTextResult(
    mut env: jni::JNIEnv<'_>,
    _: jni::objects::JClass<'_>,
    serial: jni::sys::jlong,
    text: jni::objects::JString<'_>,
    accepted: jni::sys::jboolean,
) {
    let text = (accepted != 0 && !text.is_null())
        .then(|| env.get_string(&text).ok().map(String::from))
        .flatten();
    // Never acquire egui's context lock while holding EDITOR: the UI takes them
    // in the opposite order when consuming the result.
    let ctx = {
        let mut editor = lock_editor();
        editor.complete(serial as u64, text);
        editor.active.as_ref().map(|(_, _, ctx)| ctx.clone())
    };
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
}

#[cfg(not(target_os = "android"))]
pub fn edit(_: &mut eframe::egui::Response, _: &mut String, _: usize) {}

/// Consume native Done directly instead of injecting Enter into another editor/shortcut.
#[cfg(target_os = "android")]
pub fn take_commit(id: eframe::egui::Id) -> bool {
    let mut editor = lock_editor();
    if editor.enter == Some(id) {
        editor.enter = None;
        true
    } else {
        false
    }
}

#[cfg(not(target_os = "android"))]
pub fn take_commit(_: eframe::egui::Id) -> bool {
    false
}

/// Whether a dialog's text field takes the focus back whenever nothing has it, so that typing goes there: on the
/// desktop. On a phone a tap elsewhere drops the field and its keyboard (Android's text editor) until the field is
/// tapped again, and the dialog stays: focus put back would bring the keyboard straight back.
pub const REFOCUS: bool = cfg!(not(mobile));

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
