use std::sync::Arc;
use actix_web::{Error, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use rmcp::{model::ProgressNotificationParam, service::RunningService, transport::StreamableHttpClientTransport, ServiceExt};
use tokio::sync::{mpsc, Mutex};

use crate::session::{ChatSession, ProgressHandler};
mod session;
mod queue;
mod ldap;
mod profile;
mod messages;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let _  = dotenv::dotenv();
    let queue_addr = Arc::new(Mutex::new(queue::QueueManager::new()));

    println!("Starting Urska proxy on http://127.0.0.1:8080/ws");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(queue_addr.clone()))
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
