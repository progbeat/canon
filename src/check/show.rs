use crate::check::command::output::{escape_check_output_text, write_stdout_record};
use crate::check::interrogation::policy::initial_q_scope_for_fresh_interrogation;
use crate::check::run::selection::{
    expectation_identities, order_by_absent_non_pass_history, order_by_latest_non_pass,
    select_expectations_with_identities,
};
use crate::check::CHECK_PATH;
use crate::check::{CheckRunCaches, SelectedExpectation};
use crate::cli::CommandError;
use crate::git::{validate_tree_arg, TreeSource, VisibleTreeOidCache, STAGED_TREE_ARG};
use crate::notes::arg_to_string;
use crate::repo_inspection::RepoInspectionCache;
use crate::scope::visible_scope;
use crate::xpec_state::XpecStateCache;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, Command};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::Path;

pub(crate) fn run_show_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    let command = parse_show_command_args(args)?;
    let tree_source = TreeSource::resolve(root, &command.tree, "--tree")?;
    let mut repo_cache = RepoInspectionCache::new();
    let config = repo_cache.load_check_config(root, Path::new(CHECK_PATH), &tree_source)?;
    let mut caches = CheckRunCaches::new();
    let expectations = select_show_expectations_for_current_run(ShowRenderRequest {
        root,
        config: &config,
        tree_source: Some(&tree_source),
        selectors: &command.selectors,
        pathspecs: &command.pathspecs,
        excluded_expectation_id: None,
        xpec_state: &mut caches.xpec_state,
        visible_tree_oid_cache: &mut caches.visible_tree_oid,
    })?;
    write_show_expectations(&expectations).map_err(CommandError::from)
}

pub(crate) fn show_help_command() -> Command {
    Command::new("show")
        .bin_name("canon show")
        .about("Show canon expectations.")
        .arg(
            show_value_arg("tree")
                .long("tree")
                .value_name("TREE")
                .help("Use this Git tree for pathspec filtering [default: :staged]"),
        )
        .arg(
            Arg::new("selectors")
                .value_name("SELECTOR")
                .help("Expectation selectors: <ID-PREFIX> or not:<ID-PREFIX>")
                .num_args(0..)
                .action(ArgAction::Append)
                .value_parser(OsStringValueParser::new()),
        )
        .arg(
            Arg::new("pathspecs")
                .value_name("PATHSPEC")
                .help("Limit output to expectations affected by changes matching these pathspecs")
                .num_args(1..)
                .last(true)
                .action(ArgAction::Append)
                .value_parser(OsStringValueParser::new()),
        )
}

struct ShowCommandArgs {
    tree: String,
    selectors: Vec<OsString>,
    pathspecs: Vec<String>,
}

pub(crate) struct ShowRenderRequest<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a crate::config_types::CheckConfig,
    pub(crate) tree_source: Option<&'a TreeSource>,
    pub(crate) selectors: &'a [OsString],
    pub(crate) pathspecs: &'a [String],
    pub(crate) excluded_expectation_id: Option<&'a str>,
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
        .map(|expectation| expectation.id.clone())
        .collect();
    Ok(ShowRenderedOutput {
        text: render_show_output(&ordered),
        expectation_ids,
    })
}

fn select_show_expectations_for_current_run(
    request: ShowRenderRequest<'_>,
) -> Result<Vec<SelectedExpectation>, String> {
    let identities = expectation_identities(request.config)?;
    // Shared with `canon check`; this handles include selectors and
    // `not:<ID-PREFIX>` exclusions before pathspec filtering.
    let mut selected =
        select_expectations_with_identities(request.config, &identities, request.selectors)?;
    if let Some(excluded_expectation_id) = request.excluded_expectation_id {
        selected.retain(|expectation| expectation.id != excluded_expectation_id);
    }
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
    Ok(match request.tree_source {
        Some(_) => {
            order_by_latest_non_pass(request.root, filtered, request.xpec_state, |expectation| {
                expectation
            })?
        }
        None => order_by_absent_non_pass_history(filtered),
    })
}

