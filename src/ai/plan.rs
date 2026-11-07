//! ChatGPT plan registrations. Tokens are separate from all serialized preferences.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::*;

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub selected: String,
    pub accounts: Vec<Account>,
}
impl Plan {
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.selected.is_empty()
    }
    pub fn active(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| a.client_id == self.selected)
    }
    pub fn active_mut(&mut self) -> Option<&mut Account> {
        self.accounts
            .iter_mut()
            .find(|a| a.client_id == self.selected)
    }
    pub fn accept(&mut self, account: Account) {
        self.selected = account.client_id.clone();
        if let Some(old) = self
            .accounts
            .iter_mut()
            .find(|a| a.client_id == account.client_id && a.subject == account.subject)
        {
            *old = account;
        } else {
            self.accounts.push(account);
        }
    }
}
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub client_id: String,
    pub subject: String,
    pub email: String,
    pub remember: bool,
    #[serde(skip)]
    pub session: Session,
}
impl Account {
    pub fn label(&self) -> String {
        let tail = self
            .client_id
            .chars()
            .rev()
            .take(8)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!(
            "{} · {}",
            if self.email.is_empty() {
                "ChatGPT account"
            } else {
                &self.email
            },
            tail
        )
    }
    pub fn signed_in(&self) -> bool {
        self.session.0.ready.load(Ordering::Acquire)
    }
    pub fn notice(&self) -> String {
        self.session
            .0
            .notice
            .try_lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
    pub fn persistence_failed(&self) -> bool {
        self.session.0.persist_failed.load(Ordering::Acquire)
    }
    pub fn attempted_load(&self) -> bool {
        self.session.0.loaded.load(Ordering::Acquire)
    }
    pub fn mark_loading(&self) {
        self.session.0.loaded.store(true, Ordering::Release);
    }
    pub fn clear_local(&self) {
        if let Ok(mut value) = self.session.0.tokens.lock() {
            *value = None;
        }
        self.session.0.ready.store(false, Ordering::Release);
        self.session.0.loaded.store(true, Ordering::Release);
    }
}
#[derive(Clone, Default)]
pub struct Session(Arc<SessionState>);
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session(<redacted>)")
    }
}
impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (!self.0.ready.load(Ordering::Acquire) && !other.0.ready.load(Ordering::Acquire))
    }
}
#[derive(Default)]
struct SessionState {
    tokens: Mutex<Option<Tokens>>,
    ready: AtomicBool,
    persist_failed: AtomicBool,
    loaded: AtomicBool,
    notice: Mutex<String>,
}
// Deliberately no Debug: errors, transcripts and preference serialization cannot expose these values.
#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    client_id: String,
    subject: String,
    email: String,
    access_token: String,
    refresh_token: String,
    id_token: String,
    scope: String,
    expires_at: u64,
}
