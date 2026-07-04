use std::collections::HashMap;
use std::sync::Arc;
use actix_web::{Error, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use rmcp::{model::ProgressNotificationParam, service::RunningService, transport::StreamableHttpClientTransport, ServiceExt};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::profile::Profile;
use crate::session::{ChatSession, ProgressHandler};
mod session;
mod queue;
mod ldap;
mod profile;
mod messages;

type SessionStore = Arc<Mutex<HashMap<String, Profile>>>;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let _  = dotenv::dotenv();
    let queue_addr = Arc::new(Mutex::new(queue::QueueManager::new()));
    let sessions: SessionStore = Arc::new(Mutex::new(HashMap::new()));

    println!("Starting Urska proxy on http://127.0.0.1:8080/ws");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(queue_addr.clone()))
            .app_data(web::Data::new(sessions.clone()))
            .route("/ws", web::get().to(ws_index))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}


pub async fn ws_index(
    req: HttpRequest,
    stream: web::Payload,
    queue: web::Data<Arc<Mutex<queue::QueueManager>>>,
    sessions: web::Data<SessionStore>,
) -> Result<HttpResponse, Error> {
    
    // 1) channel for progress notifications
    let (notification_tx, notif_rx) = mpsc::channel::<ProgressNotificationParam>(32);

    // 2) start SSE transport + MCP client with our ProgressHandler
    let transport = StreamableHttpClientTransport::from_uri("http://localhost:8004/mcp");


    let handler = ProgressHandler { 
        notification_tx 
    };
    let client: RunningService<_, _> = handler
        .serve(transport)
        .await
        .map_err(|e| {
            eprintln!("Failed to connect MCP client: {}", e);
            actix_web::error::ErrorInternalServerError("MCP connect error")
        })?;

    let queue = queue.get_ref().clone();
    let sessions = sessions.get_ref().clone();
    let authenticated_as = None;
    // 3) hand off to our ChatSession actor
    ws::start(
        ChatSession {
            id: Uuid::new_v4().to_string(),
            mcp_client: client,
            notification_reciever: Arc::new(Mutex::new(notif_rx)),
            queue,
            sessions,
            authenticated_as,
        },
        &req,
        stream,
    )
}
