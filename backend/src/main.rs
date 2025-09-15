use std::sync::Arc;

use actix::Actor;
use actix_web::{web, App, HttpServer};
use tokio::sync::Mutex;
mod session;
mod queue;

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    let queue_addr = Arc::new(Mutex::new(queue::QueueManager::new()));

    println!("Starting Urska proxy on http://127.0.0.1:8080/ws");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(queue_addr.clone()))
            .route("/ws", web::get().to(session::ws_index))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
