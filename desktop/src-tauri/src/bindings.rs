//! Validate shortcut plans before changing live bindings. Native registration
//! failures leave the previous dispatch table intact.
use crate::locks::MutexExt;
use crate::{config::Config, hotkey::bare_modifier_flag};
use std::sync::{Arc, Mutex};
use tauri_plugin_global_shortcut::Shortcut;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Dictate,
    Cancel,
    Retry,
    Recover,
}

#[derive(Default)]
pub struct Plan {
    pub shortcuts: Vec<(Shortcut, Action)>,
    pub watched: u64,
}
impl Plan {
    fn add(&mut self, key: &str, action: Action) -> Result<(), String> {
        if let Some(flag) = bare_modifier_flag(key) {
            if action != Action::Dictate {
                return Err("Recovery controls require an accelerator, not a bare modifier".into());
            }
            if self.watched & flag != 0 {
                return Err("Shortcut bindings must be distinct".into());
            }
            self.watched |= flag;
            return Ok(());
        }
        let shortcut = key.parse::<Shortcut>().map_err(|e| e.to_string())?;
        if self.shortcuts.iter().any(|(bound, _)| *bound == shortcut) {
            return Err("Shortcut bindings must be distinct".into());
        }
        self.shortcuts.push((shortcut, action));
        Ok(())
    }
    fn build(cfg: &Config, skip_invalid: bool) -> Result<Self, String> {
        let mut plan = Self::default();
        let entries = cfg.shortcuts.iter().map(|s| (s, Action::Dictate)).chain(
            [
                (&cfg.cancel_shortcut, Action::Cancel),
                (&cfg.retry_shortcut, Action::Retry),
                (&cfg.recovery_shortcut, Action::Recover),
            ]
            .into_iter()
            .filter_map(|(key, action)| key.as_ref().map(|key| (key, action))),
        );
        for (key, action) in entries {
            if let Err(e) = plan.add(key, action) {
                if !skip_invalid {
                    return Err(e);
                }
                log::warn!("ignoring invalid {action:?} shortcut: {e}");
            }
        }
        Ok(plan)
    }
    pub fn validate(cfg: &Config) -> Result<Self, String> {
        Self::build(cfg, false)
    }
    pub fn startup(cfg: &Config) -> Self {
        // Lenient construction cannot return an error.
        Self::build(cfg, true).unwrap_or_default()
    }
}

/// Separate from registration state: the plugin holds its own registry lock
/// while invoking callbacks, so callbacks must never take the updater's lock.
#[derive(Clone, Default)]
pub struct Dispatch(Arc<Mutex<Vec<(Shortcut, Action)>>>);
impl Dispatch {
    pub fn action(&self, shortcut: &Shortcut) -> Option<Action> {
        self.0
            .lock_recover()
            .iter()
            .find(|(s, _)| s == shortcut)
            .map(|(_, a)| *a)
    }
    fn replace(&self, active: &[(Shortcut, Action)]) {
        *self.0.lock_recover() = active.to_vec();
    }
}

