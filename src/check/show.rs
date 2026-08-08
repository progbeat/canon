mod args;

use crate::check::expectation_identities;
#[cfg(test)]
use crate::check::expectation_inspection::render_show_for_current_run;
use crate::check::expectation_inspection::{
    select_show_expectations_for_current_run, write_show_expectations, ShowRenderRequest,
};
use crate::check::CHECK_PATH;
use crate::check::{load_check_config, CheckRunCaches};
use crate::cli::CommandError;
use args::parse_show_command_args;
pub(crate) use args::show_help_command;
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn run_show_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    let command = parse_show_command_args(args)?;
    let mut caches = CheckRunCaches::new();
    // `--tree` selects one coherent repository snapshot: expectation
    // collection and pathspec/visible-tree filtering must use the same OID.
    let tree_source =
        caches
            .repo_inspection
            .resolve_tree_to_oid_source(root, &command.tree, "--tree")?;
    let config = load_check_config(
        &mut caches.repo_inspection,
        root,
        Path::new(CHECK_PATH),
        &tree_source,
    )?;
    let identities = expectation_identities(&config)?;
    let expectations = select_show_expectations_for_current_run(ShowRenderRequest {
        root,
        config: &config,
        identities: &identities,
        tree_source: Some(&tree_source),
        selectors: &command.selectors,
        pathspecs: &command.pathspecs,
        current_expectation_id: None,
        xpec_state: &mut caches.xpec_state,
        visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
    })?;
    write_show_expectations(&expectations).map_err(CommandError::from)
}

