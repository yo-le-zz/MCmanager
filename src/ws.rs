use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::state::AppState;

pub async fn console_ws(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(state, id, socket))
}

async fn handle_socket(state: AppState, id: Uuid, mut socket: WebSocket) {
    let (backlog, mut rx) = {
        let runtime = state.runtime.read().await;
        match runtime.get(&id) {
            Some(rt) => (rt.backlog.clone(), rt.tx.subscribe()),
            None => (Vec::new(), state.playit_tx.subscribe()), // dummy receiver if server never started
        }
    };

    for line in backlog {
        if socket.send(Message::Text(line)).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            line = rx.recv() => {
                match line {
                    Ok(line) => {
                        if socket.send(Message::Text(line)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let _ = crate::process::send_command(&state, id, &text).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

pub async fn playit_ws(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_playit_socket(state, socket))
}

async fn handle_playit_socket(state: AppState, mut socket: WebSocket) {
    let mut rx = state.playit_tx.subscribe();
    while let Ok(line) = rx.recv().await {
        if socket.send(Message::Text(line)).await.is_err() {
            break;
        }
    }
}
