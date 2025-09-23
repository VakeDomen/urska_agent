use actix::{Addr, Handler, Message};
use serde::{Deserialize, Serialize};

use crate::{
    profile::Profile, 
    queue::PositionInQueue, 
    session::ChatSession
};


#[derive(Debug, Deserialize)]
pub enum MessageType {
    StudentLogin,
    EmployeeLogin,
    ThumbsUp,
    ThumbsDown,
    Logout,
    Prompt,
}

#[derive(Debug, Deserialize)]
pub struct FrontendMessage {
    pub message_type: MessageType,
    pub content: String,
}


#[derive(Debug, Deserialize)]
pub struct LoginCredentials {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
pub enum BackendMessage {
    Chunk(String),
    Notification(String),
    QueuePosition(PositionInQueue),
    LoginProfile(Profile),
    Error(String),
    End,
}

pub struct SendWsText(pub String);
impl Message for SendWsText {
    type Result = ();
}

impl Handler<SendWsText> for ChatSession {
    type Result = ();

    fn handle(&mut self, msg: SendWsText, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

pub trait SendMessage {
    fn send_message_to_client(&self, end: BackendMessage) -> Result<(), serde_json::Error>;
}

impl SendMessage for Addr<ChatSession> {
    fn send_message_to_client(&self, end: BackendMessage) -> Result<(), serde_json::Error> {
        let content = serde_json::to_string(&end)?;
        let message = SendWsText(content);
        self.do_send(message);
        Ok(())
    }
}

