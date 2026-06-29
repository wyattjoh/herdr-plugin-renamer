//! Naming-engine selection: maps the `HERDR_NAMING_ENGINE` knob to an ordered
//! fallback chain. The cold phase tries each engine in turn and uses the first
//! that returns a slug, falling back to a deterministic local slug if all fail.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// On-device Apple FoundationModels via the `herdr-namer` Swift helper.
    Foundation,
    /// Headless `codex exec` call.
    Codex,
}

/// Resolve the engine knob to the ordered list of engines to try.
///
/// - `codex`: Codex only (skip the on-device helper entirely).
/// - anything else (`foundation`, unset, empty, unknown): on-device first with
///   Codex as the automatic fallback when Apple Intelligence is unavailable.
pub fn engine_chain(selection: Option<&str>) -> Vec<Engine> {
    match selection.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("codex") => vec![Engine::Codex],
        _ => vec![Engine::Foundation, Engine::Codex],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_chain_is_foundation_then_codex() {
        assert_eq!(engine_chain(None), vec![Engine::Foundation, Engine::Codex]);
    }

    #[test]
    fn foundation_selection_keeps_codex_fallback() {
        assert_eq!(
            engine_chain(Some("foundation")),
            vec![Engine::Foundation, Engine::Codex]
        );
    }

    #[test]
    fn codex_selection_skips_foundation() {
        assert_eq!(engine_chain(Some("codex")), vec![Engine::Codex]);
    }

    #[test]
    fn selection_is_case_and_whitespace_insensitive() {
        assert_eq!(engine_chain(Some("  CODEX ")), vec![Engine::Codex]);
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
