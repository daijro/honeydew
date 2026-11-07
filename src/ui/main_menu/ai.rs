//! The AI tab: connection setup, live model catalog, session chat and explicit song proposals.
use crate::ai::Config;
use crate::metrics::FontSize;
use crate::ui::UiCtx;
use eframe::egui::{self, Rect, Ui};

/// Stock egui controls otherwise inherit its light, fixed-size defaults while the menu is custom painted.
/// Scope typography and surfaces to this page; combo menus receive an explicit copy of this style.
fn page_style(ui: &mut Ui, cx: &UiCtx) {
    let colors = &cx.theme.colors;
    let text = colors.text.to_color32();
    let control = colors.control_fill.to_color32();
    let background = colors.background.to_color32();
    let accent = colors.accent.to_color32();
    let style = ui.style_mut();
    for kind in [
        egui::TextStyle::Body,
        egui::TextStyle::Button,
        egui::TextStyle::Small,
        egui::TextStyle::Monospace,
    ] {
        style.text_styles.insert(kind, cx.m.font_id(FontSize::Pt10));
    }
    style
        .text_styles
        .insert(egui::TextStyle::Heading, cx.m.font_id(FontSize::Pt12));
    style.spacing.item_spacing = egui::vec2(cx.m.u(0.25), cx.m.u(0.25));
    style.spacing.button_padding = egui::vec2(cx.m.u(0.375), cx.m.u(0.2));
    style.spacing.interact_size.y = cx.m.u(1.25);
    style.spacing.icon_width = cx.m.u(0.65);
    style.spacing.icon_width_inner = cx.m.u(0.4);
    style.spacing.icon_spacing = cx.m.u(0.2);
    style.spacing.indent = cx.m.u(0.6);
    let visuals = &mut style.visuals;
    *visuals = if colors.is_dark() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(text);
    visuals.weak_text_color = Some(text);
    visuals.hyperlink_color = accent;
    visuals.panel_fill = colors.menu_fill.to_color32();
    visuals.window_fill = colors.module_fill.to_color32();
    visuals.extreme_bg_color = background;
    visuals.faint_bg_color = control;
    visuals.text_edit_bg_color = Some(background);
    visuals.selection.bg_fill = accent;
    visuals.selection.stroke = egui::Stroke::new(cx.m.t(), colors.text_on_accent.to_color32());
    visuals.disabled_alpha = 0.65;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = control;
        widget.weak_bg_fill = control;
        widget.fg_stroke = egui::Stroke::new(cx.m.t(), text);
        widget.bg_stroke = egui::Stroke::new(cx.m.t(), text.gamma_multiply(0.35));
    }
    visuals.widgets.hovered.bg_stroke.color = accent;
    visuals.widgets.active.bg_stroke.color = accent;
    visuals.widgets.open.bg_stroke.color = accent;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(cx.m.t(), text);
    visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(cx.m.t(), text.gamma_multiply(0.35));
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use crate::ai::{
        Answer, Message, Profile, Provider,
        client::{self, Audio},
        credentials::{self, Operation},
        plan,
    };
    use serde_json::{Value, json};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct Row {
        message: Message,
        proposal: Option<Value>,
        applied: bool,
    }
    enum ResultKind {
        Models(Result<Vec<String>, String>),
        Chat(Result<Answer, String>),
        Audio(Result<Option<Audio>, String>),
        Credential(Operation, Result<Option<String>, String>),
        PlanBegin(Result<plan::Login, String>),
        PlanLogin(Result<plan::Account, String>),
        PlanRestore(String, Result<(), String>),
        PlanRemember(String, bool, Result<(), String>),
        PlanSignOut(String, Result<String, String>),
        PlanModels(Result<Vec<plan::Model>, String>),
    }
    struct Job {
        receive: Receiver<ResultKind>,
        source: Profile,
    }
    #[derive(Default)]
    pub(crate) struct AiTab {
        pub context: Value,
        prompt: String,
        rows: Vec<Row>,
        audio: Option<Audio>,
        job: Option<Job>,
        models: Vec<String>,
        manual_model: bool,
        model_names: std::collections::HashMap<String, String>,
        plan_ready: Option<plan::Login>,
        plan_result: Option<ResultKind>,
        plan_cancel: Option<Arc<AtomicBool>>,
        remember_plan: bool,
        selected_plan_ui: String,
        catalog_source: Option<Profile>,
        status: String,
        apply: Option<(usize, Value)>,
        applying: Option<usize>,
        pub apply_result: Option<Result<String, String>>,
        scroll_bottom: bool,
        credential_result: Option<(Profile, Operation, Result<Option<String>, String>)>,
    }
    impl AiTab {
        pub fn take_apply(&mut self) -> Option<Value> {
            self.apply.take().map(|(i, v)| {
                self.applying = Some(i);
                v
            })
        }
        fn poll(&mut self) {
            if let Some(job) = &self.job {
                match job.receive.try_recv() {
                    Ok(result) => {
                        let source = job.source.clone();
                        self.job = None;
                        match result {
                            ResultKind::Models(Ok(models)) => {
                                self.status = format!(
                                    "Loaded {} models. Select a chat model; audio support varies.",
                                    models.len()
                                );
                                self.models = models;
                                self.manual_model = false;
                                self.catalog_source = Some(source);
                            }
                            ResultKind::Chat(Ok(answer)) => {
                                self.rows.push(Row {
                                    message: Message {
                                        assistant: true,
                                        text: answer.text,
                                    },
                                    proposal: answer.arrangement,
                                    applied: false,
                                });
                                self.status.clear();
                                self.audio = None;
                                self.scroll_bottom = true;
                            }
                            ResultKind::Audio(Ok(audio)) => {
                                self.audio = audio;
                                self.status.clear();
                            }
                            ResultKind::Chat(Err(e)) => {
                                self.status = e;
                                if let Some(row) = self.rows.pop_if(|r| !r.message.assistant) {
                                    self.prompt = row.message.text;
                                }
                            }
                            ResultKind::Credential(operation, result) => {
                                self.credential_result = Some((source, operation, result));
                            }
                            ResultKind::PlanBegin(Ok(login)) => {
                                self.plan_cancel = Some(Arc::clone(&login.cancel));
                                self.plan_ready = Some(login);
                            }
                            ResultKind::PlanModels(Ok(models)) => {
                                self.models = models.iter().map(|m| m.slug.clone()).collect();
                                self.model_names =
                                    models.into_iter().map(|m| (m.slug, m.name)).collect();
                                self.catalog_source = Some(source);
                                self.status =
                                    "ChatGPT account models loaded. Choose a model to chat.".into();
                            }
                            ResultKind::PlanBegin(Err(e)) | ResultKind::PlanModels(Err(e)) => {
                                self.status = e
                            }
                            outcome @ (ResultKind::PlanLogin(_)
                            | ResultKind::PlanRestore(..)
                            | ResultKind::PlanRemember(..)
                            | ResultKind::PlanSignOut(..)) => self.plan_result = Some(outcome),
                            ResultKind::Models(Err(e)) | ResultKind::Audio(Err(e)) => {
                                self.status = e
                            }
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.job = None;
                        self.status = "The AI worker stopped. Try again.".into();
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if let Some(result) = self.apply_result.take() {
                let index = self.applying.take();
                match result {
                    Ok(text) => {
                        self.status = text;
                        if let Some(row) = index.and_then(|i| self.rows.get_mut(i)) {
                            row.applied = true;
                        }
                    }
                    Err(e) => self.status = e,
                }
            }
        }
        fn start(
            &mut self,
            ctx: &egui::Context,
            source: Profile,
            work: impl FnOnce() -> ResultKind + Send + 'static,
        ) {
            if self.job.is_some() {
                return;
            }
            let (send, receive) = mpsc::channel();
            let ctx = ctx.clone();
            match std::thread::Builder::new()
                .name("honeydew-ai".into())
                .spawn(move || {
                    let _ = send.send(work());
                    ctx.request_repaint();
                }) {
                Ok(_) => self.job = Some(Job { receive, source }),
                Err(_) => self.status = "Could not start the AI worker.".into(),
            }
        }
        fn attach(&mut self, ctx: &egui::Context, profile: Profile) {
            let (send, receive) = mpsc::channel();
            let wake = ctx.clone();
            self.job = Some(Job {
                receive,
                source: profile,
            });
            #[cfg(target_os = "ios")]
            crate::ios::pick(false, move |picked| {
                std::thread::spawn(move || {
                    let result = picked
                        .map(|path| {
                            let result = Audio::read(&path);
                            let _ = std::fs::remove_file(path);
                            result
                        })
                        .transpose();
                    let _ = send.send(ResultKind::Audio(result));
                    wake.request_repaint();
                });
            });
            #[cfg(target_os = "android")]
            crate::android::pick_document(client::MAX_AUDIO as u64, move |picked| {
                std::thread::spawn(move || {
                    let result = picked
                        .map(|p| {
                            p.map_err(|e| e.to_string()).and_then(|path| {
                                let result = Audio::read(&path);
                                let _ = std::fs::remove_file(path);
                                result
                            })
                        })
                        .transpose();
                    let _ = send.send(ResultKind::Audio(result));
                    wake.request_repaint();
                });
            });
            #[cfg(not(mobile))]
            std::thread::spawn(move || {
                let result = pollster::block_on(
                    rfd::AsyncFileDialog::new()
                        .add_filter("Reference audio", &["wav", "mp3"])
                        .pick_file(),
                )
                .map(|file| Audio::read(file.path()))
                .transpose();
                let _ = send.send(ResultKind::Audio(result));
                wake.request_repaint();
            });
        }
        fn start_key(&mut self, ctx: &egui::Context, profile: &mut Profile, operation: Operation) {
            if self.job.is_some() {
                return;
            }
            profile.key_state.attempted_load = true;
            profile.key_state.notice = match operation {
                Operation::Load => "Opening secure storage…",
                Operation::Save => "Saving key securely…",
                Operation::Delete => "Removing saved key…",
            }
            .into();
            let source = profile.clone();
            self.start(ctx, profile.clone(), move || {
                ResultKind::Credential(
                    operation,
                    credentials::perform(&credentials::NativeStore, operation, &source),
                )
            });
        }

        fn sync_key(&mut self, ctx: &egui::Context, profile: &mut Profile) {
            if credentials::prepare(profile) {
                self.models.clear();
                self.model_names.clear();
                self.catalog_source = None;
            }
            if credentials::supported()
                && profile.remember_key
                && !profile.key_state.attempted_load
                && profile.api_key.is_empty()
                && self.job.is_none()
            {
                self.start_key(ctx, profile, Operation::Load);
            }
        }

        fn key_settings(&mut self, ui: &mut Ui, profile: &mut Profile) {
            self.sync_key(ui.ctx(), profile);
            ui.label("API key");
            ui.add_enabled(
                self.job.is_none(),
                egui::TextEdit::singleline(&mut profile.api_key)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );
            if credentials::supported() {
                let mut remember = profile.remember_key;
                let can_remember =
                    self.job.is_none() && (remember || !profile.api_key.trim().is_empty());
                if ui
                    .add_enabled(
                        can_remember,
                        egui::Checkbox::new(&mut remember, "Remember API key securely"),
                    )
                    .changed()
                {
                    if !remember {
                        profile.remember_key = false;
                    }
                    self.start_key(
                        ui.ctx(),
                        profile,
                        if remember {
                            Operation::Save
                        } else {
                            Operation::Delete
                        },
                    );
                }
                ui.small(format!(
                    "Saved keys use {} and are bound to this provider and API URL.",
                    credentials::store_name()
                ));
                if profile.remember_key
                    && !profile.api_key.is_empty()
                    && profile.key_state.saved_hash.as_deref()
                        != Some(credentials::fingerprint(&profile.api_key).as_str())
                {
                    ui.small("The key in memory differs from the saved copy. Save the update to keep it after restart.");
                    if ui
                        .add_enabled(self.job.is_none(), egui::Button::new("Update saved key"))
                        .clicked()
                    {
                        self.start_key(ui.ctx(), profile, Operation::Save);
                    }
                }
                if ui
                    .add_enabled(self.job.is_none(), egui::Button::new("Load saved key"))
                    .clicked()
                {
                    self.start_key(ui.ctx(), profile, Operation::Load);
                }
            } else {
                profile.remember_key = false;
                ui.small("Secure key storage is unavailable on this platform. Keys stay in memory for this session only.");
            }
            if ui
                .add_enabled(self.job.is_none(), egui::Button::new("Forget API key"))
                .clicked()
            {
                profile.api_key.clear();
                profile.remember_key = false;
                self.models.clear();
                self.model_names.clear();
                self.catalog_source = None;
                if credentials::supported() {
                    self.start_key(ui.ctx(), profile, Operation::Delete);
                } else {
                    profile.remember_key = false;
                    profile.key_state.notice = "API key cleared from this session.".into();
                }
            }
            if !profile.key_state.notice.is_empty() {
                ui.label(&profile.key_state.notice);
            }
        }

        fn setup(&mut self, ui: &mut Ui, config: &mut Config, width: f32) {
            if config.selected == Provider::ChatGpt {
                self.sync_plan(ui.ctx(), config.profile());
            } else {
                self.sync_key(ui.ctx(), config.profile());
            }
            egui::CollapsingHeader::new("Provider setup")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_enabled_ui(self.job.is_none(), |ui| {
                        egui::ComboBox::from_id_salt("ai_provider")
                            .popup_style((**ui.style()).clone().into())
                            .selected_text(config.selected.label())
                            .show_ui(ui, |ui| {
                                for provider in Provider::ALL {
                                    ui.selectable_value(&mut config.selected, provider, provider.label());
                                }
                            });
                        let profile = config.profile();
                        if profile.provider == Provider::ChatGpt {
                            self.plan_settings(ui, profile);
                        } else {
                            ui.label("API base URL");
                            ui.add(egui::TextEdit::singleline(&mut profile.base_url).desired_width(f32::INFINITY));
                            self.key_settings(ui, profile);
                        }
                        if self.catalog_source.as_ref().is_some_and(|source| {
                            source.provider != profile.provider || source.base_url != profile.base_url || source.api_key != profile.api_key || source.plan.selected != profile.plan.selected
                        }) {
                            self.models.clear();
                self.model_names.clear();
                            self.catalog_source = None;
                        }
                        if ui.button("Refresh models").clicked() {
                            self.models.clear();
                self.model_names.clear();
                            let p = profile.clone();
                            self.status = "Fetching models…".into();
                            if profile.provider == Provider::ChatGpt {
                                self.start(ui.ctx(), profile.clone(), move || ResultKind::PlanModels(plan::models(&p.plan)));
                            } else {
                                self.start(ui.ctx(), profile.clone(), move || ResultKind::Models(client::models(&p)));
                            }
                        }
                        self.model_selector(ui, profile, width);
                        if profile.provider.supports_audio() {
                            ui.checkbox(&mut profile.audio_input, "This model accepts audio input");
                        } else {
                            profile.audio_input = false;
                            ui.label("This adapter supports text chat only.");
                        }
                    });
                    if config.selected != Provider::ChatGpt {
                        ui.small("Use a provider API key with API billing/access. A chat subscription is not an API key.");
                    }
                });
        }

        fn begin_plan(
            &mut self,
            ctx: &egui::Context,
            profile: &Profile,
            account: Option<plan::Account>,
        ) {
            if self.job.is_some() {
                return;
            }
            let remember = self.remember_plan;
            self.status = "Preparing ChatGPT sign-in…".into();
            self.start(ctx, profile.clone(), move || {
                ResultKind::PlanBegin(plan::begin(account, remember))
            });
        }
        fn sync_plan(&mut self, ctx: &egui::Context, profile: &mut Profile) {
            if self.selected_plan_ui != profile.plan.selected {
                self.selected_plan_ui = profile.plan.selected.clone();
                self.remember_plan = profile.plan.active().is_some_and(|a| a.remember);
                self.models.clear();
                self.model_names.clear();
                self.catalog_source = None;
            }
            if let Some(a) = profile.plan.active()
                && a.remember
                && !a.signed_in()
                && !a.attempted_load()
                && self.job.is_none()
            {
                a.mark_loading();
                let account = a.clone();
                let id = a.client_id.clone();
                self.status = "Opening remembered ChatGPT session…".into();
                self.start(ctx, profile.clone(), move || {
                    ResultKind::PlanRestore(id, plan::restore(&account))
                });
            }
        }
        fn plan_settings(&mut self, ui: &mut Ui, profile: &mut Profile) {
            self.sync_plan(ui.ctx(), profile);
            ui.small("Use an eligible ChatGPT plan's shared allowance. OpenAI will ask you to approve access in its browser sign-in. This mode does not use API-key billing.");
            let before = profile.plan.selected.clone();
            egui::ComboBox::from_id_salt("dewey_chatgpt_account")
                .popup_style((**ui.style()).clone().into())
                .selected_text(
                    profile
                        .plan
                        .active()
                        .map(|a| a.label())
                        .unwrap_or_else(|| "Add a ChatGPT account".into()),
                )
                .show_ui(ui, |ui| {
                    for a in &profile.plan.accounts {
                        ui.selectable_value(
                            &mut profile.plan.selected,
                            a.client_id.clone(),
                            a.label(),
                        );
                    }
                    ui.selectable_value(
                        &mut profile.plan.selected,
                        String::new(),
                        "Add another account…",
                    );
                });
            if before != profile.plan.selected {
                profile.model.clear();
                self.sync_plan(ui.ctx(), profile);
            }
            let active = profile.plan.active().cloned();
            if let Some(a) = &active {
                ui.label(if a.signed_in() {
                    "ChatGPT connected"
                } else {
                    "ChatGPT account is signed out"
                });
                let notice = a.notice();
                if !notice.is_empty() {
                    ui.label(notice);
                }
            }
            if credentials::supported() {
                if let Some(a) = active.as_ref().filter(|a| a.signed_in()) {
                    let mut remember = a.remember;
                    if ui
                        .checkbox(&mut remember, "Remember ChatGPT session securely")
                        .changed()
                    {
                        let account = a.clone();
                        let id = a.client_id.clone();
                        if !remember && let Some(a) = profile.plan.active_mut() {
                            a.remember = false;
                        }
                        self.start(ui.ctx(), profile.clone(), move || {
                            ResultKind::PlanRemember(
                                id,
                                remember,
                                plan::remember(&account, remember),
                            )
                        });
                    }
                } else {
                    ui.checkbox(&mut self.remember_plan, "Remember ChatGPT session securely");
                }
            } else {
                self.remember_plan = false;
                ui.small("ChatGPT credentials will be kept in memory for this session only.");
            }
            if ui
                .add_enabled(
                    self.job.is_none(),
                    egui::Button::new("Continue with ChatGPT"),
                )
                .clicked()
            {
                self.begin_plan(ui.ctx(), profile, active.clone());
            }
            if let Some(a) = active.as_ref().filter(|a| a.signed_in() || a.remember)
                && ui
                    .add_enabled(self.job.is_none(), egui::Button::new("Sign out of ChatGPT"))
                    .clicked()
            {
                let account = a.clone();
                let id = a.client_id.clone();
                if let Some(a) = profile.plan.active_mut() {
                    a.remember = false;
                    a.mark_loading();
                }
                self.models.clear();
                self.model_names.clear();
                self.catalog_source = None;
                self.start(ui.ctx(), profile.clone(), move || {
                    ResultKind::PlanSignOut(id, plan::sign_out(&account))
                });
            }
            if ui.link("Manage ChatGPT app usage and access").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(
                    "https://chatgpt.com/#settings/Usage",
                ));
            }
            ui.small("ChatGPT plan sign-in is text-only for now. For WAV/MP3 references, use a compatible API-key provider.");
        }
        fn finish_plan(&mut self, ctx: &egui::Context, config: &mut Config) {
            if let Some(login) = self.plan_ready.take() {
                #[cfg(target_os = "ios")]
                let opened = crate::ios::present_auth_browser(&login.url);
                #[cfg(not(target_os = "ios"))]
                let opened = webbrowser::open(&login.url).map_err(|_| {
                    "Could not open the system browser for ChatGPT sign-in.".to_owned()
                });
                if let Err(error) = opened {
                    self.status = error;
                    self.plan_cancel = None;
                } else {
                    let source = config
                        .profiles
                        .iter()
                        .find(|p| p.provider == Provider::ChatGpt)
                        .cloned()
                        .unwrap_or_else(|| Profile::new(Provider::ChatGpt));
                    self.status =
                        "Complete Continue with ChatGPT in the browser. Waiting for authorization…"
                            .into();
                    self.start(ctx, source, move || {
                        ResultKind::PlanLogin(plan::finish(login))
                    });
                }
            }
            if let Some(profile) = config
                .profiles
                .iter_mut()
                .find(|p| p.provider == Provider::ChatGpt)
            {
                for account in &mut profile.plan.accounts {
                    if account.persistence_failed() {
                        account.remember = false;
                    }
                }
            }
            let Some(result) = self.plan_result.take() else {
                return;
            };
            let Some(profile) = config
                .profiles
                .iter_mut()
                .find(|p| p.provider == Provider::ChatGpt)
            else {
                return;
            };
            match result {
                ResultKind::PlanLogin(result) => {
                    self.plan_cancel = None;
                    #[cfg(target_os = "ios")]
                    crate::ios::dismiss_auth_browser();
                    match result {
                        Ok(account) => {
                            let notice = account.notice();
                            profile.plan.accept(account);
                            profile.model.clear();
                            self.models.clear();
                            self.model_names.clear();
                            self.catalog_source = None;
                            self.status = if notice.is_empty() {
                                "ChatGPT connected. Refresh models and choose one to begin.".into()
                            } else {
                                notice
                            };
                        }
                        Err(error) => self.status = error,
                    }
                }
                ResultKind::PlanRestore(id, result) => {
                    if profile.plan.accounts.iter().any(|a| a.client_id == id) {
                        self.status=match result {Ok(())=>"Remembered ChatGPT session loaded. Refresh models to see available choices.".into(),Err(e)=>e};
                    }
                }
                ResultKind::PlanRemember(id, enabled, result) => {
                    if let Some(a) = profile.plan.accounts.iter_mut().find(|a| a.client_id == id) {
                        match result {
                            Ok(()) => {
                                a.remember = enabled;
                                self.status = if enabled {
                                    "ChatGPT session saved securely."
                                } else {
                                    "Saved ChatGPT session removed; this session remains in memory."
                                }
                                .into();
                            }
                            Err(e) => self.status = e,
                        }
                    }
                }
                ResultKind::PlanSignOut(id, result) => {
                    if let Some(a) = profile.plan.accounts.iter_mut().find(|a| a.client_id == id) {
                        a.remember = false;
                        a.clear_local();
                    }
                    self.status=result.unwrap_or_else(|e|format!("Signed out locally, but remote revocation was not confirmed. Disconnect the app in ChatGPT Settings. {e}"));
                }
                _ => {}
            }
        }

        fn model_selector(&mut self, ui: &mut Ui, profile: &mut Profile, width: f32) {
            if profile.provider == Provider::Azure {
                ui.label("Deployment name");
                ui.add(egui::TextEdit::singleline(&mut profile.model).desired_width(f32::INFINITY));
                ui.small("Enter the deployment name from your Azure resource.");
                return;
            }
            ui.label("Model ID");
            egui::ComboBox::from_id_salt("ai_model_list")
                .popup_style((**ui.style()).clone().into())
                .width((width - 20.0).max(60.0))
                .truncate()
                .selected_text(if profile.model.is_empty() {
                    "Choose a model"
                } else {
                    self.model_names
                        .get(&profile.model)
                        .map(String::as_str)
                        .unwrap_or(&profile.model)
                })
                .show_ui(ui, |ui| {
                    if self.models.is_empty() {
                        ui.add_enabled(
                            false,
                            egui::Label::new("Refresh models to load the available choices"),
                        );
                    }
                    for model in &self.models {
                        let name = self.model_names.get(model).unwrap_or(model);
                        if ui
                            .selectable_value(&mut profile.model, model.clone(), name)
                            .clicked()
                        {
                            self.manual_model = false;
                        }
                    }
                    ui.separator();
                    if ui
                        .selectable_label(self.manual_model, "Enter model ID manually…")
                        .clicked()
                    {
                        self.manual_model = true;
                    }
                });
            if self.models.is_empty() {
                ui.small(if profile.provider == Provider::ChatGpt {
                    "Continue with ChatGPT, then refresh models to load your account's available choices."
                } else if profile.api_key.trim().is_empty() {
                    "Enter or load your API key, then press Refresh models to populate this dropdown."
                } else {
                    "Press Refresh models to fetch the current choices from your provider."
                });
            }
            if self.manual_model {
                ui.add(
                    egui::TextEdit::singleline(&mut profile.model)
                        .hint_text("Custom model ID")
                        .desired_width(f32::INFINITY),
                );
            }
        }

        fn transcript(&mut self, ui: &mut Ui) {
            if self.rows.is_empty() {
                ui.label("Ask for a beat, melody, or an explanation. AI can propose editable clips; listen, tweak and learn.");
            }
            let mut selected = None;
            for (index, row) in self.rows.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.strong(if row.message.assistant {
                        "Assistant"
                    } else {
                        "You"
                    });
                    ui.add(egui::Label::new(&row.message.text).wrap().selectable(true));
                    if let Some(proposal) = &row.proposal {
                        let tracks = proposal.get("tracks").and_then(Value::as_array);
                        let count = tracks.map_or(0, Vec::len);
                        let notes: usize = tracks
                            .map(|tracks| {
                                tracks
                                    .iter()
                                    .map(|t| {
                                        t.get("notes").and_then(Value::as_array).map_or(0, Vec::len)
                                    })
                                    .sum()
                            })
                            .unwrap_or(0);
                        ui.label(format!("Proposal: {count} tracks, {notes} notes."));
                        if let Some(tempo) = proposal.get("tempo_bpm").and_then(Value::as_f64) {
                            ui.label(format!("Tempo: {tempo} BPM"));
                        }
                        if let Some(tracks) = tracks {
                            for track in tracks {
                                let target = &track["target"];
                                let name = target
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .or_else(|| target.get("instrument").and_then(Value::as_str))
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| format!("Channel {}", target["channel_id"]));
                                let operation = if target.get("replace_clip_id").is_some() {
                                    "Replace clip on"
                                } else {
                                    "Add clip to"
                                };
                                let start = track["start_bar"].as_u64().unwrap_or(0) + 1;
                                ui.label(format!(
                                    "{operation} {name} · bar {start} · {} bars",
                                    track["bars"]
                                ));
                            }
                        }
                        ui.small(
                            "Apply changes the current song. Undo reverses the whole proposal.",
                        );
                        let button = egui::Button::new(if row.applied {
                            "Applied"
                        } else {
                            "Apply to song"
                        });
                        if ui
                            .add_enabled(self.job.is_none() && !row.applied, button)
                            .clicked()
                        {
                            selected = Some((index, proposal.clone()));
                        }
                    }
                    ui.separator();
                });
            }
            if let Some(selected) = selected {
                self.apply = Some(selected);
            }
        }

        fn send(&mut self, ctx: &egui::Context, profile: &Profile) {
            if self.prompt.len() > 16000 {
                self.status = "Keep each message under 16,000 bytes.".into();
                return;
            }
            if self.audio.is_some() && (!profile.audio_input || !profile.provider.supports_audio())
            {
                self.status = "Remove the reference or choose an audio-capable model first.".into();
                return;
            }
            let text = std::mem::take(&mut self.prompt);
            self.rows.push(Row {
                message: Message {
                    assistant: false,
                    text,
                },
                proposal: None,
                applied: false,
            });
            let messages = self
                .rows
                .iter()
                .rev()
                .take(12)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .skip_while(|row| row.message.assistant)
                .map(|row| Message {
                    assistant: row.message.assistant,
                    text: if row.message.assistant {
                        json!({"reply":row.message.text,"arrangement":row.proposal}).to_string()
                    } else {
                        row.message.text.clone()
                    },
                })
                .collect::<Vec<_>>();
            let context = self.context.clone();
            let audio = self.audio.clone();
            let p = profile.clone();
            self.status = "Asking the provider…".into();
            self.scroll_bottom = true;
            self.start(ctx, profile.clone(), move || {
                ResultKind::Chat(client::chat(&p, &messages, &context, audio.as_ref()))
            });
        }

        fn composer(&mut self, ui: &mut Ui, profile: &Profile) {
            ui.add_enabled_ui(self.job.is_none(), |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.prompt)
                        .hint_text("Describe what you want to make…")
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
                if let Some(audio) = &self.audio {
                    ui.label(format!(
                        "Reference: {} ({:.1} MB)",
                        audio.name,
                        audio.bytes.len() as f64 / 1048576.0
                    ));
                    if ui.button("Remove reference").clicked() {
                        self.audio = None;
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    let attach = egui::Button::new("Attach WAV / MP3…");
                    if ui
                        .add_enabled(
                            profile.audio_input && profile.provider.supports_audio(),
                            attach,
                        )
                        .clicked()
                    {
                        self.attach(ui.ctx(), profile.clone());
                    }
                    let can_send = self.job.is_none()
                        && !self.prompt.trim().is_empty()
                        && profile.connected()
                        && !profile.model.trim().is_empty();
                    if ui
                        .add_enabled(can_send, egui::Button::new("Send"))
                        .clicked()
                    {
                        self.send(ui.ctx(), profile);
                    }
                    if ui.button("Clear chat").clicked() {
                        self.rows.clear();
                        self.audio = None;
                        self.prompt.clear();
                        self.status.clear();
                    }
                });
            });
        }

        pub fn show(&mut self, ui: &mut Ui, cx: &UiCtx, rect: Rect, config: &mut Config) {
            self.poll();
            self.finish_plan(ui.ctx(), config);
            if let Some((source, operation, result)) = self.credential_result.take() {
                if let Some(profile) = config
                    .profiles
                    .iter_mut()
                    .find(|p| p.provider == source.provider)
                {
                    credentials::complete(profile, &source, operation, result);
                }
                self.models.clear();
                self.model_names.clear();
                self.catalog_source = None;
            }
            if self.job.is_some() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("ai_tab")
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            page_style(&mut child, cx);
            egui::ScrollArea::vertical().id_salt("ai_page").show(&mut child, |ui| {
                let width = ui.available_width().max(1.0);
                ui.set_width(width);
                ui.heading("Dewey music assistant");
                if let Some(cancel) = &self.plan_cancel
                    && ui.button("Cancel ChatGPT sign-in").clicked() {
                        cancel.store(true, Ordering::Release);
                        #[cfg(target_os = "ios")] crate::ios::dismiss_auth_browser();
                    }
                self.setup(ui, config, width);
                let profile = config.profile().clone();
                ui.label(format!("{} · {}", profile.provider.label(), if profile.model.is_empty() { "Choose a model" } else { &profile.model }));
                ui.small(format!("Send shares your message, recent chat and song structure with {}. Attached audio is uploaded only when you press Send.", profile.base_url));
                if profile.provider == Provider::ChatGpt {
                    ui.label(profile.plan.active().map(|a|format!("ChatGPT account: {}",a.label())).unwrap_or_else(||"Continue with ChatGPT in Provider setup to connect your plan.".into()));
                }
                ui.separator();
                self.transcript(ui);
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
                if self.job.is_some() {
                    ui.horizontal(|ui| { ui.spinner(); ui.label("Working…"); });
                }
                self.composer(ui, &profile);
                ui.small("Audio: WAV or MP3, up to 10 MB. Use a short loop; references are sent for that turn only. Model catalogs may include models that do not support chat or audio.");
                if self.scroll_bottom {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                    self.scroll_bottom = false;
                }
            });
        }
    }
    impl Drop for AiTab {
        fn drop(&mut self) {
            if let Some(cancel) = &self.plan_cancel {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn proposal_is_applied_only_after_the_user_requests_it_and_success_is_reported() {
            let proposal = json!({"tracks":[]});
            let mut tab = AiTab::default();
            tab.rows.push(Row {
                message: Message {
                    assistant: true,
                    text: "Try a beat".into(),
                },
                proposal: Some(proposal.clone()),
                applied: false,
            });
            assert!(tab.take_apply().is_none());
            tab.apply = Some((0, proposal.clone()));
            assert_eq!(tab.take_apply(), Some(proposal));
            assert!(!tab.rows[0].applied);
            tab.apply_result = Some(Err("invalid notes".into()));
            tab.poll();
            assert!(!tab.rows[0].applied);
            tab.apply = Some((0, json!({"tracks":[]})));
            tab.take_apply();
            tab.apply_result = Some(Ok("Applied".into()));
            tab.poll();
            assert!(tab.rows[0].applied);
        }

        #[test]
        fn failed_cloud_request_restores_the_prompt_and_keeps_the_reference_for_retry() {
            let mut tab = AiTab::default();
            tab.rows.push(Row {
                message: Message {
                    assistant: false,
                    text: "Recreate this beat".into(),
                },
                proposal: None,
                applied: false,
            });
            tab.audio = Some(Audio {
                name: "beat.wav".into(),
                format: "wav",
                bytes: vec![1, 2, 3],
            });
            let (send, receive) = mpsc::channel();
            tab.job = Some(Job {
                receive,
                source: Profile::default(),
            });
            send.send(ResultKind::Chat(Err("quota exceeded".into())))
                .unwrap();
            tab.poll();
            assert_eq!(tab.prompt, "Recreate this beat");
            assert!(tab.rows.is_empty());
            assert!(tab.audio.is_some());
            assert!(tab.job.is_none());
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::AiTab;

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct AiTab {
    pub context: serde_json::Value,
    pub apply_result: Option<Result<String, String>>,
}
#[cfg(target_arch = "wasm32")]
impl AiTab {
    pub fn take_apply(&mut self) -> Option<serde_json::Value> {
        None
    }
    pub fn show(&mut self, ui: &mut Ui, cx: &UiCtx, rect: Rect, _: &mut Config) {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        page_style(&mut child, cx);
        child.label("AI chat is available in the native Honeydew app. API keys are not sent from this web build.");
    }
}
