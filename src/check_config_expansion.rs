use crate::check_generator_paths::expand_generator_paths;
use crate::check_validation::normalize_agent_ignore_pattern_for_config;
use crate::config_types::{
    AgentConfig, CheckConfig, Expectation, RawCheckConfig, RawExpectationItem,
    RawGeneratorExpectation, RawIncludeExpectation,
};
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

#[cfg(test)]
use std::fs;

pub(crate) fn expand_raw_check_config(
    root: Option<&Path>,
    config_path: &Path,
    raw: RawCheckConfig,
    cache: Option<&mut RepoInspectionCache>,
    source: CheckConfigSource,
) -> Result<CheckConfig, String> {
    let mut expansion = RawExpectationExpansion {
        root,
        cache,
        source,
        include_stack: Vec::new(),
        expectations: Vec::new(),
    };
    expansion.expand_items(config_path, raw.expectations)?;
    Ok(CheckConfig {
        version: raw.version,
        agent: normalize_agent_config(raw.agent)?,
        expectations: expansion.expectations,
    })
}

fn normalize_agent_config(mut agent: AgentConfig) -> Result<AgentConfig, String> {
    for pattern in &mut agent.ignore {
        *pattern = normalize_agent_ignore_pattern_for_config(pattern)?;
    }
    Ok(agent)
}

#[derive(Clone, Copy)]
pub(crate) enum CheckConfigSource {
    Staged,
    #[cfg(test)]
    Worktree,
}

impl CheckConfigSource {
    fn is_staged(self) -> bool {
        match self {
            CheckConfigSource::Staged => true,
            #[cfg(test)]
            CheckConfigSource::Worktree => false,
        }
    }
}

struct RawExpectationExpansion<'a> {
    root: Option<&'a Path>,
    cache: Option<&'a mut RepoInspectionCache>,
    source: CheckConfigSource,
    include_stack: Vec<String>,
    expectations: Vec<Expectation>,
}

impl RawExpectationExpansion<'_> {
    fn expand_items(
        &mut self,
        config_path: &Path,
        items: Vec<RawExpectationItem>,
    ) -> Result<(), String> {
        for (index, item) in items.into_iter().enumerate() {
            match item {
                RawExpectationItem::Explicit(item) => self.expectations.push(Expectation {
                    q: item.q,
                    a: item.a,
                    prompt_scope: Vec::new(),
                    cooldown: item.cooldown,
                    thinking: item.thinking,
                }),
                RawExpectationItem::Generator(item) => {
                    self.expand_path_generator(config_path, index, item)?
                }
                RawExpectationItem::Include(item) => {
                    self.expand_include(config_path, index, item)?
                }
            }
        }
        Ok(())
    }

    fn expand_path_generator(
        &mut self,
        config_path: &Path,
        index: usize,
        item: RawGeneratorExpectation,
    ) -> Result<(), String> {
        let item_number = index + 1;
        let files = self.expand_paths(config_path, &item.path, item_number, "path")?;
        let uses_content = item.q_template.contains("{{content}}");
        for file in files {
            let content = if uses_content {
                self.read_expanded_file(&file)?
            } else {
                String::new()
            };
            self.expectations.push(Expectation {
                q: render_generator_question(&item.q_template, &content),
                a: item.a.clone(),
                prompt_scope: uses_content.then(|| vec![file]).unwrap_or_default(),
                cooldown: item.cooldown.clone(),
                thinking: item.thinking.clone(),
            });
        }
        Ok(())
    }

    fn expand_include(
        &mut self,
        config_path: &Path,
        index: usize,
        item: RawIncludeExpectation,
    ) -> Result<(), String> {
        let item_number = index + 1;
        let files = self.expand_paths(config_path, &item.include, item_number, "include")?;
        for file in files {
            if self.include_stack.contains(&file) {
                return Err(format!("recursive expectation include: {}", file));
            }
            self.include_stack.push(file.clone());
            let result = (|| {
                let content = self.read_expanded_file(&file)?;
                let included = self.parse_included_items(&file, &content)?;
                self.expand_items(Path::new(&file), included)
            })();
            self.include_stack.pop();
            result?;
        }
        Ok(())
    }

    fn expand_paths(
        &mut self,
        config_path: &Path,
        path: &str,
        item_number: usize,
        label: &str,
    ) -> Result<Vec<String>, String> {
        let root = self.root.ok_or_else(|| {
            format!(
                "expectation {} uses {} but config expansion has no project root",
                item_number, label
            )
        })?;
        let files = match self.cache.as_deref_mut() {
            Some(cache) => cache.generator_paths(root, config_path, path, self.source.is_staged()),
            None => expand_generator_paths(root, config_path, path, self.source.is_staged()),
        }?;
        Ok(files)
    }

    fn read_expanded_file(&mut self, file: &str) -> Result<String, String> {
        let root = self
            .root
            .ok_or_else(|| "config expansion has no project root".to_string())?;
        match self.source {
            CheckConfigSource::Staged => match self.cache.as_deref_mut() {
                Some(cache) => cache.staged_file_content(root, file),
                None => Err("staged config expansion requires RepoInspectionCache".to_string()),
            },
            #[cfg(test)]
            CheckConfigSource::Worktree => {
                let absolute = root.join(file);
                match self.cache.as_deref_mut() {
                    Some(cache) => cache.read_to_string(&absolute),
                    None => fs::read_to_string(&absolute)
                        .map_err(|err| format!("failed to read {}: {}", file, err)),
                }
            }
        }
    }

    fn parse_included_items(
        &mut self,
        file: &str,
        content: &str,
    ) -> Result<Vec<RawExpectationItem>, String> {
        match self.cache.as_deref_mut() {
            Some(cache) => cache.included_expectation_items(file, content),
            None => serde_saphyr::from_str(content)
                .map_err(|err| format!("failed to parse {}: {}", file, err)),
        }
    }
}

fn render_generator_question(template: &str, content: &str) -> String {
    // The expectations spec defines generator rendering as plain
    // `{{content}}` substitution: no placeholder leaves the template
    // unchanged, and repeated placeholders all receive the matched file
    // contents.
    template.replace("{{content}}", content)
}
