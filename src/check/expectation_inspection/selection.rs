use crate::check::q_scope::initial_q_scope_for_check_run;
use crate::check::ResolvedExpectation;
use crate::check::{order_selected_by_rank_and_latest_fail, select_expectations_with_identities};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::scope::visible_scope;
use crate::xpec_state::XpecStateCache;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use super::render::render_show_expectations_text;

pub(crate) struct ShowRenderRequest<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a crate::config_types::CheckConfig,
    pub(crate) identities: &'a [crate::check::ExpectationIdentity],
    pub(crate) tree_source: Option<&'a TreeSource>,
    pub(crate) selectors: &'a [OsString],
    pub(crate) pathspecs: &'a [String],
    pub(crate) current_expectation_id: Option<&'a str>,
    pub(crate) xpec_state: &'a mut XpecStateCache,
    pub(crate) visible_tree_oid_cache: &'a mut VisibleTreeOidCache,
}

pub(crate) struct ShowRenderedOutput {
    pub(crate) text: String,
    pub(crate) expectation_ids: BTreeSet<String>,
}

pub(crate) fn render_show_for_current_run(
    request: ShowRenderRequest<'_>,
) -> Result<ShowRenderedOutput, String> {
    let ordered = select_show_expectations_for_current_run(request)?;
    let expectation_ids = ordered
        .iter()
        .map(|expectation| expectation.require_configured_id().map(str::to_string))
        .collect::<Result<_, _>>()?;
    Ok(ShowRenderedOutput {
        text: render_show_expectations_text(&ordered),
        expectation_ids,
    })
}

pub(crate) fn select_show_expectations_for_current_run(
    request: ShowRenderRequest<'_>,
) -> Result<Vec<ResolvedExpectation>, String> {
    let mut selectors = request.selectors.to_vec();
    // xpec: 6,t
    // The dynamic-tool contract defines its answer-leakage prohibition as an
    // appended exclusion. Route that contract input through the ordinary
    // selector component so the tool exposes the same selection and validation
    // behavior as `canon show`, including user-visible selector errors.
    if let Some(current_expectation_id) = request.current_expectation_id {
        selectors.push(OsString::from(format!("not:{}", current_expectation_id)));
    }
    // Shared with `canon check`; this handles include selectors and
    // `not:<ID-PREFIX>` exclusions before pathspec filtering.
    let selected =
        select_expectations_with_identities(request.config, request.identities, &selectors)?;
    let filtered = match request.tree_source {
        Some(tree_source) => filter_expectations_by_pathspecs(
            request.root,
            tree_source,
            selected,
            request.pathspecs,
            request.visible_tree_oid_cache,
            request.xpec_state,
        )?,
        None if request.pathspecs.is_empty() => selected,
        None => {
            return Err("canon.show pathspecs require a Git-backed check run".to_string());
        }
    };
    order_selected_by_rank_and_latest_fail(
        request.root,
        filtered,
        request.xpec_state,
        |expectation| expectation,
    )
}

fn filter_expectations_by_pathspecs(
    root: &Path,
    tree_source: &TreeSource,
    expectations: Vec<ResolvedExpectation>,
    pathspecs: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    xpec_state: &mut XpecStateCache,
) -> Result<Vec<ResolvedExpectation>, String> {
    if pathspecs.is_empty() {
        return Ok(expectations);
    }
    let mut filtered = Vec::new();
    for expectation in expectations {
        if expectation_is_affected_by_pathspecs(
            root,
            tree_source,
            &expectation,
            pathspecs,
            visible_tree_oid_cache,
            xpec_state,
        )? {
            filtered.push(expectation);
        }
    }
    Ok(filtered)
}

fn expectation_is_affected_by_pathspecs(
    root: &Path,
    tree_source: &TreeSource,
    expectation: &ResolvedExpectation,
    pathspecs: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    xpec_state: &mut XpecStateCache,
) -> Result<bool, String> {
    let q_scope = initial_q_scope_for_check_run(root, expectation, xpec_state)?;
    visible_tree_oid_is_affected_by_pathspecs(
        root,
        tree_source,
        expectation,
        &q_scope,
        pathspecs,
        visible_tree_oid_cache,
    )
}

pub(super) fn visible_tree_oid_is_affected_by_pathspecs(
    root: &Path,
    tree_source: &TreeSource,
    expectation: &ResolvedExpectation,
    q_scope: &[String],
    pathspecs: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<bool, String> {
    // A missing visible tree OID is a separate exclusion gate from the
    // changed-file overlap test below.
    if visible_tree_oid_cache
        .visible_tree_oid_for_reuse(root, tree_source, &expectation.agent, q_scope)?
        .is_none()
    {
        return Ok(false);
    }
    // If every tracked file matched by `pathspecs` changed, the visible tree
    // OID would change exactly when at least one such file is selected by the
    // complete visible scope.
    let visible_scope = visible_scope(&expectation.agent, q_scope)?;
    visible_tree_oid_cache.visible_scope_intersects_pathspecs(
        root,
        tree_source,
        &visible_scope,
        pathspecs,
    )
}

#[cfg(test)]
mod co_located_unit_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: r8
    fn pathspec_filter_accepts_union_scope_with_stale_term() {
        let root = git_project("canon-show-filter-visible-oid");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/app.rs"), "fn main() {}\n").unwrap();
        git(&root, &["add", "src/app.rs"]);
        let expectation = ResolvedExpectation {
            kind: crate::check::core::ResolvedExpectationKind::Configured {
                id: "11111111111111111111".to_string(),
            },
            display_id: "1".to_string(),
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            question: "Does source matter?".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: Default::default(),
            cooldown: None,
            q_scope: Default::default(),
        };
        let tree_source = TreeSource::Staged;
        let mut visible_tree_oid_cache = VisibleTreeOidCache::default();

        let affected = visible_tree_oid_is_affected_by_pathspecs(
            &root,
            &tree_source,
            &expectation,
            &["src/app.rs".to_string(), "missing.rs".to_string()],
            &["src/app.rs".to_string()],
            &mut visible_tree_oid_cache,
        )
        .unwrap();

        let _ = fs::remove_dir_all(root);

        assert!(affected);
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!(
                "canon-show-selection-{}-{}-{}",
                name,
                process::id(),
                unique
            ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        // xpec: r8
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
