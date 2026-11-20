use crate::app_settings::{HOTKEY_MODIFIERS_MASK, SHIFT_KEY_MASK};
use crate::dictation::context::frontmost_pid;
use crate::suppression::{InputActivity, held_modifiers};

/// The app and input state when this dictation's shortcut finished.
#[derive(Clone, Copy)]
pub(crate) struct ShiftSubmit {
    pid: i32,
    input_revision: u64,
    shift_at_finish: bool,
}

impl ShiftSubmit {
    pub(crate) fn capture(activity: &InputActivity, modifiers: Option<u64>) -> Option<Self> {
        if !crate::labs::shift_to_submit() {
            return None;
        }
        let input_revision = activity.revision();
        let pid = frontmost_pid().filter(|pid| *pid != std::process::id() as i32)?;
        Some(Self {
            pid,
            input_revision,
            shift_at_finish: modifiers.unwrap_or_else(held_modifiers) & HOTKEY_MODIFIERS_MASK
                == SHIFT_KEY_MASK,
        })
    }

    pub(super) fn pid(self) -> i32 {
        self.pid
    }

    pub(super) fn requested(self) -> bool {
        let modifiers = held_modifiers();
        matches!(modifiers, 0 | SHIFT_KEY_MASK)
            && (self.shift_at_finish || modifiers == SHIFT_KEY_MASK)
    }

    pub(super) fn target_is_current(self, activity: &InputActivity) -> bool {
        !crate::desktop::process_exit::quitting()
            && crate::labs::shift_to_submit()
            && self.same_target(frontmost_pid(), activity.revision())
    }

    fn same_target(self, pid: Option<i32>, revision: u64) -> bool {
        pid == Some(self.pid) && revision == self.input_revision
    }

    pub(super) fn no_other_modifier() -> bool {
        held_modifiers() & !SHIFT_KEY_MASK == 0
    }

    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        Self {
            pid: 42,
            input_revision: 0,
            shift_at_finish: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_apps_or_interacting_with_the_target_clears_the_request() {
        let submit = ShiftSubmit::fixture();
        assert!(submit.same_target(Some(42), 0));
        assert!(!submit.same_target(Some(43), 0));
        assert!(!submit.same_target(Some(42), 1));
        assert!(!submit.same_target(None, 0));
    }
}
