# qdrant:    ./qdrant.sh

memory:    bash wait-port.sh 127.0.0.1 6333 && cd mcp-memory  && cargo run --release
scraper:   cd mcp-scraper && cargo run --release

staff:     bash wait-port.sh 127.0.0.1 8002 && bash wait-port.sh 127.0.0.1 8000 && cd mcp-staff         && cargo run --release
programme: bash wait-port.sh 127.0.0.1 8002 && bash wait-port.sh 127.0.0.1 8000 && cd mcp-programme     && cargo run --release
rag:       bash wait-port.sh 127.0.0.1 6333 && cd mcp-rag && cargo run --release

urska:     bash wait-port.sh 127.0.0.1 8001 && bash wait-port.sh 127.0.0.1 8003 && bash wait-port.sh 127.0.0.1 8005 && cd core                    && cargo run --release

#inspector: nvm use 22 && npx @modelcontextprotocol/inspector <-does not work 