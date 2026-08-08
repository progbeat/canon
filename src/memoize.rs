use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) type MemoizedResult<T> = Arc<OnceLock<Result<T, String>>>;

pub(crate) fn mutex_memoized_result<S, K: Ord, T: Clone>(
    state: &Mutex<S>,
    key: K,
    poisoned_error: &str,
    map: impl for<'a> Fn(&'a S) -> &'a BTreeMap<K, MemoizedResult<T>>,
    map_mut: impl for<'a> Fn(&'a mut S) -> &'a mut BTreeMap<K, MemoizedResult<T>>,
    compute: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let result = {
        let mut state = state.lock().map_err(|_| poisoned_error.to_string())?;
        if let Some(result) = map(&state).get(&key).cloned() {
            result
        } else {
            let result = Arc::new(OnceLock::new());
            map_mut(&mut state).insert(key, Arc::clone(&result));
            result
        }
    };
    result.get_or_init(compute).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread;
    use std::time::Duration;

    #[test] // xpec: d
    fn concurrent_misses_for_one_key_compute_once() {
        let state = Arc::new(Mutex::new(BTreeMap::<String, MemoizedResult<usize>>::new()));
        let start = Arc::new(Barrier::new(3));
        let calls = Arc::new(AtomicUsize::new(0));
        let handles = (0..2)
            .map(|_| {
                let state = Arc::clone(&state);
                let start = Arc::clone(&start);
                let calls = Arc::clone(&calls);
                thread::spawn(move || {
                    start.wait();
                    mutex_memoized_result(
                        &state,
                        "key".to_string(),
                        "test cache is poisoned",
                        |entries| entries,
                        |entries| entries,
                        || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            thread::sleep(Duration::from_millis(20));
                            Ok(7)
                        },
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        start.wait();

        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(results, vec![7, 7]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
