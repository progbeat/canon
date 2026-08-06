mod glob;
mod path;

pub(crate) use glob::expand_foreach_paths_from_listing;
pub(crate) use path::resolve_foreach_read_path;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test] // xpec: jM
    fn star_foreach_glob_matches_one_path_segment() {
        let files = expand_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &staged_paths(&[
                "specs/root.md",
                "specs/nested/child.md",
                "specs/nested/child.txt",
                "src/root.md",
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/root.md"]);
    }

    #[test] // xpec: jM
    fn double_star_foreach_glob_matches_nested_path_segments() {
        let files = expand_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/**.md",
            &staged_paths(&[
                "specs/root.md",
                "specs/nested/child.md",
                "specs/nested/deeper/child.md",
                "specs/nested/child.txt",
                "src/root.md",
            ]),
        )
        .unwrap();

        assert_eq!(
            files,
            vec![
                "specs/nested/child.md",
                "specs/nested/deeper/child.md",
                "specs/root.md"
            ]
        );
    }

    #[test] // xpec: jM
    fn path_segment_foreach_globs_match_scope_pathspecs() {
        let files = expand_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*/*.md",
            &staged_paths(&[
                "specs/root.md",
                "specs/nested/child.md",
                "specs/nested/deeper/child.md",
                "specs/other/child.md",
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/nested/child.md", "specs/other/child.md"]);
    }

    #[test] // xpec: jM
    fn question_mark_foreach_glob_matches_one_unicode_character() {
        let files = expand_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/?.md",
            &staged_paths(&["specs/é.md", "specs/ab.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/é.md"]);
    }

    #[test] // xpec: jM
    fn foreach_glob_preserves_each_repeated_path_selection() {
        let files = expand_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &staged_paths(&["specs/alpha.md", "specs/alpha.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/alpha.md", "specs/alpha.md"]);
    }

    #[test] // xpec: jM,nK
    fn non_utf8_paths_only_fail_when_the_glob_matches_them() {
        let unrelated_non_utf8 = vec![b'o', b't', b'h', b'e', b'r', b'/', 0xff];
        let files = expand_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &[b"specs/good.md".to_vec(), unrelated_non_utf8],
        )
        .unwrap();

        assert_eq!(files, vec!["specs/good.md"]);

        let matched_non_utf8 = vec![
            b's', b'p', b'e', b'c', b's', b'/', b'f', 0xff, b'.', b'm', b'd',
        ];
        let error = expand_foreach_paths_from_listing(
            Path::new("check.yml"),
            "specs/*.md",
            &[matched_non_utf8],
        )
        .unwrap_err();

        assert_eq!(
            error,
            "!foreach matched a non-UTF-8 file path that cannot be bound to `path`"
        );
    }

    #[test] // xpec: jM
    fn foreach_paths_stay_relative_to_the_document_directory() {
        let files = expand_foreach_paths_from_listing(
            Path::new(".canon/includes/xpecs.yml"),
            "specs/*.md",
            &staged_paths(&[".canon/includes/specs/alpha.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/alpha.md"]);

        let files = expand_foreach_paths_from_listing(
            Path::new(".canon/includes/xpecs.yml"),
            "../specs/*.md",
            &staged_paths(&[".canon/specs/root.md"]),
        )
        .unwrap();

        assert_eq!(files, vec!["../specs/root.md"]);
        assert_eq!(
            resolve_foreach_read_path(Path::new(".canon/includes/xpecs.yml"), "../specs/root.md")
                .unwrap(),
            ".canon/specs/root.md"
        );
    }

    #[test] // xpec: jM
    fn wildcard_characters_in_the_document_directory_are_literal() {
        let files = expand_foreach_paths_from_listing(
            Path::new(".canon/include*?/xpecs.yml"),
            "specs/*.md",
            &staged_paths(&[
                ".canon/include*?/specs/right.md",
                ".canon/include12/specs/sibling.md",
                ".canon/include*?/other/wrong.md",
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["specs/right.md"]);
    }

    #[test] // xpec: jM
    fn a_glob_segment_keeps_pattern_semantics_when_it_matches_a_literal_directory_name() {
        let files = expand_foreach_paths_from_listing(
            Path::new(".canon/include*/xpecs.yml"),
            "../include*/specs/*.md",
            &staged_paths(&[
                ".canon/include*/specs/own.md",
                ".canon/include12/specs/sibling.md",
                ".canon/other/specs/wrong.md",
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["../include12/specs/sibling.md", "specs/own.md"]);
    }

    fn staged_paths(paths: &[&str]) -> Vec<Vec<u8>> {
        paths.iter().map(|path| path.as_bytes().to_vec()).collect()
    }
}
