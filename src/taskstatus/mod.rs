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

macro_rules! increment_status {
    () => {
        let mut stat = TASK_STATUS_QUEUE.lock().unwrap();
        match &mut stat.status {
            Some(TaskStatus::TaskPercentage(name, len, val)) => {
                info!("Updating task status with value {}", val);
                stat.status = Some(TaskStatus::TaskPercentage(name.to_owned(), *len, *val + 1))
            }
            None => {}
        }
    };
}

macro_rules! set_task_status {
    ($name:expr, $len:expr, $cnt:expr) => {
        taskstatus::TASK_STATUS_QUEUE.lock().unwrap().status =
            Some(TaskStatus::TaskPercentage($name.to_owned(), $len, $cnt));
    };
}

macro_rules! set_task_completed {
    () => {
        taskstatus::TASK_STATUS_QUEUE.lock().unwrap().status = None
    };
}
