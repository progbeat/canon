use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub(crate) fn read_git_blobs(root: &Path, object_ids: &[String]) -> Result<Vec<Vec<u8>>, String> {
    if object_ids.is_empty() {
        return Ok(Vec::new());
    }
    GitBlobReader::new(root)?.read_blobs(object_ids)
}

pub(crate) struct GitBlobReader {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    blobs_by_object_id: BTreeMap<String, Vec<u8>>,
}

impl GitBlobReader {
    pub(crate) fn new(root: &Path) -> Result<GitBlobReader, String> {
        GitBlobReader::new_with_git_program(root, OsStr::new("git"))
    }

    fn new_with_git_program(root: &Path, git_program: &OsStr) -> Result<GitBlobReader, String> {
        let mut child = Command::new(git_program)
            .arg("-C")
            .arg(root)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("failed to run git cat-file: {}", err))?;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                return Err(cleanup_git_cat_file_child(
                    child,
                    "failed to open git cat-file stdin".to_string(),
                ))
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                drop(stdin);
                return Err(cleanup_git_cat_file_child(
                    child,
                    "failed to open git cat-file stdout".to_string(),
                ));
            }
        };
        Ok(GitBlobReader {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            blobs_by_object_id: BTreeMap::new(),
        })
    }

    pub(crate) fn read_blobs(&mut self, object_ids: &[String]) -> Result<Vec<Vec<u8>>, String> {
        let mut blobs = Vec::with_capacity(object_ids.len());
        for object_id in object_ids {
            if let Some(blob) = self.blobs_by_object_id.get(object_id) {
                blobs.push(blob.clone());
                continue;
            }
            let blob = self.read_blob(object_id)?;
            self.blobs_by_object_id
                .insert(object_id.clone(), blob.clone());
            blobs.push(blob);
        }
        Ok(blobs)
    }

    fn read_blob(&mut self, object_id: &str) -> Result<Vec<u8>, String> {
        writeln!(self.stdin, "{}", object_id)
            .map_err(|err| format!("failed to write git cat-file input: {}", err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to write git cat-file input: {}", err))?;

        let mut header = String::new();
        let bytes_read = self
            .stdout
            .read_line(&mut header)
            .map_err(|err| format!("failed to read git cat-file output: {}", err))?;
        if bytes_read == 0 {
            return Err(format!(
                "git cat-file output missing header for {}",
                object_id
            ));
        }
        let header = header.trim_end_matches('\n');
        let mut fields = header.split_whitespace();
        let actual_id = fields
            .next()
            .ok_or_else(|| "git cat-file header missing object id".to_string())?;
        let object_type = fields
            .next()
            .ok_or_else(|| format!("git cat-file header missing type for {}", actual_id))?;
        if object_type == "missing" {
            return Err(format!("Git blob {} is missing", actual_id));
        }
        if object_type != "blob" {
            return Err(format!(
                "Git object {} is {}, not blob",
                actual_id, object_type
            ));
        }
        let size = fields
            .next()
            .ok_or_else(|| format!("git cat-file header missing size for {}", actual_id))?
            .parse::<usize>()
            .map_err(|_| format!("git cat-file header has invalid size for {}", actual_id))?;
        let mut blob = vec![0; size];
        self.stdout
            .read_exact(&mut blob)
            .map_err(|_| format!("git cat-file output truncated for {}", actual_id))?;
        let mut delimiter = [0u8; 1];
        self.stdout.read_exact(&mut delimiter).map_err(|_| {
            format!(
                "git cat-file output missing object delimiter for {}",
                actual_id
            )
        })?;
        if delimiter != *b"\n" {
            return Err(format!(
                "git cat-file output missing object delimiter for {}",
                actual_id
            ));
        }
        Ok(blob)
    }
}

impl Drop for GitBlobReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn cleanup_git_cat_file_child(child: Child, message: String) -> String {
    match child.wait_with_output() {
        Ok(_) => message,
        Err(err) => format!("{}; failed to reap git cat-file: {}", message, err),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test] // xpec: d
    fn blob_reader_reuses_one_repository_read_for_repeated_object_id() {
        let root = std::env::temp_dir().join(format!(
            "canon-test-blob-reader-cache-{}-{:016x}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let fake_git = root.join("fake-git");
        fs::write(
            &fake_git,
            "#!/bin/sh\nIFS= read -r oid || exit 1\nprintf '%s blob 4\\nsame\\n' \"$oid\"\n",
        )
        .unwrap();
        fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();
        let object_id = "0123456789012345678901234567890123456789".to_string();
        let mut reader = GitBlobReader::new_with_git_program(&root, fake_git.as_os_str()).unwrap();

        let duplicated = reader
            .read_blobs(&[object_id.clone(), object_id.clone()])
            .unwrap();
        let later = reader.read_blobs(std::slice::from_ref(&object_id)).unwrap();

        assert_eq!(duplicated, vec![b"same".to_vec(), b"same".to_vec()]);
        assert_eq!(later, vec![b"same".to_vec()]);
        drop(reader);
        fs::remove_dir_all(root).unwrap();
    }
}