#[derive(Default)]
pub struct Bindings {
    pub dispatch: Dispatch,
    active: Vec<(Shortcut, Action)>,
    // Failed unregisters remain tracked but have no dispatch action, so they
    // cannot accidentally trigger an old command or be registered twice.
    registered: Vec<Shortcut>,
}
impl Bindings {
    pub fn startup(
        &mut self,
        plan: &Plan,
        mut register: impl FnMut(Shortcut) -> Result<(), String>,
    ) {
        for &(shortcut, action) in &plan.shortcuts {
            match register(shortcut) {
                Ok(()) => {
                    self.registered.push(shortcut);
                    self.active.push((shortcut, action));
                    self.dispatch.replace(&self.active);
                }
                Err(e) => log::warn!("ignoring unavailable {action:?} shortcut: {e}"),
            }
        }
    }
    pub fn apply(
        &mut self,
        plan: &Plan,
        mut register: impl FnMut(Shortcut) -> Result<(), String>,
        mut unregister: impl FnMut(Shortcut) -> Result<(), String>,
    ) -> Result<(), String> {
        // Acquire new native registrations before removing any working binding.
        for &(shortcut, _) in &plan.shortcuts {
            if !self.registered.contains(&shortcut) {
                if let Err(e) = register(shortcut) {
                    self.cleanup(&mut unregister);
                    return Err(e);
                }
                self.registered.push(shortcut);
            }
        }
        // All callbacks dispatch through this table; changing a key's action
        // does not require an unregister/register gap.
        self.active = plan.shortcuts.clone();
        self.dispatch.replace(&self.active);
        self.cleanup(&mut unregister);
        Ok(())
    }
    fn cleanup(&mut self, unregister: &mut impl FnMut(Shortcut) -> Result<(), String>) {
        self.registered.retain(|shortcut| {
            if self.active.iter().any(|(key, _)| key == shortcut) {
                return true;
            }
            if let Err(e) = unregister(*shortcut) {
                log::warn!("could not unregister inactive shortcut: {e}");
                return true;
            }
            false
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sc(key: &str) -> Shortcut {
        key.parse().unwrap()
    }

    #[test]
    fn invalid_and_colliding_recovery_keys_are_skipped_only_at_startup() {
        for bad in [
            "Fn",
            "Ctrl",
            "not a shortcut",
            "Ctrl+Alt+R",
            "Control+Alt+R",
        ] {
            let cfg = Config {
                shortcuts: vec!["Fn".into(), "Ctrl+Alt+R".into()],
                cancel_shortcut: Some(bad.into()),
                recovery_shortcut: Some("Ctrl+Alt+C".into()),
                ..Default::default()
            };
            assert!(Plan::validate(&cfg).is_err(), "{bad}");
            let startup = Plan::startup(&cfg);
            assert_eq!(startup.watched, bare_modifier_flag("Fn").unwrap());
            assert_eq!(
                startup.shortcuts,
                vec![
                    (sc("Ctrl+Alt+R"), Action::Dictate),
                    (sc("Ctrl+Alt+C"), Action::Recover)
                ]
            );
        }
        let cfg = Config {
            cancel_shortcut: Some("Ctrl+Alt+C".into()),
            retry_shortcut: Some("Ctrl+Alt+C".into()),
            ..Default::default()
        };
        assert!(Plan::validate(&cfg).is_err());
        assert_eq!(Plan::startup(&cfg).shortcuts.len(), 1);
    }

    #[test]
    fn startup_native_failure_does_not_block_other_keys() {
        let plan = Plan::validate(&Config {
            shortcuts: vec!["Ctrl+Alt+D".into()],
            cancel_shortcut: Some("Ctrl+Alt+C".into()),
            recovery_shortcut: Some("Ctrl+Alt+R".into()),
            ..Default::default()
        })
        .unwrap();
        let mut bindings = Bindings::default();
        bindings.startup(&plan, |key| {
            if key == sc("Ctrl+Alt+C") {
                Err("occupied".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(
            bindings.dispatch.action(&sc("Ctrl+Alt+D")),
            Some(Action::Dictate)
        );
        assert_eq!(bindings.dispatch.action(&sc("Ctrl+Alt+C")), None);
        assert_eq!(
            bindings.dispatch.action(&sc("Ctrl+Alt+R")),
            Some(Action::Recover)
        );
    }

    #[test]
    fn failed_reload_keeps_old_dispatch_and_rolls_back_added_keys() {
        let mut bindings = Bindings::default();
        let old = Plan {
            shortcuts: vec![(sc("Ctrl+Alt+D"), Action::Dictate)],
            watched: 0,
        };
        bindings.startup(&old, |_| Ok(()));
        let replacement = Plan {
            shortcuts: vec![
                (sc("Ctrl+Alt+C"), Action::Cancel),
                (sc("Ctrl+Alt+R"), Action::Recover),
            ],
            watched: 0,
        };
        let mut removed = vec![];
        let dispatch = bindings.dispatch.clone();
        assert!(bindings
            .apply(
                &replacement,
                |key| {
                    // Callbacks can read the old dispatch table even while
                    // registration is in progress, without taking its lock.
                    assert_eq!(dispatch.action(&sc("Ctrl+Alt+D")), Some(Action::Dictate));
                    assert_eq!(dispatch.action(&sc("Ctrl+Alt+C")), None);
                    if key == sc("Ctrl+Alt+R") {
                        Err("occupied".into())
                    } else {
                        Ok(())
                    }
                },
                |key| {
                    assert_eq!(dispatch.action(&sc("Ctrl+Alt+D")), Some(Action::Dictate));
                    removed.push(key);
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(bindings.active, old.shortcuts);
        assert_eq!(bindings.registered, vec![sc("Ctrl+Alt+D")]);
        assert_eq!(removed, vec![sc("Ctrl+Alt+C")]);
    }

    #[test]
    fn action_changes_and_failed_cleanup_never_dispatch_stale_actions() {
        let mut bindings = Bindings::default();
        bindings.startup(
            &Plan {
                shortcuts: vec![(sc("Ctrl+Alt+D"), Action::Dictate)],
                watched: 0,
            },
            |_| Ok(()),
        );
        let changed = Plan {
            shortcuts: vec![(sc("Ctrl+Alt+D"), Action::Recover)],
            watched: 0,
        };
        bindings
            .apply(
                &changed,
                |_| panic!("already registered"),
                |_| panic!("still active"),
            )
            .unwrap();
        assert_eq!(
            bindings.dispatch.action(&sc("Ctrl+Alt+D")),
            Some(Action::Recover)
        );
        bindings
            .apply(
                &Plan::default(),
                |_| panic!(),
                |_| Err("native failure".into()),
            )
            .unwrap();
        assert_eq!(bindings.dispatch.action(&sc("Ctrl+Alt+D")), None);
        bindings
            .apply(
                &changed,
                |_| panic!("tracked registration reused"),
                |_| panic!(),
            )
            .unwrap();
        assert_eq!(
            bindings.dispatch.action(&sc("Ctrl+Alt+D")),
            Some(Action::Recover)
        );
    }
}