fn parse_show_command_args(args: &[OsString]) -> Result<ShowCommandArgs, String> {
    let matches = show_help_command()
        .no_binary_name(true)
        .disable_version_flag(true)
        .try_get_matches_from(args)
        .map_err(|err| err.to_string())?;
    let tree = match matches.get_one::<OsString>("tree") {
        Some(value) => {
            let value = arg_to_string(value)?;
            validate_tree_arg(&value, "--tree")?;
            value
        }
        None => STAGED_TREE_ARG.to_string(),
    };
    let pathspecs = matches
        .get_many::<OsString>("pathspecs")
        .unwrap_or_default()
        .map(arg_to_string)
        .collect::<Result<Vec<_>, _>>()?;
    if pathspecs.iter().any(|pathspec| pathspec.is_empty()) {
        return Err("pathspec must not be empty".to_string());
    }
    Ok(ShowCommandArgs {
        tree,
        selectors: matches
            .get_many::<OsString>("selectors")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        pathspecs,
    })
}

fn show_value_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .num_args(1)
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new())
}

fn filter_expectations_by_pathspecs(
    root: &Path,
    tree_source: &TreeSource,
    expectations: Vec<SelectedExpectation>,
    pathspecs: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    xpec_state: &mut XpecStateCache,
) -> Result<Vec<SelectedExpectation>, String> {
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
    expectation: &SelectedExpectation,
    pathspecs: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    xpec_state: &mut XpecStateCache,
) -> Result<bool, String> {
    // This chooses the q-scope used for the selected-tree visible OID below.
    // Under the `canon show` pathspec rule, "the visible tree OID would change
    // if every tracked file matched by the pathspecs changed" is equivalent to
    // "at least one tracked file is in both the visible scope and the pathspecs".
    // The helper below implements that OID-change predicate by testing the
    // overlap directly instead of materializing a synthetic changed tree.
    let q_scope = show_q_scope(root, tree_source, expectation, xpec_state)?;
    let visible_scope = visible_scope(&expectation.agent, &q_scope)?;
    visible_tree_oid_cache.visible_scope_intersects_pathspecs(
        root,
        tree_source,
        &visible_scope,
        pathspecs,
    )
}

fn show_q_scope(
    root: &Path,
    _tree_source: &TreeSource,
    expectation: &SelectedExpectation,
    xpec_state: &mut XpecStateCache,
) -> Result<Vec<String>, String> {
    initial_q_scope_for_fresh_interrogation(root, expectation, xpec_state)
}

fn write_show_expectations(expectations: &[SelectedExpectation]) -> Result<(), String> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write_show_expectations_to(&mut stdout, expectations)
}

fn write_show_expectations_to(
    output: &mut dyn std::io::Write,
    expectations: &[SelectedExpectation],
) -> Result<(), String> {
    for expectation in expectations {
        write_stdout_record(
            output,
            render_show_expectation(expectation).as_bytes(),
            "show expectation",
        )?;
    }
    Ok(())
}

fn render_show_output(expectations: &[SelectedExpectation]) -> String {
    expectations
        .iter()
        .map(render_show_expectation)
        .collect::<String>()
}

