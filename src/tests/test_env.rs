use super::*;

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());
pub(crate) static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(name: &str) -> TestDir {
        let path = test_path(name);
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create test directory");
        TestDir { path }
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.path.clone()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn temp_home(name: &str) -> PathBuf {
    let path = test_path(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create test directory");
    path
}

pub(crate) fn test_path(name: &str) -> PathBuf {
    let unique = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-tmp")
        .join(format!("canon-test-{}-{}-{}", name, process::id(), unique))
}

pub(crate) struct EnvSnapshot {
    values: Vec<(&'static str, Option<OsString>)>,
}

impl EnvSnapshot {
    pub(crate) fn capture(keys: &[&'static str]) -> EnvSnapshot {
        EnvSnapshot {
            values: keys.iter().map(|key| (*key, env::var_os(key))).collect(),
        }
    }

    pub(crate) fn set(&self, key: &str, value: impl AsRef<std::ffi::OsStr>) {
        env::set_var(key, value);
    }

    pub(crate) fn remove(&self, key: &str) {
        env::remove_var(key);
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        for (key, value) in &self.values {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }
}

pub(crate) fn with_env<F>(name: &str, f: F)
where
    F: FnOnce(PathBuf),
{
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "CODEX_THREAD_ID"]);
    let home = TestDir::new(name);
    env_snapshot.set("CANON_HOME", home.path());
    env_snapshot.set("CODEX_THREAD_ID", "thread-test");
    f(home.path());
}
