use std::sync::{Arc, Mutex};

pub enum TaskStatus {
    TaskPercentage(String, usize, usize),
}

#[derive(Default)]
pub struct TaskStatusContainer {
    pub status: Option<TaskStatus>,
}

lazy_static! {
    pub static ref TASK_STATUS_QUEUE: Arc<Mutex<TaskStatusContainer>> =
        Arc::new(Mutex::new(TaskStatusContainer::default()));
}

pub fn increment_status() {
    let mut stat = TASK_STATUS_QUEUE.lock().unwrap();
    match &mut stat.status {
        Some(TaskStatus::TaskPercentage(name, len, val)) => {
            info!("Updating task status with value {}", val);
            stat.status = Some(TaskStatus::TaskPercentage(name.to_owned(), *len, *val + 1))
        }
        None => {}
    }
}

pub fn set_task_status(task_name: &str, len: usize, cnt: usize) {
    TASK_STATUS_QUEUE.lock().unwrap().status =
        Some(TaskStatus::TaskPercentage(task_name.to_owned(), len, cnt))
}

pub fn set_task_completed() {
    TASK_STATUS_QUEUE.lock().unwrap().status = None
}
