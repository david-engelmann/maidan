use std::collections::HashMap;
use std::sync::RwLock;

use crate::protocol::Task;

#[derive(Debug, Default)]
pub struct TaskRegistry {
    inner: RwLock<HashMap<String, Task>>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, task: Task) {
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(task.id.clone(), task);
        }
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.inner.read().ok()?.get(id).cloned()
    }
}
