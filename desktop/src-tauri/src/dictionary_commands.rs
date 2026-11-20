//! Tauri commands for the personal dictionary and spoken shortcuts panel.
use crate::dictionary::{self, DictionaryEntry, SpokenShortcut};
use crate::locks::MutexExt;
use crate::{config, AppState};
use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
pub(crate) struct PersonalState {
    dictionary: Vec<DictionaryEntry>,
    spoken_shortcuts: Vec<SpokenShortcut>,
}

#[tauri::command]
pub(crate) fn dictionary_state(app: AppHandle) -> PersonalState {
    let cfg = app.state::<AppState>().cfg.lock_recover().clone();
    PersonalState {
        dictionary: cfg.dictionary,
        spoken_shortcuts: cfg.spoken_shortcuts,
    }
}

/// Validate and persist both lists. Applies to the next dictation; existing
/// history records are never rewritten.
#[tauri::command]
pub(crate) fn dictionary_save(
    app: AppHandle,
    dictionary: Vec<DictionaryEntry>,
    spoken_shortcuts: Vec<SpokenShortcut>,
) -> Result<PersonalState, String> {
    let personal = dictionary::validate(dictionary, spoken_shortcuts)?;
    let state = app.state::<AppState>();
    let mut live = state.cfg.lock_recover();
    // Start from the file so settings edited elsewhere are not overwritten,
    // and never replace a file that exists but can't be parsed.
    let mut cfg = match std::fs::read_to_string(config::config_path()) {
        Ok(text) => serde_json::from_str::<config::Config>(&text).map_err(|e| {
            format!("Settings file couldn't be read ({e}); fix it and reload settings first.")
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => live.clone(),
        Err(e) => return Err(format!("Couldn't read settings: {e}")),
    };
    cfg.dictionary = personal.dictionary.clone();
    cfg.spoken_shortcuts = personal.shortcuts.clone();
    cfg.try_save()
        .map_err(|e| format!("Couldn't save settings: {e}"))?;
    live.dictionary = personal.dictionary.clone();
    live.spoken_shortcuts = personal.shortcuts.clone();
    log::info!(
        "personal dictionary saved: {} entries, {} shortcuts",
        personal.dictionary.len(),
        personal.shortcuts.len()
    );
    Ok(PersonalState {
        dictionary: personal.dictionary,
        spoken_shortcuts: personal.shortcuts,
    })
}
