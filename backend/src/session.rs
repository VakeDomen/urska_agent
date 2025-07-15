use std::sync::Arc;

use actix::prelude::*;
use actix_web::{Error, HttpRequest, HttpResponse, web};
use actix_web_actors::ws;
use futures::StreamExt;
use rmcp::{
    model::{CallToolRequestParam, ProgressNotificationParam},
    service::RunningService,
    transport::SseClientTransport,
    ClientHandler,                     // trait
    ServiceExt,                        // for .serve()
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, Mutex};

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
    End,
}

// -- our MCP progress‐notification handler --

#[derive(Debug)]
struct ProgressHandler {
    tx: mpsc::Sender<ProgressNotificationParam>,
}

impl ClientHandler for ProgressHandler {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _ctx: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        // simply forward the raw param into our channel
        let _ = self.tx.send(params).await;
    }
}

// -- the Actix‐Web entry point for WebSocket upgrade --

pub async fn ws_index(
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, Error> {
    // 1) channel for progress notifications
    let (notif_tx, notif_rx) = mpsc::channel::<ProgressNotificationParam>(32);

    // 2) start SSE transport + MCP client with our ProgressHandler
    let transport = SseClientTransport::start("http://localhost:8004/sse")
        .await
        .map_err(|e| {
            eprintln!("Failed to start SSE transport: {}", e);
            actix_web::error::ErrorInternalServerError("SSE transport error")
        })?;

    let handler = ProgressHandler { tx: notif_tx };
    let client: RunningService<_, _> = handler
        .serve(transport)
        .await
        .map_err(|e| {
            eprintln!("Failed to connect MCP client: {}", e);
            actix_web::error::ErrorInternalServerError("MCP connect error")
        })?;

    // 3) hand off to our ChatSession actor
    ws::start(
        ChatSession {
            mcp_client: client,
            notif_rx: Arc::new(Mutex::new(notif_rx)),
        },
        &req,
        stream,
    )
}

// -- the per‐WebSocket actor that ties everything together --

#[derive(Debug)]
struct ChatSession {
    mcp_client: RunningService<rmcp::RoleClient, ProgressHandler>,
    notif_rx: Arc<Mutex<mpsc::Receiver<ProgressNotificationParam>>>,
}

// at top of session.rs, add:
use actix::Message;

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
        println!("New actor: {:#?}", self);

        // 2) clone your receiver
        let notif_rx = self.notif_rx.clone();

        // 3) spawn a tokio task (or actix::spawn) that lives 'static
        actix::spawn(async move {
            while let Some(params) = notif_rx.lock().await.recv().await {
                // here you own `addr` and can use it
                let text = params
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
        println!("New message: {:#?}", item);
        match item {
            Ok(ws::Message::Text(txt)) => {
                // parse the prompt request
                if let Ok(req) = serde_json::from_str::<FrontendMessage>(&txt) {
                    let mut client = self.mcp_client.clone();
                    let addr = ctx.address();
                    // spawn an async task in the actor to call the tool
                    println!("drek");
                    let _a = ctx.spawn(
                        async move {
                            // build the tool call
                            println!("Calling tool!");

                            let mut args = Map::new();
                            args.insert("question".to_owned(), Value::String(req.question.clone()));

                            let call = CallToolRequestParam {
                                name: "ask_urska".into(),
                                arguments: Some(args),
                            };
                            // call the tool (fires progress events to our handler)
                            let result = client.call_tool(call).await;
                            println!("Tool result: {:#?}", result);
                            if let Err(e) = &result {
                                // on error, send it as a chunk
                                let err_msg = BackendMessage::Chunk(format!(
                                    "[Tool error] {}",
                                    e
                                ));
                                let _ = addr.do_send(SendWsText(serde_json::to_string(&err_msg).unwrap()));
                            }

                            // once done, send the End marker
                            let end = BackendMessage::End;
                            addr.do_send(SendWsText(serde_json::to_string(&end).unwrap()));
                        }
                        .into_actor(self),
                    );
                    
                }
            }
            Ok(ws::Message::Ping(p)) => ctx.pong(&p),
            Ok(ws::Message::Close(_)) => ctx.stop(),
            _ => {}
        }
    }
}
