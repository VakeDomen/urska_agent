use actix_web::{web, App, HttpServer};
mod session;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("Starting Urska proxy on http://127.0.0.1:8080/ws");
    HttpServer::new(|| {
        App::new()
            .route("/ws", web::get().to(session::ws_index))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
