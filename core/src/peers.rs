use std::{collections::HashMap, sync::{Arc, LazyLock}};

use rmcp::{Peer, RoleServer};
use tokio::sync::RwLock;

pub static CLIENT_PEERS: LazyLock<RwLock<HashMap<String, Peer<RoleServer>>>> = LazyLock::new(|| RwLock::new(HashMap::new()));