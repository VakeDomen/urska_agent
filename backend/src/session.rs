use std::{sync::Arc};
use actix::prelude::*;
use actix_web_actors::ws;
use rmcp::{
    model::{CallToolRequestParam, ProgressNotificationParam},
    service::RunningService,
    ClientHandler,                     
};
use serde_json::{Map, Value};
use tokio::{fs, sync::{mpsc::{self, Receiver}, Mutex}};
use crate::{
    ldap::{employee_ldap_login, stdent_ldap_login}, 
    messages::{BackendMessage, FrontendMessage, LoginCredentials, MessageType, SendMessage}, 
    profile::Profile, 
    queue::{self, QueueManager, QueueMessage}
};
#[derive(Debug)]
pub struct ProgressHandler {
    pub notification_tx: mpsc::Sender<ProgressNotificationParam>,
}

impl ClientHandler for ProgressHandler {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _ctx: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        let _ = self.notification_tx.send(params).await;
    }
}



#[derive(Message)]
#[rtype(result = "()")]
struct Authenticated(Profile);

#[derive(Message)]
#[rtype(result = "()")]
struct Logout;

#[derive(Debug)]
pub struct ChatSession {
    pub id: String,
    pub mcp_client: RunningService<rmcp::RoleClient, ProgressHandler>,
    pub notification_reciever: Arc<Mutex<mpsc::Receiver<ProgressNotificationParam>>>,
    pub queue: Arc<Mutex<QueueManager>>,
    pub authenticated_as: Option<Profile>,
}

impl Handler<Authenticated> for ChatSession {
    type Result = ();

    fn handle(&mut self, msg: Authenticated, _: &mut Self::Context) {
        self.authenticated_as = Some(msg.0);
    }
}

impl Handler<Logout> for ChatSession {
    type Result = ();

    fn handle(&mut self, _msg: Logout, _: &mut Self::Context) {
        self.authenticated_as = None;
        println!("Logged out...")
    }
}


impl Actor for ChatSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // 1) take the actor address
        let addr = ctx.address();
        println!("New actor connected...");

        // 2) clone your receiver
        let notification_reciever = self.notification_reciever.clone();

        // 3) spawn a tokio task (or actix::spawn) that lives 'static
        // thread that forwards notifications | mcp -> BE -(here)> client
        actix::spawn(async move {
            while let Some(notification_params) = notification_reciever
                .lock()
                .await
                .recv()
                .await 
            {
                let text = notification_params
                    .message
                    .clone()
                    .unwrap_or_else(|| "▱".to_string());
                let msg = BackendMessage::Notification(text);
                let _ = addr.send_message_to_client(msg);
            }
        });
    }
}


impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChatSession {
    fn handle(
        &mut self,
        item: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        match item {
            Ok(ws::Message::Text(txt)) => {
                if let Ok(req) = serde_json::from_str::<FrontendMessage>(&txt) {
                    self.handle_message_from_client(ctx, req);
                }
            }
            Ok(ws::Message::Ping(p)) => ctx.pong(&p),
            Ok(ws::Message::Close(_)) => ctx.stop(),
            a => {
                println!("Something else recieved: {:#?}", a)

            }
        }
    }
}

impl ChatSession {
    fn handle_message_from_client(
        &mut self, 
        ctx: &mut ws::WebsocketContext<ChatSession>,
        message: FrontendMessage,
    ) {
        match &message.message_type {
            MessageType::Prompt => self.prompt(ctx, message.content),
            MessageType::EmployeeLogin => self.employee_login(ctx, message.content),
            MessageType::StudentLogin => self.student_login(ctx, message.content),
            MessageType::Logout => self.logout(ctx, message),
            MessageType::ThumbsUp => self.save_thumbs_up(ctx, message),
            MessageType::ThumbsDown => self.save_thumbs_down(ctx, message),
                    }
    }

    fn employee_login(
        &mut self, 
        ctx: &mut ws::WebsocketContext<ChatSession>,
        message: String,
    ) {
        println!("Employee Login");
        let addr: Addr<ChatSession> = ctx.address();
        
        let Ok(credentials) = serde_json::from_str::<LoginCredentials>(&message) else {
            let end = BackendMessage::Error("Invalid credentials shape".into());
            let _ = addr.send_message_to_client(end);
            return;
        };

        println!("{:#?}", credentials);

        actix::spawn(async move {

            let resp = match employee_ldap_login(credentials.username, credentials.password).await {
                Ok(r) => r,
                Err(e) =>  {
                    let end = BackendMessage::Error(format!("Auth connection failed: {:#?}", e));
                    let _ = addr.send_message_to_client(end);
                    return;
                },
            };
            
            println!("Checking credential validity");

            let Some(resp) = resp else {
                let end = BackendMessage::Error(format!("Invalid credentials"));
                let _ = addr.send_message_to_client(end);
                return;
            };

            println!("RESP: {:#?}", resp);

            let profile = match Profile::try_from_employee_string(resp) {
                Ok(p) => p,
                Err(e) => {
                    let end = BackendMessage::Error(format!("Profile parsing: {}", e));
                    let _ = addr.send_message_to_client(end);
                    return;
                },
            };

            let _  = addr.send_message_to_client(BackendMessage::LoginProfile(profile.clone()));
            addr.do_send(Authenticated(profile));
        });
    }



