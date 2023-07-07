use gtk::glib::Sender;

pub enum TaskStatus {
    TaskPercentage(String, usize, usize),
}

#[derive(Default)]
pub struct TaskStatusContainer {
    pub status: Option<TaskStatus>,
}

pub fn set_task_status(
    sender: &Sender<TaskStatusContainer>,
    task_name: &str,
    num_parts: usize,
    progress: usize,
) {
    sender
        .send(TaskStatusContainer {
            status: Some(TaskStatus::TaskPercentage(
                task_name.to_owned(),
                num_parts,
                progress,
            )),
        })
        .expect("Failed to sent task status");
}

pub fn set_task_completed(sender: &Sender<TaskStatusContainer>) {
    sender
        .send(TaskStatusContainer { status: None })
        .expect("Failed to sent task status");
}
