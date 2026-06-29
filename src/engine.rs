//! Naming-engine selection: maps the `HERDR_NAMING_ENGINE` knob to an ordered
//! fallback chain. The cold phase tries each engine in turn and uses the first
//! that returns a slug, falling back to a deterministic local slug if all fail.
//!
//! The on-device `Foundation` engine is macOS-only and is compiled out entirely
//! on other targets (e.g. Linux): the enum variant does not exist there, so a
//! non-macOS build can neither select nor reference Apple's FoundationModels.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// On-device Apple FoundationModels via the `herdr-namer` Swift helper.
    #[cfg(target_os = "macos")]
    Foundation,
    /// Headless `codex exec` call.
    Codex,
}

/// Resolve the engine knob to the ordered list of engines to try.
///
/// - `codex`: Codex only (skip the on-device helper entirely).
/// - anything else (`foundation`, unset, empty, unknown): the platform default
///   chain. On macOS that is on-device first with Codex as the automatic
///   fallback; on every other target there is no on-device engine, so the
///   default collapses to Codex only.
pub fn engine_chain(selection: Option<&str>) -> Vec<Engine> {
    match selection.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("codex") => vec![Engine::Codex],
        _ => default_chain(),
    }
}

#[cfg(target_os = "macos")]
fn default_chain() -> Vec<Engine> {
    vec![Engine::Foundation, Engine::Codex]
}

#[cfg(not(target_os = "macos"))]
fn default_chain() -> Vec<Engine> {
    vec![Engine::Codex]
}

#[cfg(test)]
mod tests {
    use super::*;

    // `codex` is always honored and always Codex-only, on every platform.
    #[test]
    fn codex_selection_skips_foundation() {
        assert_eq!(engine_chain(Some("codex")), vec![Engine::Codex]);
    }

    #[test]
    fn selection_is_case_and_whitespace_insensitive() {
        assert_eq!(engine_chain(Some("  CODEX ")), vec![Engine::Codex]);
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;

        #[test]
        fn default_chain_is_foundation_then_codex() {
            assert_eq!(engine_chain(None), vec![Engine::Foundation, Engine::Codex]);
        }

        #[test]
        fn foundation_selection_keeps_codex_fallback() {
            assert_eq!(
                engine_chain(Some(" Foundation ")),
                vec![Engine::Foundation, Engine::Codex]
            );
        }

        #[test]
        fn unknown_or_empty_falls_back_to_default_chain() {
            assert_eq!(
                engine_chain(Some("bogus")),
                vec![Engine::Foundation, Engine::Codex]
            );
            assert_eq!(
                engine_chain(Some("")),
                vec![Engine::Foundation, Engine::Codex]
            );
        }
    }

    // Off macOS there is no on-device engine: every selection that is not an
    // explicit `codex` still resolves to Codex only, and `foundation` is
    // silently downgraded rather than attempted.
    #[cfg(not(target_os = "macos"))]
    mod non_macos {
        use super::*;

        #[test]
        fn default_chain_is_codex_only() {
            assert_eq!(engine_chain(None), vec![Engine::Codex]);
        }

        #[test]
        fn foundation_request_is_downgraded_to_codex() {
            assert_eq!(engine_chain(Some("foundation")), vec![Engine::Codex]);
        }

        #[test]
        fn unknown_or_empty_is_codex_only() {
            assert_eq!(engine_chain(Some("bogus")), vec![Engine::Codex]);
            assert_eq!(engine_chain(Some("")), vec![Engine::Codex]);
        }
    }
}