fn render_show_expectation(expectation: &SelectedExpectation) -> String {
    format!(
        "{}.\n{}\nExpected: {}\n",
        expectation.display_id,
        escape_check_output_text(&expectation.question),
        escape_check_output_text(&expectation.expected_answer)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command as ProcessCommand;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parse_show_splits_selectors_from_pathspecs_after_separator() {
        let parsed = parse_show_command_args(&[
            OsString::from("abc"),
            OsString::from("--"),
            OsString::from("src/lib.rs"),
        ])
        .unwrap();

        assert_eq!(parsed.selectors, vec![OsString::from("abc")]);
        assert_eq!(parsed.pathspecs, vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn parse_show_supports_pathspecs_without_selectors() {
        let parsed = parse_show_command_args(&[
            OsString::from("--"),
            OsString::from("src/lib.rs"),
            OsString::from("tests"),
        ])
        .unwrap();

        assert!(parsed.selectors.is_empty());
        assert_eq!(
            parsed.pathspecs,
            vec!["src/lib.rs".to_string(), "tests".to_string()]
        );
    }

    #[test]
    fn parse_show_accepts_separator_without_pathspecs() {
        let parsed = parse_show_command_args(&[OsString::from("--")]).unwrap();

        assert!(parsed.selectors.is_empty());
        assert!(parsed.pathspecs.is_empty());
    }

    #[test]
    fn show_output_escapes_question_and_expected_answer() {
        let expectation = SelectedExpectation {
            number: 1,
            id: "11111111111111111111".to_string(),
            display_id: "1".to_string(),
            question: "Line one\nLine two".to_string(),
            expected_answer: "yes\tplease".to_string(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: false,
            agent: Default::default(),
            cooldown: None,
        };

        assert_eq!(
            render_show_expectation(&expectation),
            "1.\nLine one\\nLine two\nExpected: yes\\tplease\n"
        );
    }

    #[test]
    fn show_selector_supports_not_prefix() {
        let root = git_project("canon-show-not-selector");
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
        git(&root, &["add", ".canon/check.yml"]);
        let alpha_id = crate::hash::expectation_id("Does alpha pass?", "yes", "");
        let selector = format!("not:{}", alpha_id);

        let mut output = Vec::new();
        let result = run_show_for_test(&root, &[selector.as_str()], &mut output);

        let _ = fs::remove_dir_all(root);

        result.unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("Does alpha pass?"));
        assert!(output.contains("Does beta pass?"));
    }

    #[test]
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

        let mut output = Vec::new();
        let result = run_show_for_test(&root, &["--", "src/app.rs"], &mut output);

        let _ = fs::remove_dir_all(root);

        result.unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Does source matter?"));
        assert!(!output.contains("Does ignored source matter?"));
    }

    // xpec: G6
    #[test]
    fn current_run_show_excludes_current_expectation_even_when_explicitly_selected() {
        let root = git_project("canon-show-excludes-current");
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
        git(&root, &["add", ".canon/check.yml"]);
        let alpha_id = crate::hash::expectation_id("Does alpha pass?", "yes", "");
        let selector = OsString::from(alpha_id.clone());
        let tree_source = TreeSource::Staged;
        let mut repo_cache = RepoInspectionCache::new();
        let config = repo_cache
            .load_check_config(&root, Path::new(CHECK_PATH), &tree_source)
            .unwrap();
        let mut caches = CheckRunCaches::new();

        let rendered = render_show_for_current_run(ShowRenderRequest {
            root: &root,
            config: &config,
            tree_source: Some(&tree_source),
            selectors: &[selector],
            pathspecs: &[],
            excluded_expectation_id: Some(&alpha_id),
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid,
        })
        .unwrap();

        let _ = fs::remove_dir_all(root);

        assert_eq!(rendered.text, "");
        assert!(rendered.expectation_ids.is_empty());
    }

    fn run_show_for_test(
        root: &Path,
        args: &[&str],
        output: &mut dyn std::io::Write,
    ) -> Result<(), CommandError> {
        let command =
            parse_show_command_args(&args.iter().map(OsString::from).collect::<Vec<OsString>>())?;
        let tree_source = TreeSource::resolve(root, &command.tree, "--tree")?;
        let mut repo_cache = RepoInspectionCache::new();
        let config = repo_cache.load_check_config(root, Path::new(CHECK_PATH), &tree_source)?;
        let mut caches = CheckRunCaches::new();
        let expectations = select_show_expectations_for_current_run(ShowRenderRequest {
            root,
            config: &config,
            tree_source: Some(&tree_source),
            selectors: &command.selectors,
            pathspecs: &command.pathspecs,
            excluded_expectation_id: None,
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid,
        })?;
        write_show_expectations_to(output, &expectations).map_err(CommandError::from)?;
        Ok(())
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
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
