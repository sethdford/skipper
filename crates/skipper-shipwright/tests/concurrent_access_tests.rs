//! Concurrent access tests for cache and shared data structures.

#[cfg(test)]
mod tests {
    use dashmap::DashMap;
    use std::sync::Arc;
    use tokio::task;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dashmap_concurrent_insert_read() {
        let cache = Arc::new(DashMap::new());

        // Insert initial values from multiple tasks
        let mut tasks = vec![];

        for i in 0..10 {
            let cache_clone = Arc::clone(&cache);
            let task = task::spawn(async move {
                cache_clone.insert(format!("key-{}", i), i * 10);
            });
            tasks.push(task);
        }

        // Wait for all inserts
        for task in tasks {
            task.await.unwrap();
        }

        // Verify all values are present
        assert_eq!(cache.len(), 10);

        // Read values concurrently
        let mut read_tasks = vec![];
        for i in 0..10 {
            let cache_clone = Arc::clone(&cache);
            let task = task::spawn(async move {
                cache_clone
                    .get(&format!("key-{}", i))
                    .map(|entry| *entry)
            });
            read_tasks.push(task);
        }

        // Verify all reads succeed
        for task in read_tasks {
            let result = task.await.unwrap();
            assert!(result.is_some());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dashmap_concurrent_update() {
        let cache = Arc::new(DashMap::new());
        cache.insert("counter", 0);

        // Update same key from multiple tasks
        let mut tasks = vec![];
        for _ in 0..5 {
            let cache_clone = Arc::clone(&cache);
            let task = task::spawn(async move {
                for _ in 0..10 {
                    cache_clone.alter(&"counter", |_, v| v + 1);
                }
            });
            tasks.push(task);
        }

        // Wait for all updates
        for task in tasks {
            task.await.unwrap();
        }

        // Final value should be 50 (5 tasks * 10 increments)
        let final_value = *cache.get("counter").unwrap();
        assert_eq!(final_value, 50);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dashmap_concurrent_insert_remove() {
        let cache = Arc::new(DashMap::new());

        // Insert values
        for i in 0..5 {
            cache.insert(format!("key-{}", i), i);
        }

        // Remove and re-insert from multiple tasks concurrently
        let mut tasks = vec![];
        for i in 0..5 {
            let cache_clone = Arc::clone(&cache);
            let task = task::spawn(async move {
                cache_clone.remove(&format!("key-{}", i));
                cache_clone.insert(format!("key-{}", i), i * 2);
            });
            tasks.push(task);
        }

        // Wait for all operations
        for task in tasks {
            task.await.unwrap();
        }

        // Verify all values are updated
        assert_eq!(cache.len(), 5);
        for i in 0..5 {
            let value = cache.get(&format!("key-{}", i)).unwrap();
            assert_eq!(*value, i * 2);
        }
    }

    #[test]
    fn test_dashmap_no_deadlock_under_contention() {
        use std::thread;
        use std::time::Duration;

        let cache = Arc::new(DashMap::new());

        let mut handles = vec![];
        for task_id in 0..4 {
            let cache_clone = Arc::clone(&cache);
            let handle = thread::spawn(move || {
                for i in 0..10 {
                    let key = format!("task-{}-key-{}", task_id, i);
                    cache_clone.insert(key.clone(), i);
                    // Read without holding a guard across the remove
                    let _value = cache_clone.get(&key).map(|r| *r);
                    cache_clone.remove(&key);
                }
            });
            handles.push(handle);
        }

        // Use a timeout to detect deadlocks
        let start = std::time::Instant::now();
        for handle in handles {
            // If threads don't finish in 5 seconds, something is wrong
            while !handle.is_finished() {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("Deadlock detected: threads did not finish in 5 seconds");
                }
                thread::sleep(Duration::from_millis(10));
            }
            handle.join().unwrap();
        }

        assert_eq!(cache.len(), 0);
    }
}
