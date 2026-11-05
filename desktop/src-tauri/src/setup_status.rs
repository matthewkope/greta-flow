//! Backend-only setup state. No window content or menu labels live here.
use crate::locks::MutexExt;
use crate::{config, engine, model, AppState};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, serde::Serialize)]
pub(crate) struct ModelStatus {
    pub generation: u64,
    pub revision: u64,
    pub model: String,
    #[serde(flatten)]
    pub progress: model::Progress,
    pub failure_reason: Option<String>,
}

pub(crate) fn publish(
    app: &AppHandle,
    generation: u64,
    name: &str,
    progress: model::Progress,
    failure_reason: Option<String>,
) {
    let state = app.state::<AppState>();
    let mut slot = state.model_status.lock_recover();
    if state.model_generation.load(Ordering::SeqCst) != generation {
        return;
    }
    let status = ModelStatus {
        revision: slot.as_ref().map_or(1, |old| old.revision + 1),
        generation,
        model: name.into(),
        progress,
        failure_reason,
    };
    *slot = Some(status.clone());
    let _ = app.emit("setup-model-status", status);
}

pub(crate) fn phase(app: &AppHandle, generation: u64, name: &str, phase: &str) {
    publish(app, generation, name, model::Progress::phase(phase), None);
}

pub(crate) fn load(
    app: &AppHandle,
    cfg: &config::Config,
    generation: u64,
) -> Result<engine::Engine, String> {
    let path = model::ensure_model_with_progress(&cfg.model, &mut |progress| {
        publish(app, generation, &cfg.model, progress, None);
    })?;
    phase(app, generation, &cfg.model, "preparing");
    engine::Engine::load(&path.to_string_lossy(), cfg)
}

#[tauri::command]
pub(crate) fn setup_model_status(app: AppHandle) -> Option<ModelStatus> {
    app.state::<AppState>().model_status.lock_recover().clone()
}

/// Conservative fresh Parakeet budget: download plus working/compilation headroom.
/// Actual per-file downloads also check the cache volume before writing.
#[tauri::command]
pub(crate) async fn setup_disk_space() -> Result<model::DiskSpace, String> {
    model::disk_space(
        &model::model_path(config::DEFAULT_MODEL),
        1024 * 1024 * 1024,
    )
}

#[derive(serde::Serialize)]
pub(crate) struct Permissions {
    accessibility: bool,
    input_monitoring: bool,
}

#[tauri::command]
pub(crate) fn setup_permissions() -> Permissions {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightListenEventAccess() -> bool;
    }
    // Read current process authorization every call, without prompting or caching.
    unsafe {
        Permissions {
            accessibility: AXIsProcessTrusted(),
            input_monitoring: CGPreflightListenEventAccess(),
        }
    }
}

fn step_path() -> std::path::PathBuf {
    config::config_path().with_file_name("setup-step.json")
}
fn valid_step(step: &str) -> bool {
    matches!(
        step,
        "welcome"
            | "model"
            | "microphone"
            | "accessibility"
            | "input"
            | "level"
            | "practice"
            | "finish"
    )
}

#[tauri::command]
pub(crate) fn setup_saved_step() -> Result<Option<String>, String> {
    read_step(&step_path())
}

fn read_step(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let step: String = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if !valid_step(&step) {
                return Err("invalid saved setup step".into());
            }
            Ok(Some(step))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub(crate) fn setup_save_step(step: Option<String>) -> Result<(), String> {
    // Serialize writes so two window invokes cannot race on the temporary file.
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE.lock_recover();
    write_step(&step_path(), step)
}

fn write_step(path: &std::path::Path, step: Option<String>) -> Result<(), String> {
    if let Some(step) = step {
        if !valid_step(&step) {
            return Err("invalid setup step".into());
        }
        std::fs::create_dir_all(path.parent().ok_or("missing setup directory")?)
            .map_err(|e| e.to_string())?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(&step).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[tauri::command]
pub(crate) fn setup_relaunch(app: AppHandle, step: String) -> Result<(), String> {
    setup_save_step(Some(step))?;
    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_step_survives_reopen_and_completion_clears_it() {
        let root = std::env::temp_dir().join(format!(
            "gf-setup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("setup-step.json");
        assert_eq!(read_step(&path).unwrap(), None);
        write_step(&path, Some("input".into())).unwrap();
        assert_eq!(read_step(&path).unwrap().as_deref(), Some("input"));
        assert!(write_step(&path, Some("invalid".into())).is_err());
        assert_eq!(read_step(&path).unwrap().as_deref(), Some("input"));
        write_step(&path, None).unwrap();
        write_step(&path, None).unwrap();
        assert_eq!(read_step(&path).unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_setup_steps_can_be_persisted() {
        assert!(valid_step("input"));
        assert!(valid_step("finish"));
        assert!(!valid_step("../config"));
        assert!(!valid_step(""));
    }
}
