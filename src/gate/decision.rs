// [c0,cw] This pure component interface maps gate inputs to the outcome that
// the parent renders; its tests do not depend on IO or regression internals.
pub(super) enum Outcome {
    Pass,
    RegressionFailure,
    MixedCanonChangeFailure,
}

pub(super) fn decide(
    unresolved_pass_to_fail_regressions: usize,
    changed_paths: &[Vec<u8>],
) -> Outcome {
    // [W4,cw] The regression producer has already made same-tree passes
    // authoritative over older failures. Only unresolved baseline-pass to
    // staged-fail transitions can reach this decision as a non-zero count.
    if unresolved_pass_to_fail_regressions > 0 {
        return Outcome::RegressionFailure;
    }
    if has_mixed_canon_and_non_canon_changes(changed_paths) {
        return Outcome::MixedCanonChangeFailure;
    }
    Outcome::Pass
}

fn has_mixed_canon_and_non_canon_changes(changed_paths: &[Vec<u8>]) -> bool {
    let has_canon_change = changed_paths.iter().any(|path| is_canon_project_path(path));
    has_canon_change && !all_paths_are_canon(changed_paths)
}

fn is_canon_project_path(path: &[u8]) -> bool {
    path.starts_with(b".canon/")
}

fn all_paths_are_canon(paths: &[Vec<u8>]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_canon_project_path(path))
}

#[cfg(test)]
mod tests {
    use super::{decide, Outcome};

    #[test] // xpec: cw
    fn passes_canon_only_change_without_regressions() {
        let changed_paths = vec![b".canon/check.yml".to_vec()];

        assert!(matches!(decide(0, &changed_paths), Outcome::Pass));
    }

    #[test] // xpec: cw
    fn prioritizes_regressions_over_mixed_change() {
        let changed_paths = vec![b".canon/check.yml".to_vec(), b"src/lib.rs".to_vec()];

        assert!(matches!(
            decide(1, &changed_paths),
            Outcome::RegressionFailure
        ));
        assert!(matches!(
            decide(0, &changed_paths),
            Outcome::MixedCanonChangeFailure
        ));
    }

    #[test] // xpec: W4,cw
    fn passes_when_no_unresolved_expectation_regressions_remain() {
        let changed_paths = vec![b"src/lib.rs".to_vec()];

        assert!(matches!(decide(0, &changed_paths), Outcome::Pass));
    }
}
