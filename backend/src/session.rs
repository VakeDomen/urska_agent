use std::{clone, sync::Arc};

use actix::prelude::*;
use actix_web::{Error, HttpRequest, HttpResponse, web};
use actix_web_actors::ws;
use rmcp::{
    model::{CallToolRequestParam, ProgressNotificationParam},
    service::RunningService,
    transport::StreamableHttpClientTransport,
    ClientHandler,                     // trait
    ServiceExt,                        // for .serve()
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::{mpsc::{self, Receiver}, Mutex};

// -- messages exchanged with the front end --

#[derive(Deserialize)]
struct FrontendMessage {
    question: String,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
enum BackendMessage {
    Chunk(String),
    Notification(String),
    QueuePosition(PositionInQueue),
    End,
}

// -- our MCP progress‐notification handler --

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

// -- the Actix‐Web entry point for WebSocket upgrade --

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
    // 3) hand off to our ChatSession actor
    ws::start(
        ChatSession {
            mcp_client: client,
            notification_reciever: Arc::new(Mutex::new(notif_rx)),
            queue,
        },
        &req,
        stream,
    )
}

// -- the per‐WebSocket actor that ties everything together --

#[derive(Debug)]
struct ChatSession {
    mcp_client: RunningService<rmcp::RoleClient, ProgressHandler>,
    notification_reciever: Arc<Mutex<mpsc::Receiver<ProgressNotificationParam>>>,
    queue: Arc<Mutex<QueueManager>>
}

// at top of session.rs, add:
use actix::Message;

use crate::queue::{self, PositionInQueue, QueueManager, QueueMessage};

// define an internal actor‐message for sending WS text
struct SendWsText(pub String);
impl Message for SendWsText {
    type Result = ();
}

// in ChatSession, add the handler:
impl Handler<SendWsText> for ChatSession {
    type Result = ();

    fn handle(&mut self, msg: SendWsText, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

// then replace your `started` impl with:

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
                // here you own `addr` and can use it
                let text = notification_params
                    .message
                    .clone()
                    .unwrap_or_else(|| "▱".to_string());
                let json = serde_json::to_string(&BackendMessage::Notification(text))
                    .unwrap();

                // 4) send it back into the actor
                addr.do_send(SendWsText(json));
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
        // println!("New message: {:#?}", item);
        match item {
            Ok(ws::Message::Text(txt)) => {
                // parse the prompt request
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

        println!("Message");
        let client = self.mcp_client.clone();
        let addr = ctx.address();
        let queue = self.queue.clone();
        
        
        
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
                            let content = serde_json::to_string(&end).unwrap();
                            let message = SendWsText(content);
                            if let Err(e) = addr.try_send(message) {
                                println!("Something went wrong: {:#?}", e)
                            } 
                            continue;
                        },
                    };
                    
                }
            }

            let mut urska_argument_map = Map::new();

            urska_argument_map.insert(
                "question".to_string(), 
                Value::String(message.question.clone())
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
                let content = serde_json::to_string(&err_msg).unwrap();
                let message = SendWsText(content);
                let _ = addr.do_send(message);
            }

            // once done, send the End marker
            let end = BackendMessage::End;
            let content = serde_json::to_string(&end).unwrap();
            let message = SendWsText(content);
            addr.do_send(message);
            
            queue
                .clone()
                .lock()
                .await
                .notify_done(job_id.unwrap())
                .await;

        });
    }
}