    fn student_login(
        &mut self, 
        ctx: &mut ws::WebsocketContext<ChatSession>,
        message: String,
    ) {
        println!("Student Login");
        let addr = ctx.address();
        
        let Ok(credentials) = serde_json::from_str::<LoginCredentials>(&message) else {
            let end = BackendMessage::Error("Invalid credentials shape".into());
            let _ = addr.send_message_to_client(end);
            return;
        };

        println!("{:#?}", credentials);

        actix::spawn(async move {

            let resp = match stdent_ldap_login(credentials.username, credentials.password).await {
                Ok(r) => r,
                Err(e) =>  {
                    let end = BackendMessage::Error(format!("LDAP failed: {:#?}", e));
                    let _ = addr.send_message_to_client(end);
                    return;
                },
            };

            println!("Checking credential validity");
            
            let Some(resp) = resp else {
                let end = BackendMessage::Error(format!("LDAP invalid credentials"));
                let _ = addr.send_message_to_client(end);
                return;
            };

            println!("RESP: {:#?}", resp);

            let profile = match Profile::try_from_student_string(resp) {
                Ok(p) => p,
                Err(e) => {
                    let end = BackendMessage::Error(format!("LDAP profile parsing: {}", e));
                    let _ = addr.send_message_to_client(end);
                    return;
                },
            };

            let _  = addr.send_message_to_client(BackendMessage::LoginProfile(profile.clone()));
            addr.do_send(Authenticated(profile));
        });
    }

    fn prompt(
        &mut self, 
        ctx: &mut ws::WebsocketContext<ChatSession>,
        message: String,
    ) {
        println!("Prompt");
        let client = self.mcp_client.clone();
        let addr = ctx.address();
        let queue = self.queue.clone();
        
        if self.authenticated_as.is_none() {
            let _ = addr.send_message_to_client(BackendMessage::Error("Not logged in...".into()));
            return;
        }
        
        
        actix::spawn(async move {
        
            let mut job_id: Option<uuid::Uuid> = None;
            
            let mut reciever: Receiver<QueueMessage> = {
                queue
                    .lock()
                    .await
                    .enter_queue()
                    .await
            };    
        
            {
                while let Some(message) = reciever
                    .recv()
                    .await
                {
                    match message {
                        queue::QueueMessage::StartJob(uuid) => {
                            job_id = Some(uuid);
                            break;
                        },
                        queue::QueueMessage::PositionUpade(position) => {
                            let end = BackendMessage::QueuePosition(position);
                            let _ = addr.send_message_to_client(end);
                            continue;
                        },
                    };
            
                }
            }
        
            let mut urska_argument_map = Map::new();
        
            urska_argument_map.insert(
                "question".to_string(), 
                Value::String(message.clone())
            );
        
        
            let fn_call_request = CallToolRequestParam {
                name: "ask_urska".into(),
                arguments: Some(urska_argument_map),
            };
        
            
            let result = client
                .call_tool(fn_call_request)
                .await;
        
            
            if let Err(e) = &result {
                let error_conetent = format!("[Tool error] {}",e);
                let err_msg = BackendMessage::Chunk(error_conetent);
                let _ = addr.send_message_to_client(err_msg);
            }
        
            // once done, send the End marker
            let end = BackendMessage::End;
            let _ = addr.send_message_to_client(end);
            
            queue
                .clone()
                .lock()
                .await
                .notify_done(job_id.unwrap())
                .await;
        
        });
    }
    
    fn logout(&self, ctx: &mut ws::WebsocketContext<ChatSession>, _: FrontendMessage) {
        let addr = ctx.address();
        addr.do_send(Logout);
    }
    
    fn save_thumbs_up(&self, ctx: &mut ws::WebsocketContext<ChatSession>, _message: FrontendMessage) {
        let client = self.mcp_client.clone();
        let session_id = self.id.clone();
    
        actix::spawn(async move {
            let fn_call_request = CallToolRequestParam {
                name: "export_conversation".into(),
                arguments: None,
            };
        
            let result = client
                .call_tool(fn_call_request)
                .await;
            let binding = result.unwrap().content.clone();
            let content = &binding[0].as_text().unwrap().text;

            let _ = fs::write(
                format!("up_{}.json", session_id), 
                content
            ).await;
        
        });
    }
    
    fn save_thumbs_down(&self, ctx: &mut ws::WebsocketContext<ChatSession>, _message: FrontendMessage) {
        let client = self.mcp_client.clone();
        let session_id = self.id.clone();
    
        actix::spawn(async move {
            let fn_call_request = CallToolRequestParam {
                name: "export_conversation".into(),
                arguments: None,
            };
        
            let result = client
                .call_tool(fn_call_request)
                .await;

            let binding = result.unwrap().content.clone();
            let content = &binding[0].as_text().unwrap().text;

            let _ = fs::write(
                format!("down_{}.json", session_id), 
                content
            ).await;
        
        });
    }
}

