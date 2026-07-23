//! Ordered naming-engine selection with platform-aware fallbacks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    #[cfg(target_os = "macos")]
    Foundation,
    /// Headless Pi call using Pi's configured model and authentication.
    Pi,
    /// Headless `codex exec` call.
    Codex,
}

/// Resolve `HERDR_NAMING_ENGINE` to the engines to try in order.
pub fn engine_chain(selection: Option<&str>) -> Vec<Engine> {
    match selection.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("pi") => vec![Engine::Pi],
        Some("codex") => vec![Engine::Codex],
        _ => default_chain(),
    }
}

#[cfg(target_os = "macos")]
fn default_chain() -> Vec<Engine> {
    vec![Engine::Foundation, Engine::Pi, Engine::Codex]
}

#[cfg(not(target_os = "macos"))]
fn default_chain() -> Vec<Engine> {
    vec![Engine::Pi, Engine::Codex]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_engine_selection_is_honored() {
        assert_eq!(engine_chain(Some("pi")), vec![Engine::Pi]);
        assert_eq!(engine_chain(Some("codex")), vec![Engine::Codex]);
        assert_eq!(engine_chain(Some("  PI ")), vec![Engine::Pi]);
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;

        #[test]
        fn default_chain_is_foundation_then_pi_then_codex() {
            assert_eq!(
                engine_chain(None),
                vec![Engine::Foundation, Engine::Pi, Engine::Codex]
            );
        }

        #[test]
        fn non_explicit_selection_uses_default_chain() {
            for selection in [Some("foundation"), Some("bogus"), Some("")] {
                assert_eq!(
                    engine_chain(selection),
                    vec![Engine::Foundation, Engine::Pi, Engine::Codex]
                );
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    mod non_macos {
        use super::*;

        #[test]
        fn default_chain_is_pi_then_codex() {
            assert_eq!(engine_chain(None), vec![Engine::Pi, Engine::Codex]);
        }

        #[test]
        fn non_explicit_selection_uses_default_chain() {
            for selection in [Some("foundation"), Some("bogus"), Some("")] {
                assert_eq!(engine_chain(selection), vec![Engine::Pi, Engine::Codex]);
            }
        }
    }
}
