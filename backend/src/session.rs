use std::sync::Arc;
use actix::prelude::*;
use actix_web::{Error, HttpRequest, HttpResponse, web};
use actix_web_actors::ws;
use rmcp::{
    model::{CallToolRequestParam, ProgressNotificationParam},
    service::RunningService,
    transport::StreamableHttpClientTransport,
    ClientHandler,                     
    ServiceExt,                        
};
use serde_json::{Map, Value};
use tokio::sync::{mpsc::{self, Receiver}, Mutex};
use crate::{
    ldap::{employee_ldap_login, stdent_ldap_login}, messages::{BackendMessage, FrontendMessage, LoginCredentials, MessageType, SendMessage, SendWsText}, profile::Profile, queue::{self, QueueManager, QueueMessage}
};
#[derive(Debug)]
struct ProgressHandler {
    notification_tx: mpsc::Sender<ProgressNotificationParam>,
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

pub async fn ws_index(
    req: HttpRequest,
    stream: web::Payload,
    queue: web::Data<Arc<Mutex<queue::QueueManager>>>,
) -> Result<HttpResponse, Error> {
    
    // 1) channel for progress notifications
    let (notification_tx, notif_rx) = mpsc::channel::<ProgressNotificationParam>(32);

    // 2) start SSE transport + MCP client with our ProgressHandler
    let transport = StreamableHttpClientTransport::from_uri("http://localhost:8004/mcp");


    let handler = ProgressHandler { notification_tx: notification_tx };
    let client: RunningService<_, _> = handler
        .serve(transport)
        .await
        .map_err(|e| {
            eprintln!("Failed to connect MCP client: {}", e);
            actix_web::error::ErrorInternalServerError("MCP connect error")
        })?;

    let queue = queue.get_ref().clone();
    let authenticated_as = None;
    // 3) hand off to our ChatSession actor
    ws::start(
        ChatSession {
            mcp_client: client,
            notification_reciever: Arc::new(Mutex::new(notif_rx)),
            queue,
            authenticated_as,
        },
        &req,
        stream,
    )
}


#[derive(Message)]
#[rtype(result = "()")]
struct Authenticated(Profile);

#[derive(Debug)]
pub struct ChatSession {
    mcp_client: RunningService<rmcp::RoleClient, ProgressHandler>,
    notification_reciever: Arc<Mutex<mpsc::Receiver<ProgressNotificationParam>>>,
    queue: Arc<Mutex<QueueManager>>,
    authenticated_as: Option<Profile>,
}

impl Handler<Authenticated> for ChatSession {
    type Result = ();

    fn handle(&mut self, msg: Authenticated, _: &mut Self::Context) {
        self.authenticated_as = Some(msg.0);
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

            let profile = match Profile::try_from_employee_string(resp) {
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
}

