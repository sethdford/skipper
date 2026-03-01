//! Concurrent access tests for cache and shared data structures.

#[cfg(test)]
mod tests {
    use dashmap::DashMap;
    use std::sync::Arc;
    use tokio::task;

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
    async fn test_dashmap_no_deadlock_under_contention() {
        let cache = Arc::new(DashMap::new());

        // Create many concurrent operations
        let mut tasks = vec![];
        for task_id in 0..20 {
            let cache_clone = Arc::clone(&cache);
            let task = task::spawn(async move {
                for i in 0..50 {
                    let key = format!("task-{}-key-{}", task_id, i);
                    cache_clone.insert(key.clone(), i);

                    // Read back immediately
                    let _value = cache_clone.get(&key);

                    // Remove
                    cache_clone.remove(&key);
                }
            });
            tasks.push(task);
        }

        // All tasks should complete without deadlock
        for task in tasks {
            task.await.unwrap();
        }

        // Cache should be empty at the end
        assert_eq!(cache.len(), 0);
    }
}
