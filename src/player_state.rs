// -- player_state.rs -- //
//
// État partagé du lecteur (équivalent de l'objet `Player` de gui.pyw, sans la
// partie graphique) : moteur audio, playlist, index, libellé, volume, mute.
// Tout est protégé pour pouvoir être utilisé depuis le thread d'auto-lecture.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::audio_engine::Engine;

pub struct Inner {
    pub playlist: Vec<PathBuf>,
    pub index: isize,
    pub label: String,
    pub muted: bool,
    pub user_volume: i32,
    /// Nombre de fichiers illisibles consécutifs (évite une boucle infinie d'erreurs).
    pub failures: usize,
}

pub struct PlayerState {
    pub engine: Engine,
    pub inner: Mutex<Inner>,
    notify: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl PlayerState {
    pub fn new() -> PlayerState {
        PlayerState {
            engine: Engine::new(),
            inner: Mutex::new(Inner {
                playlist: Vec::new(),
                index: -1,
                label: "Aucune musique".to_string(),
                muted: false,
                user_volume: 80,
                failures: 0,
            }),
            notify: Mutex::new(None),
        }
    }

    /// Verrou tolérant à l'empoisonnement (un panic dans un thread ne doit pas figer le lecteur).
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// Callback appelé quand l'état change depuis un autre thread (réveil de l'interface).
    pub fn set_notifier(&self, f: Box<dyn Fn() + Send + Sync>) {
        if let Ok(mut n) = self.notify.lock() {
            *n = Some(f);
        }
    }

    pub fn notify(&self) {
        if let Ok(n) = self.notify.lock() {
            if let Some(f) = n.as_ref() {
                f();
            }
        }
    }
}