#[cfg(test)]
mod co_located_unit_tests {
    use super::*;
    use crate::git::TreeSource;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command as ProcessCommand;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: 2gZ
    fn pathspec_filter_respects_expectation_visible_scope() {
        let root = git_project("canon-show-filter");
        fs::create_dir_all(root.join(".canon")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/app.rs"), "fn main() {}\n").unwrap();
        fs::write(
            root.join(".canon/check.yml"),
            r#"version: 1
presets:
  default: {}
expectations:
  - q: Does source matter?
    a: yes
  - q: Does ignored source matter?
    a: yes
    ignore: ["src/**"]
"#,
        )
        .unwrap();
        git(&root, &["add", ".canon/check.yml", "src/app.rs"]);

        let output = render_show_for_test(&root, &[], &["src/app.rs".to_string()]).unwrap();
        let _ = fs::remove_dir_all(root);

        assert!(output.contains("Does source matter?"));
        assert!(!output.contains("Does ignored source matter?"));
    }

    #[test] // xpec: 2gZ
    fn pathspec_filter_uses_git_wildcard_semantics() {
        let root = git_project("canon-show-filter-git-wildcard");
        fs::create_dir_all(root.join(".canon")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/app.rs"), "fn main() {}\n").unwrap();
        fs::write(
            root.join(".canon/check.yml"),
            r#"version: 1
presets:
  default: {}
expectations:
  - q: Does wildcard-selected source matter?
    a: yes
"#,
        )
        .unwrap();
        git(&root, &["add", ".canon/check.yml", "src/app.rs"]);

        let output = render_show_for_test(&root, &[], &["src/*.rs".to_string()]).unwrap();
        let _ = fs::remove_dir_all(root);

        assert!(output.contains("Does wildcard-selected source matter?"));
    }

    #[test] // xpec: 2gZ
    fn show_selector_supports_not_prefix() {
        let root = git_project("canon-show-not-selector");
        write_two_expectations(&root);
        let alpha_id = crate::hash::expectation_id("Does alpha pass?", "agent", "yes", "");
        let selector = OsString::from(format!("not:{}", alpha_id));

        let output = render_show_for_test(&root, &[selector], &[]).unwrap();
        let _ = fs::remove_dir_all(root);

        assert!(!output.contains("Does alpha pass?"));
        assert!(output.contains("Does beta pass?"));
    }

    #[test] // xpec: 6,t
    fn dynamic_show_excludes_current_without_revealing_its_identity() {
        let root = git_project("canon-show-excludes-current");
        write_two_expectations(&root);
        let alpha_id = crate::hash::expectation_id("Does alpha pass?", "agent", "yes", "");
        let beta_id = crate::hash::expectation_id("Does beta pass?", "agent", "yes", "");
        let alpha_selector = OsString::from(alpha_id.clone());
        let beta_selector = OsString::from(beta_id);
        let unknown_selector = OsString::from("00000000000000000000");
        let tree_source = TreeSource::Staged;
        let mut caches = CheckRunCaches::new();
        let config = load_check_config(
            &mut caches.repo_inspection,
            &root,
            Path::new(CHECK_PATH),
            &tree_source,
        )
        .unwrap();
        let identities = expectation_identities(&config).unwrap();
        let alpha_display_id = identities
            .iter()
            .find(|identity| identity.id == alpha_id)
            .expect("alpha expectation identity exists")
            .display_id
            .clone();

        let rendered = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors: &[],
            pathspecs: &[],
            current_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })
        .unwrap();
        let known_candidate_error = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors: std::slice::from_ref(&alpha_selector),
            pathspecs: &[],
            current_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })
        .err()
        .expect("an explicitly included current expectation must remain hidden");
        let unknown_candidate_error = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors: std::slice::from_ref(&unknown_selector),
            pathspecs: &[],
            current_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })
        .err()
        .expect("an unknown include selector must fail");
        let known_candidate_with_other_error = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors: &[alpha_selector, beta_selector.clone()],
            pathspecs: &[],
            current_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })
        .err()
        .expect("the current expectation candidate must fail");
        let unknown_candidate_with_other_error = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors: &[unknown_selector, beta_selector],
            pathspecs: &[],
            current_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })
        .err()
        .expect("an unknown candidate must fail");
        let known_exclusion = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors: &[OsString::from(format!("not:{}", alpha_display_id))],
            pathspecs: &[],
            current_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })
        .unwrap();
        let unknown_exclusion = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors: &[OsString::from("not:00000000000000000000")],
            pathspecs: &[],
            current_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })
        .unwrap();
        let _ = fs::remove_dir_all(root);

        assert!(!rendered.text.contains("Does alpha pass?"));
        assert!(rendered.text.contains("Does beta pass?"));
        // [6,t] Candidate IDs derived from possible expected answers must be
        // observationally indistinguishable through the show component.
        assert_eq!(known_candidate_error, unknown_candidate_error);
        assert_eq!(
            known_candidate_with_other_error,
            unknown_candidate_with_other_error
        );
        assert_eq!(known_exclusion.text, unknown_exclusion.text);
        assert_eq!(
            known_exclusion.expectation_ids,
            unknown_exclusion.expectation_ids
        );
    }

    fn render_show_for_test(
        root: &Path,
        selectors: &[OsString],
        pathspecs: &[String],
    ) -> Result<String, String> {
        let tree_source = TreeSource::Staged;
        let mut caches = CheckRunCaches::new();
        let config = load_check_config(
            &mut caches.repo_inspection,
            root,
            Path::new(CHECK_PATH),
            &tree_source,
        )?;
        let identities = expectation_identities(&config)?;
        Ok(render_show_for_current_run(ShowRenderRequest {
            root,
            config: &config,
            identities: &identities,
            tree_source: Some(&tree_source),
            selectors,
            pathspecs,
            current_expectation_id: None,
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid_cache,
        })?
        .text)
    }

    fn write_two_expectations(root: &Path) {
        fs::create_dir_all(root.join(".canon")).unwrap();
        fs::write(
            root.join(".canon/check.yml"),
            r#"version: 1
presets:
  default: {}
expectations:
  - q: Does alpha pass?
    a: yes
  - q: Does beta pass?
    a: yes
"#,
        )
        .unwrap();
        git(root, &["add", ".canon/check.yml"]);
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["config", "core.eol", "lf"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = ProcessCommand::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        // xpec: 2gZ
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
