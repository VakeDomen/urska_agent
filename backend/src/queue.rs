use actix::prelude::*;
use std::collections::VecDeque;
use tokio::sync::mpsc::{self, Receiver, Sender};
use uuid::Uuid;

pub const MAX_CONCURRENT: usize = 1; 

pub type PositionInQueue = usize;
pub type JobId = Uuid;
pub enum QueueMessage {
    PositionUpade(PositionInQueue),
    StartJob(JobId)
}


#[derive(Debug)]
struct QueueItem {
    job_id: JobId,
    sender: Sender<QueueMessage>,
}

#[derive(Debug)]
pub struct QueueManager {
    waiting: VecDeque<QueueItem>,
    running: Vec<QueueItem>,
}

impl QueueManager {
    pub fn new() -> Self {
        Self {
            waiting: VecDeque::new(),
            running: Vec::new(),
        }
    }

    async fn broadcast_positions(&self) {
        for (idx, item) in self.waiting.iter().enumerate() {
            if let Err(e) = item
                .sender
                .send(QueueMessage::PositionUpade(idx + 1))
                .await 
            {
                println!("Failed to send position message: {:#?}", e)
            } 
        }
    }

    pub async fn enter_queue(&mut self) -> Receiver<QueueMessage> {
        let job_id = Uuid::new_v4();
        let (sender, reciever) = mpsc::channel::<QueueMessage>(10);
        let item = QueueItem {
            job_id,
            sender
        };
        self.waiting.push_back(item);
        self.queue_update().await;
        reciever
    }

    async fn queue_update(&mut self) {
        while self.running.len() < MAX_CONCURRENT {
            if self.waiting.is_empty() {
                break;
            }

            let Some(next_item_to_run) = self.waiting.pop_front() else {
                break;
            };

            match next_item_to_run
                .sender
                .send(QueueMessage::StartJob(next_item_to_run.job_id.clone()))
                .await 
            {
                Ok(_) => self.running.push(next_item_to_run),
                Err(e) => println!("Failed to send StartJob message: {:#?}", e),
            } 
        }
        
        self.broadcast_positions().await;
    }

    pub async  fn notify_done(&mut self, done_job_id: JobId) {
        let mut index = None;
        
        for (i, job) in self
            .running
            .iter()
            .enumerate() 
        {
            if job.job_id.eq(&done_job_id) {
                index = Some(i);
                break;
            }
        }
        if let Some(i) = index {
            self.running.remove(i);
        }
        self.queue_update().await;
    }

}

impl Actor for QueueManager {
    type Context = Context<Self>;
}
